//! Filesystem-backed clip index and actions for the desktop library.
//!
//! The catalog is an index only. Completed media files in the configured
//! output directory remain the source of truth; scans add files that are not
//! in the catalog and retain catalog rows for files that have disappeared so
//! the UI can report them as missing. All filesystem work can be sent to the
//! bounded [`LibraryWorker`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::UNIX_EPOCH;
use thiserror::Error;

pub const CATALOG_VERSION: u32 = 2;
pub const LIBRARY_QUEUE_BOUND: usize = 8;

const MAX_HEADER_BYTES: usize = 1024 * 1024;

/// A completed clip or a previously indexed clip whose file is now absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipEntry {
    pub path: PathBuf,
    pub name: String,
    #[serde(default)]
    pub game_name: Option<String>,
    pub created_at_unix_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub size_bytes: u64,
    pub missing: bool,
    #[serde(default)]
    pub protected: bool,
}

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("clip library worker is stopped")]
    WorkerStopped,

    #[error("clip library queue is full ({bound} pending); try again shortly")]
    QueueFull { bound: usize },

    #[error("clip library worker could not start: {details}")]
    WorkerStart { details: String },

    #[error("clip library I/O failed during {operation} for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("clip catalog is invalid at {path}: {message}")]
    Catalog { path: PathBuf, message: String },

    #[error("invalid clip path {path}: {reason}")]
    InvalidPath { path: PathBuf, reason: String },

    #[error("invalid clip name '{name}': {reason}")]
    InvalidName { name: String, reason: String },

    #[error("clip is not indexed: {path}")]
    NotIndexed { path: PathBuf },

    #[error("clip file is missing: {path}")]
    Missing { path: PathBuf },

    #[error("a clip already exists at {path}")]
    AlreadyExists { path: PathBuf },
}

impl LibraryError {
    /// Stable category for the desktop error boundary.
    pub fn code(&self) -> &'static str {
        match self {
            Self::WorkerStopped => "LIBRARY_WORKER_STOPPED",
            Self::QueueFull { .. } => "LIBRARY_QUEUE_FULL",
            Self::WorkerStart { .. } => "LIBRARY_WORKER_START_FAILED",
            Self::Io { operation, .. } => match *operation {
                "scan" => "LIBRARY_SCAN_FAILED",
                "rename" => "LIBRARY_RENAME_FAILED",
                "delete" => "LIBRARY_DELETE_FAILED",
                "catalog" => "LIBRARY_CATALOG_FAILED",
                _ => "LIBRARY_IO_FAILED",
            },
            Self::Catalog { .. } => "LIBRARY_CATALOG_INVALID",
            Self::InvalidPath { .. } => "LIBRARY_INVALID_PATH",
            Self::InvalidName { .. } => "LIBRARY_INVALID_NAME",
            Self::NotIndexed { .. } => "LIBRARY_CLIP_NOT_INDEXED",
            Self::Missing { .. } => "LIBRARY_CLIP_MISSING",
            Self::AlreadyExists { .. } => "LIBRARY_NAME_EXISTS",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogDocument {
    version: u32,
    entries: Vec<ClipEntry>,
}

/// Storage behavior owned by the library worker. A missing quota means that
/// quota accounting is disabled; automatic deletion is intentionally opt-in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoragePolicy {
    pub quota_bytes: Option<u64>,
    pub automatic_deletion_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSummary {
    pub output_directory: String,
    pub total_bytes: u64,
    pub quota_bytes: Option<u64>,
    pub protected_bytes: u64,
    pub eligible_bytes: u64,
    pub automatic_deletion_enabled: bool,
    pub deleted_count: usize,
    pub deleted_bytes: u64,
}

/// Synchronous, worker-owned library state. It does not spawn threads or
/// perform any UI work itself.
pub struct ClipLibrary {
    output_dir: PathBuf,
    catalog_path: PathBuf,
    entries: Vec<ClipEntry>,
    storage_policy: StoragePolicy,
}

impl ClipLibrary {
    /// Open an existing catalog without touching the output directory.
    pub fn open(output_dir: PathBuf, catalog_path: PathBuf) -> Result<Self, LibraryError> {
        cleanup_temporary_files(&output_dir)
            .map_err(|source| io_error("scan", &output_dir, source))?;
        // The catalog is disposable index state. A corrupt or obsolete index
        // must not hide files that can be recovered from the filesystem.
        let mut entries = match load_catalog(&catalog_path) {
            Ok(entries) => entries,
            Err(LibraryError::Catalog { .. }) => Vec::new(),
            Err(error) => return Err(error),
        }
        .into_iter()
        .filter(|entry| is_valid_clip_path(&output_dir, &entry.path))
        .collect::<Vec<_>>();
        for entry in &mut entries {
            entry.game_name = game_name_for_path(&output_dir, &entry.path);
        }
        Ok(Self {
            output_dir,
            catalog_path,
            entries,
            storage_policy: StoragePolicy::default(),
        })
    }

    /// Replace the directory used by future scans and actions.
    pub fn set_output_directory(&mut self, output_dir: PathBuf) {
        self.output_dir = output_dir;
        self.entries
            .retain(|entry| is_valid_clip_path(&self.output_dir, &entry.path));
        for entry in &mut self.entries {
            entry.game_name = game_name_for_path(&self.output_dir, &entry.path);
        }
    }

    pub fn set_storage_policy(&mut self, policy: StoragePolicy) -> StorageSummary {
        self.storage_policy = policy;
        self.make_storage_summary(0, 0)
    }

    /// Reconcile the catalog with completed files currently in the output
    /// directory. Missing catalog entries are retained and marked missing.
    pub fn scan(&mut self) -> Result<Vec<ClipEntry>, LibraryError> {
        let mut seen = BTreeSet::new();
        for entry in &mut self.entries {
            let Some(parent) = clip_parent(&self.output_dir, &entry.path) else {
                entry.missing = true;
                continue;
            };
            match is_safe_clip_parent(&self.output_dir, &parent) {
                Ok(true) => {}
                Ok(false) => {
                    entry.missing = true;
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    entry.missing = true;
                    continue;
                }
                Err(source) => {
                    return Err(io_error("scan", &parent, source));
                }
            }
            match fs::symlink_metadata(&entry.path) {
                Ok(metadata) if is_regular_file(&metadata) => {
                    entry.game_name = game_name_for_path(&self.output_dir, &entry.path);
                    refresh_entry(entry, &metadata);
                    seen.insert(normalize_path(&entry.path));
                }
                Ok(_) => entry.missing = true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => entry.missing = true,
                Err(source) => {
                    return Err(io_error("scan", &entry.path, source));
                }
            }
        }

        match fs::read_dir(&self.output_dir) {
            Ok(directory) => {
                for item in directory {
                    let item = item.map_err(|source| io_error("scan", &self.output_dir, source))?;
                    let path = item.path();
                    let metadata = match fs::symlink_metadata(&path) {
                        Ok(metadata) => metadata,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                        Err(source) => return Err(io_error("scan", &path, source)),
                    };
                    if is_regular_file(&metadata) {
                        if is_supported_clip(&path) {
                            self.add_discovered_file(path, &metadata, None, &mut seen);
                        }
                        continue;
                    }
                    if !is_regular_directory(&metadata) {
                        continue;
                    }

                    let game_name = Some(file_name(&path));
                    let game_directory = match fs::read_dir(&path) {
                        Ok(directory) => directory,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                        Err(source) => return Err(io_error("scan", &path, source)),
                    };
                    for child in game_directory {
                        let child = child.map_err(|source| io_error("scan", &path, source))?;
                        let child_path = child.path();
                        let child_metadata = match fs::symlink_metadata(&child_path) {
                            Ok(metadata) => metadata,
                            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                            Err(source) => {
                                return Err(io_error("scan", &child_path, source));
                            }
                        };
                        if is_regular_file(&child_metadata) && is_supported_clip(&child_path) {
                            self.add_discovered_file(
                                child_path,
                                &child_metadata,
                                game_name.clone(),
                                &mut seen,
                            );
                        }
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                for entry in &mut self.entries {
                    entry.missing = true;
                }
            }
            Err(source) => return Err(io_error("scan", &self.output_dir, source)),
        }

        sort_entries(&mut self.entries);
        save_catalog(&self.catalog_path, &self.entries)?;
        self.enforce_storage_policy()?;
        Ok(self.entries.clone())
    }

    fn add_discovered_file(
        &mut self,
        path: PathBuf,
        metadata: &fs::Metadata,
        game_name: Option<String>,
        seen: &mut BTreeSet<PathBuf>,
    ) {
        if !seen.insert(normalize_path(&path)) {
            return;
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| same_path(&entry.path, &path))
        {
            entry.game_name = game_name;
            refresh_entry(entry, metadata);
        } else {
            self.entries
                .push(entry_from_file(path, metadata, game_name));
        }
    }

    pub fn entries(&self) -> Vec<ClipEntry> {
        self.entries.clone()
    }

    pub fn storage_summary(&self) -> StorageSummary {
        self.make_storage_summary(0, 0)
    }

    fn make_storage_summary(&self, deleted_count: usize, deleted_bytes: u64) -> StorageSummary {
        let mut total_bytes = 0_u64;
        let mut protected_bytes = 0_u64;
        let mut eligible_bytes = 0_u64;
        for entry in &self.entries {
            if entry.missing {
                continue;
            }
            total_bytes = total_bytes.saturating_add(entry.size_bytes);
            if entry.protected {
                protected_bytes = protected_bytes.saturating_add(entry.size_bytes);
            } else {
                eligible_bytes = eligible_bytes.saturating_add(entry.size_bytes);
            }
        }
        StorageSummary {
            output_directory: self.output_dir.display().to_string(),
            total_bytes,
            quota_bytes: self.storage_policy.quota_bytes,
            protected_bytes,
            eligible_bytes,
            automatic_deletion_enabled: self.storage_policy.automatic_deletion_enabled,
            deleted_count,
            deleted_bytes,
        }
    }

    fn enforce_storage_policy(&mut self) -> Result<StorageSummary, LibraryError> {
        let Some(quota_bytes) = self.storage_policy.quota_bytes else {
            return Ok(self.make_storage_summary(0, 0));
        };
        if !self.storage_policy.automatic_deletion_enabled {
            return Ok(self.make_storage_summary(0, 0));
        }

        let mut total_bytes = self
            .entries
            .iter()
            .filter(|entry| !entry.missing)
            .map(|entry| entry.size_bytes)
            .sum::<u64>();
        let mut candidates: Vec<(u64, String, PathBuf)> = self
            .entries
            .iter()
            .filter(|entry| !entry.missing && !entry.protected)
            .map(|entry| {
                (
                    entry.created_at_unix_ms.unwrap_or(0),
                    entry.name.to_ascii_lowercase(),
                    entry.path.clone(),
                )
            })
            .collect();
        candidates.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| normalize_path(&left.2).cmp(&normalize_path(&right.2)))
        });

        let mut deleted_count = 0_usize;
        let mut deleted_bytes = 0_u64;
        for (_, _, path) in candidates {
            if total_bytes <= quota_bytes {
                break;
            }
            let Some(entry) = self
                .entries
                .iter()
                .find(|entry| same_path(&entry.path, &path))
            else {
                continue;
            };
            let size = entry.size_bytes;
            self.delete(&path)?;
            total_bytes = total_bytes.saturating_sub(size);
            deleted_count += 1;
            deleted_bytes = deleted_bytes.saturating_add(size);
        }
        Ok(self.make_storage_summary(deleted_count, deleted_bytes))
    }

    /// Rename a clip on the same output volume and update the catalog only
    /// after the filesystem operation succeeds.
    pub fn rename(&mut self, path: &Path, requested_name: &str) -> Result<ClipEntry, LibraryError> {
        let index = self.find_entry_index(path)?;
        let old_path = self.checked_existing_path(index)?;
        let new_path = renamed_path(&self.output_dir, &old_path, requested_name)?;
        if same_path(&old_path, &new_path) {
            return Ok(self.entries[index].clone());
        }
        if fs::symlink_metadata(&new_path).is_ok()
            || self
                .entries
                .iter()
                .any(|entry| same_path(&entry.path, &new_path))
        {
            return Err(LibraryError::AlreadyExists { path: new_path });
        }

        fs::rename(&old_path, &new_path).map_err(|source| io_error("rename", &old_path, source))?;
        let previous = self.entries[index].clone();
        self.entries[index].path = new_path.clone();
        self.entries[index].name = file_name(&new_path);
        self.entries[index].missing = false;
        if let Err(error) = save_catalog(&self.catalog_path, &self.entries) {
            let rollback = fs::rename(&new_path, &old_path);
            self.entries[index] = previous;
            if let Err(rollback_error) = rollback {
                return Err(LibraryError::Catalog {
                    path: self.catalog_path.clone(),
                    message: format!("{error}; filesystem rollback failed: {rollback_error}"),
                });
            }
            return Err(error);
        }
        Ok(self.entries[index].clone())
    }

    /// Delete a file and remove its index row. Missing rows can be removed
    /// without touching the filesystem.
    pub fn delete(&mut self, path: &Path) -> Result<(), LibraryError> {
        let index = self.find_entry_index(path)?;
        if self.entries[index].missing {
            self.entries.remove(index);
            return save_catalog(&self.catalog_path, &self.entries);
        }

        let actual_path = match self.checked_existing_path(index) {
            Ok(path) => path,
            Err(LibraryError::Missing { .. }) => {
                self.entries.remove(index);
                return save_catalog(&self.catalog_path, &self.entries);
            }
            Err(error) => return Err(error),
        };
        let staged_path = delete_staged_path(&actual_path);
        fs::rename(&actual_path, &staged_path)
            .map_err(|source| io_error("delete", &actual_path, source))?;
        let previous = self.entries.remove(index);
        if let Err(error) = save_catalog(&self.catalog_path, &self.entries) {
            let rollback = fs::rename(&staged_path, &actual_path);
            self.entries.insert(index, previous);
            if let Err(rollback_error) = rollback {
                return Err(LibraryError::Catalog {
                    path: self.catalog_path.clone(),
                    message: format!("{error}; filesystem rollback failed: {rollback_error}"),
                });
            }
            return Err(error);
        }
        if let Err(source) = fs::remove_file(&staged_path) {
            return Err(io_error("delete", &staged_path, source));
        }
        Ok(())
    }

    pub fn set_protected(
        &mut self,
        path: &Path,
        protected: bool,
    ) -> Result<ClipEntry, LibraryError> {
        let index = self.find_entry_index(path)?;
        if !self.entries[index].missing {
            self.checked_existing_path(index)?;
        }
        self.entries[index].protected = protected;
        save_catalog(&self.catalog_path, &self.entries)?;
        Ok(self.entries[index].clone())
    }

    /// Return an indexed, existing file after checking its real parent. The
    /// desktop shell passes only this result to the official opener plugin.
    pub fn checked_action_path(&self, path: &Path) -> Result<PathBuf, LibraryError> {
        let index = self.find_entry_index(path)?;
        self.checked_existing_path(index)
    }

    fn find_entry_index(&self, path: &Path) -> Result<usize, LibraryError> {
        if !is_valid_clip_path(&self.output_dir, path) {
            return Err(LibraryError::InvalidPath {
                path: path.to_path_buf(),
                reason: "clip must be directly under the output directory or one game directory"
                    .to_string(),
            });
        }
        self.entries
            .iter()
            .position(|entry| same_path(&entry.path, path))
            .ok_or_else(|| LibraryError::NotIndexed {
                path: path.to_path_buf(),
            })
    }

    fn checked_existing_path(&self, index: usize) -> Result<PathBuf, LibraryError> {
        let actual_path = self.entries[index].path.clone();
        let Some(parent) = clip_parent(&self.output_dir, &actual_path) else {
            return Err(LibraryError::InvalidPath {
                path: actual_path,
                reason: "clip must be directly under the output directory or one game directory"
                    .to_string(),
            });
        };
        if self.entries[index].missing {
            return Err(LibraryError::Missing { path: actual_path });
        }
        match is_safe_clip_parent(&self.output_dir, &parent) {
            Ok(true) => {}
            Ok(false) => {
                return Err(LibraryError::InvalidPath {
                    path: actual_path,
                    reason: "clip game directory must be a regular directory, not a link or reparse point"
                        .to_string(),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(LibraryError::Missing { path: actual_path });
            }
            Err(source) => return Err(io_error("scan", &parent, source)),
        }
        let metadata = match fs::symlink_metadata(&actual_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(LibraryError::Missing { path: actual_path });
            }
            Err(source) => return Err(io_error("scan", &actual_path, source)),
        };
        if !is_regular_file(&metadata) {
            return Err(LibraryError::InvalidPath {
                path: actual_path,
                reason: "clip must be a regular file, not a link or directory".to_string(),
            });
        }
        let root = fs::canonicalize(&self.output_dir)
            .map_err(|source| io_error("scan", &self.output_dir, source))?;
        let canonical = match fs::canonicalize(&actual_path) {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(LibraryError::Missing { path: actual_path });
            }
            Err(source) => return Err(io_error("scan", &actual_path, source)),
        };
        if canonical.parent().is_none_or(|parent| {
            !same_path(parent, &root)
                && !parent
                    .parent()
                    .is_some_and(|grandparent| same_path(grandparent, &root))
        }) {
            return Err(LibraryError::InvalidPath {
                path: actual_path,
                reason: "clip resolves outside the configured output directory".to_string(),
            });
        }
        Ok(actual_path)
    }
}

enum Request {
    Scan {
        reply: SyncSender<Result<Vec<ClipEntry>, LibraryError>>,
    },
    SetOutputDirectory {
        output_dir: PathBuf,
        reply: SyncSender<Result<(), LibraryError>>,
    },
    Rename {
        path: PathBuf,
        name: String,
        reply: SyncSender<Result<ClipEntry, LibraryError>>,
    },
    Delete {
        path: PathBuf,
        reply: SyncSender<Result<(), LibraryError>>,
    },
    SetProtected {
        path: PathBuf,
        protected: bool,
        reply: SyncSender<Result<ClipEntry, LibraryError>>,
    },
    SetStoragePolicy {
        policy: StoragePolicy,
        reply: SyncSender<StorageSummary>,
    },
    GetStorageSummary {
        reply: SyncSender<StorageSummary>,
    },
    CheckPath {
        path: PathBuf,
        reply: SyncSender<Result<PathBuf, LibraryError>>,
    },
}

/// Bounded command bridge to a dedicated filesystem worker.
pub struct LibraryWorker {
    requests: Option<SyncSender<Request>>,
    worker: Option<JoinHandle<()>>,
}

impl LibraryWorker {
    pub fn new(output_dir: PathBuf, catalog_path: PathBuf) -> Result<Self, LibraryError> {
        Self::new_with_policy(output_dir, catalog_path, StoragePolicy::default())
    }

    pub fn new_with_policy(
        output_dir: PathBuf,
        catalog_path: PathBuf,
        policy: StoragePolicy,
    ) -> Result<Self, LibraryError> {
        let (requests, receiver) = mpsc::sync_channel(LIBRARY_QUEUE_BOUND);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("silk-library-worker".to_string())
            .spawn(move || run_worker(output_dir, catalog_path, policy, receiver, ready_tx))
            .map_err(|error| LibraryError::WorkerStart {
                details: error.to_string(),
            })?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                requests: Some(requests),
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err(LibraryError::WorkerStopped)
            }
        }
    }

    pub fn scan(&self) -> Result<Vec<ClipEntry>, LibraryError> {
        let (reply, result) = response_channel();
        self.send(Request::Scan { reply })?;
        receive(result)
    }

    pub fn set_output_directory(&self, output_dir: PathBuf) -> Result<(), LibraryError> {
        let (reply, result) = response_channel();
        self.send(Request::SetOutputDirectory { output_dir, reply })?;
        receive(result)
    }

    pub fn rename(&self, path: PathBuf, name: String) -> Result<ClipEntry, LibraryError> {
        let (reply, result) = response_channel();
        self.send(Request::Rename { path, name, reply })?;
        receive(result)
    }

    pub fn delete(&self, path: PathBuf) -> Result<(), LibraryError> {
        let (reply, result) = response_channel();
        self.send(Request::Delete { path, reply })?;
        receive(result)
    }

    pub fn set_protected(&self, path: PathBuf, protected: bool) -> Result<ClipEntry, LibraryError> {
        let (reply, result) = response_channel();
        self.send(Request::SetProtected {
            path,
            protected,
            reply,
        })?;
        receive(result)
    }

    pub fn set_storage_policy(
        &self,
        policy: StoragePolicy,
    ) -> Result<StorageSummary, LibraryError> {
        let (reply, result) = mpsc::sync_channel(1);
        self.send(Request::SetStoragePolicy { policy, reply })?;
        result.recv().map_err(|_| LibraryError::WorkerStopped)
    }

    pub fn storage_summary(&self) -> Result<StorageSummary, LibraryError> {
        let (reply, result) = mpsc::sync_channel(1);
        self.send(Request::GetStorageSummary { reply })?;
        result.recv().map_err(|_| LibraryError::WorkerStopped)
    }

    pub fn checked_action_path(&self, path: PathBuf) -> Result<PathBuf, LibraryError> {
        let (reply, result) = response_channel();
        self.send(Request::CheckPath { path, reply })?;
        receive(result)
    }

    fn send(&self, request: Request) -> Result<(), LibraryError> {
        let Some(sender) = self.requests.as_ref() else {
            return Err(LibraryError::WorkerStopped);
        };
        sender.try_send(request).map_err(|error| match error {
            TrySendError::Full(_) => LibraryError::QueueFull {
                bound: LIBRARY_QUEUE_BOUND,
            },
            TrySendError::Disconnected(_) => LibraryError::WorkerStopped,
        })
    }
}

impl Drop for LibraryWorker {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

type Reply<T> = SyncSender<Result<T, LibraryError>>;
type Response<T> = Receiver<Result<T, LibraryError>>;

fn response_channel<T>() -> (Reply<T>, Response<T>) {
    mpsc::sync_channel(1)
}

fn receive<T>(receiver: Receiver<Result<T, LibraryError>>) -> Result<T, LibraryError> {
    receiver.recv().unwrap_or(Err(LibraryError::WorkerStopped))
}

fn run_worker(
    output_dir: PathBuf,
    catalog_path: PathBuf,
    policy: StoragePolicy,
    receiver: Receiver<Request>,
    ready: SyncSender<Result<(), LibraryError>>,
) {
    let mut library = match ClipLibrary::open(output_dir, catalog_path) {
        Ok(library) => library,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    library.storage_policy = policy;
    let _ = ready.send(Ok(()));
    while let Ok(request) = receiver.recv() {
        match request {
            Request::Scan { reply } => {
                let _ = reply.send(library.scan());
            }
            Request::SetOutputDirectory { output_dir, reply } => {
                library.set_output_directory(output_dir);
                let _ = reply.send(Ok(()));
            }
            Request::Rename { path, name, reply } => {
                let _ = reply.send(library.rename(&path, &name));
            }
            Request::Delete { path, reply } => {
                let _ = reply.send(library.delete(&path));
            }
            Request::SetProtected {
                path,
                protected,
                reply,
            } => {
                let _ = reply.send(library.set_protected(&path, protected));
            }
            Request::SetStoragePolicy { policy, reply } => {
                let _ = reply.send(library.set_storage_policy(policy));
            }
            Request::GetStorageSummary { reply } => {
                let _ = reply.send(library.storage_summary());
            }
            Request::CheckPath { path, reply } => {
                let _ = reply.send(library.checked_action_path(&path));
            }
        }
    }
}

fn load_catalog(path: &Path) -> Result<Vec<ClipEntry>, LibraryError> {
    let rendered = match fs::read_to_string(path) {
        Ok(rendered) => rendered,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_error("catalog", path, source)),
    };
    let document: CatalogDocument =
        serde_json::from_str(&rendered).map_err(|error| LibraryError::Catalog {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    match document.version {
        1 | CATALOG_VERSION => {}
        version => {
            return Err(LibraryError::Catalog {
                path: path.to_path_buf(),
                message: format!(
                    "unsupported catalog version {} (supported {})",
                    version, CATALOG_VERSION
                ),
            });
        }
    }
    Ok(document.entries)
}

fn save_catalog(path: &Path, entries: &[ClipEntry]) -> Result<(), LibraryError> {
    let document = CatalogDocument {
        version: CATALOG_VERSION,
        entries: entries.to_vec(),
    };
    let rendered =
        serde_json::to_string_pretty(&document).map_err(|error| LibraryError::Catalog {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| io_error("catalog", parent, source))?;
    }
    let temp = sibling_temp_path(path);
    if let Err(source) = fs::write(&temp, rendered.as_bytes()) {
        return Err(io_error("catalog", &temp, source));
    }
    if let Err(source) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(io_error("catalog", path, source));
    }
    Ok(())
}

fn sibling_temp_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "catalog".into());
    name.push(".tmp");
    path.parent().unwrap_or_else(|| Path::new(".")).join(name)
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> LibraryError {
    LibraryError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn entry_from_file(path: PathBuf, metadata: &fs::Metadata, game_name: Option<String>) -> ClipEntry {
    ClipEntry {
        path: path.clone(),
        name: file_name(&path),
        game_name,
        created_at_unix_ms: created_at(metadata),
        duration_ms: duration_from_file(&path),
        size_bytes: metadata.len(),
        missing: false,
        protected: false,
    }
}

fn refresh_entry(entry: &mut ClipEntry, metadata: &fs::Metadata) {
    entry.name = file_name(&entry.path);
    // Keep the indexed age stable across scans; filesystem timestamps may only
    // have millisecond precision and can collapse distinct clips together.
    entry.created_at_unix_ms = entry.created_at_unix_ms.or_else(|| created_at(metadata));
    entry.duration_ms = duration_from_file(&entry.path).or(entry.duration_ms);
    entry.size_bytes = metadata.len();
    entry.missing = false;
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn cleanup_temporary_files(output_dir: &Path) -> io::Result<()> {
    let directory = match fs::read_dir(output_dir) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for item in directory {
        let item = item?;
        let path = item.path();
        let metadata = fs::symlink_metadata(&path)?;
        if is_regular_file(&metadata) && is_temporary_file(&path) {
            fs::remove_file(path)?;
        } else if is_regular_directory(&metadata) {
            let game_directory = fs::read_dir(item.path())?;
            for child in game_directory {
                let child = child?;
                let child_path = child.path();
                let child_metadata = fs::symlink_metadata(&child_path)?;
                if is_regular_file(&child_metadata) && is_temporary_file(&child_path) {
                    fs::remove_file(child_path)?;
                }
            }
        }
    }
    Ok(())
}

fn is_regular_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file() && !is_link_or_reparse(metadata)
}

fn is_regular_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && !is_link_or_reparse(metadata)
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || is_reparse_point(metadata)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn is_temporary_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "part")
        || path.file_name().is_some_and(|name| {
            let name = name.to_string_lossy();
            name.ends_with(".part.lock") || name.contains(".silk-delete")
        })
}

fn delete_staged_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clip".to_string());
    let mut candidate = parent.join(format!(".{name}.silk-delete"));
    let mut index = 1_u32;
    while fs::symlink_metadata(&candidate).is_ok() {
        candidate = parent.join(format!(".{name}.silk-delete-{index}"));
        index = index.saturating_add(1);
    }
    candidate
}

fn created_at(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .created()
        .or_else(|_| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
}

fn duration_from_file(path: &Path) -> Option<u64> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let mut pos = 0_u64;

    while pos < file_len {
        let remaining = file_len.checked_sub(pos)?;
        if remaining < 8 {
            return None;
        }
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;

        let raw_size = u32::from_be_bytes(header[0..4].try_into().ok()?);
        let box_type: [u8; 4] = header[4..8].try_into().ok()?;

        let box_size = match raw_size {
            0 => remaining,
            1 => {
                if remaining < 16 {
                    return None;
                }
                let mut ext_buf = [0u8; 8];
                file.read_exact(&mut ext_buf).ok()?;
                let large_size = u64::from_be_bytes(ext_buf);
                if large_size < 16 {
                    return None;
                }
                large_size
            }
            2..=7 => return None,
            _ => u64::from(raw_size),
        };

        if box_size > remaining {
            return None;
        }

        let next_pos = pos.checked_add(box_size)?;

        if box_type == *b"moov" {
            if box_size > MAX_HEADER_BYTES as u64 {
                return None;
            }
            let read_len = usize::try_from(box_size).ok()?;
            let mut moov_bytes = vec![0u8; read_len];
            file.seek(SeekFrom::Start(pos)).ok()?;
            file.read_exact(&mut moov_bytes).ok()?;
            return parse_mp4_duration(&moov_bytes);
        }

        pos = next_pos;
    }

    None
}

fn parse_mp4_duration(data: &[u8]) -> Option<u64> {
    let mut cursor = 0_usize;
    let limit = data.len();
    while cursor < limit {
        let header = read_box(data, cursor, limit)?;
        if header.box_type == *b"moov" {
            let mut moov_cursor = header.payload_start;
            let moov_limit = header.payload_end;
            while moov_cursor < moov_limit {
                let child = read_box(data, moov_cursor, moov_limit)?;
                if child.box_type == *b"mvhd" {
                    return parse_mvhd(&data[child.payload_start..child.payload_end]);
                }
                moov_cursor = child.next_offset;
            }
            return None;
        }
        cursor = header.next_offset;
    }
    None
}

struct BoxHeader {
    box_type: [u8; 4],
    payload_start: usize,
    payload_end: usize,
    next_offset: usize,
}

fn read_box(data: &[u8], offset: usize, limit: usize) -> Option<BoxHeader> {
    let header_min_end = offset.checked_add(8)?;
    if header_min_end > limit {
        return None;
    }
    let raw_size = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?);
    let box_type: [u8; 4] = data[offset + 4..offset + 8].try_into().ok()?;
    match raw_size {
        0 => Some(BoxHeader {
            box_type,
            payload_start: offset + 8,
            payload_end: limit,
            next_offset: limit,
        }),
        1 => {
            let header_ext_end = offset.checked_add(16)?;
            if header_ext_end > limit {
                return None;
            }
            let large_size = u64::from_be_bytes(data[offset + 8..offset + 16].try_into().ok()?);
            if large_size < 16 {
                return None;
            }
            let box_len = usize::try_from(large_size).ok()?;
            let box_end = offset.checked_add(box_len)?;
            if box_end > limit {
                return None;
            }
            Some(BoxHeader {
                box_type,
                payload_start: offset + 16,
                payload_end: box_end,
                next_offset: box_end,
            })
        }
        2..=7 => None,
        _ => {
            let box_len = usize::try_from(raw_size).ok()?;
            let box_end = offset.checked_add(box_len)?;
            if box_end > limit {
                return None;
            }
            Some(BoxHeader {
                box_type,
                payload_start: offset + 8,
                payload_end: box_end,
                next_offset: box_end,
            })
        }
    }
}

fn parse_mvhd(payload: &[u8]) -> Option<u64> {
    if payload.len() < 4 {
        return None;
    }
    let version = payload[0];
    let (timescale, duration) = match version {
        0 => {
            if payload.len() < 20 {
                return None;
            }
            let timescale = u32::from_be_bytes(payload[12..16].try_into().ok()?);
            let duration = u32::from_be_bytes(payload[16..20].try_into().ok()?);
            (u64::from(timescale), u64::from(duration))
        }
        1 => {
            if payload.len() < 32 {
                return None;
            }
            let timescale = u32::from_be_bytes(payload[20..24].try_into().ok()?);
            let duration = u64::from_be_bytes(payload[24..32].try_into().ok()?);
            (u64::from(timescale), duration)
        }
        _ => return None,
    };
    if timescale == 0 {
        return None;
    }
    let ms = (u128::from(duration).saturating_mul(1_000)) / u128::from(timescale);
    Some(u64::try_from(ms).unwrap_or(u64::MAX))
}

fn sort_entries(entries: &mut [ClipEntry]) {
    entries.sort_by(|left, right| {
        left.missing
            .cmp(&right.missing)
            .then_with(|| right.created_at_unix_ms.cmp(&left.created_at_unix_ms))
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then_with(|| normalize_path(&left.path).cmp(&normalize_path(&right.path)))
    });
}

fn is_supported_clip(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
}

fn is_valid_clip_path(root: &Path, path: &Path) -> bool {
    clip_parent(root, path).is_some()
}

fn clip_parent(root: &Path, path: &Path) -> Option<PathBuf> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return None;
    }
    let root = normalize_path(root);
    let path = normalize_path(path);
    let parent = path.parent()?;
    if same_path(parent, &root)
        || parent
            .parent()
            .is_some_and(|grandparent| same_path(grandparent, &root))
    {
        Some(parent.to_path_buf())
    } else {
        None
    }
}

fn game_name_for_path(root: &Path, path: &Path) -> Option<String> {
    let parent = clip_parent(root, path)?;
    (!same_path(&parent, &normalize_path(root))).then(|| file_name(&parent))
}

fn is_safe_clip_parent(root: &Path, parent: &Path) -> io::Result<bool> {
    if same_path(parent, &normalize_path(root)) {
        return Ok(true);
    }
    let metadata = fs::symlink_metadata(parent)?;
    Ok(is_regular_directory(&metadata))
}

fn normalize_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn renamed_path(
    root: &Path,
    old_path: &Path,
    requested_name: &str,
) -> Result<PathBuf, LibraryError> {
    let name = requested_name.trim();
    validate_name(name)?;
    let old_extension = old_path
        .extension()
        .and_then(|extension| extension.to_str());
    let requested = Path::new(name);
    let requested_extension = requested
        .extension()
        .and_then(|extension| extension.to_str());
    if let Some(extension) = requested_extension {
        if old_extension.is_none_or(|old| !old.eq_ignore_ascii_case(extension)) {
            return Err(LibraryError::InvalidName {
                name: name.to_string(),
                reason: "the existing clip extension must be preserved".to_string(),
            });
        }
    }
    let mut file_name = name.to_string();
    if requested_extension.is_none() {
        if let Some(extension) = old_extension {
            file_name.push('.');
            file_name.push_str(extension);
        }
    }
    let parent = old_path.parent().unwrap_or(root);
    let new_path = parent.join(file_name);
    if !is_valid_clip_path(root, &new_path) {
        return Err(LibraryError::InvalidPath {
            path: new_path,
            reason: "renamed clip must remain under the output directory or one game directory"
                .to_string(),
        });
    }
    Ok(new_path)
}

fn validate_name(name: &str) -> Result<(), LibraryError> {
    if name.is_empty() {
        return Err(LibraryError::InvalidName {
            name: name.to_string(),
            reason: "name must not be empty".to_string(),
        });
    }
    if name == "." || name == ".." || name.ends_with('.') || name.ends_with(' ') {
        return Err(LibraryError::InvalidName {
            name: name.to_string(),
            reason: "name must not be dot-like or end with a dot or space".to_string(),
        });
    }
    const INVALID: [char; 9] = ['<', '>', ':', '"', '|', '?', '*', '/', '\\'];
    if name
        .chars()
        .any(|character| character.is_control() || INVALID.contains(&character))
    {
        return Err(LibraryError::InvalidName {
            name: name.to_string(),
            reason: "name contains characters that are invalid on Windows".to_string(),
        });
    }
    let stem = name.split('.').next().unwrap_or_default();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return Err(LibraryError::InvalidName {
            name: name.to_string(),
            reason: "name is reserved by Windows".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "silk-library-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("test directory");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn catalog_path(dir: &TestDir) -> PathBuf {
        dir.path.join("library.json")
    }

    fn make_box(box_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len() + 8).expect("box size");
        let mut bytes = Vec::with_capacity(payload.len() + 8);
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(box_type);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn make_box_ext(box_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = 1_u32;
        let large_size = u64::try_from(payload.len() + 16).expect("box large size");
        let mut bytes = Vec::with_capacity(payload.len() + 16);
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(box_type);
        bytes.extend_from_slice(&large_size.to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn make_box_to_eof(box_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = 0_u32;
        let mut bytes = Vec::with_capacity(payload.len() + 8);
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(box_type);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn make_mvhd_v0(timescale: u32, duration: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        payload.extend_from_slice(&0_u32.to_be_bytes());
        payload.extend_from_slice(&0_u32.to_be_bytes());
        payload.extend_from_slice(&timescale.to_be_bytes());
        payload.extend_from_slice(&duration.to_be_bytes());
        payload.extend_from_slice(&[0u8; 80]);
        make_box(b"mvhd", &payload)
    }

    fn make_mvhd_v1(timescale: u32, duration: u64) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        payload.extend_from_slice(&0_u64.to_be_bytes());
        payload.extend_from_slice(&0_u64.to_be_bytes());
        payload.extend_from_slice(&timescale.to_be_bytes());
        payload.extend_from_slice(&duration.to_be_bytes());
        payload.extend_from_slice(&[0u8; 80]);
        make_box(b"mvhd", &payload)
    }

    fn mp4_with_duration_v0(timescale: u32, duration: u32) -> Vec<u8> {
        let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isommp41");
        let mvhd = make_mvhd_v0(timescale, duration);
        let moov = make_box(b"moov", &mvhd);
        [ftyp, moov].concat()
    }

    fn mp4_with_duration_v1(timescale: u32, duration: u64) -> Vec<u8> {
        let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isommp41");
        let mvhd = make_mvhd_v1(timescale, duration);
        let moov = make_box(b"moov", &mvhd);
        [ftyp, moov].concat()
    }

    #[test]
    fn scan_indexes_completed_files_and_retains_missing_rows() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        let first = output.join("first.mp4");
        let second = output.join("second.mp4");
        fs::write(&first, mp4_with_duration_v0(1_000, 1_250)).expect("first");
        fs::write(&second, b"mp4-placeholder").expect("second");
        fs::write(output.join("partial.mp4.part"), b"partial").expect("partial");

        let mut library = ClipLibrary::open(output.clone(), catalog_path(&dir)).expect("open");
        let entries = library.scan().expect("scan");
        assert_eq!(entries.len(), 2);
        let first_entry = entries
            .iter()
            .find(|entry| entry.path == first)
            .expect("first row");
        assert_eq!(first_entry.duration_ms, Some(1_250));
        assert_eq!(
            first_entry.size_bytes,
            fs::metadata(&first).expect("metadata").len()
        );
        assert!(entries.iter().all(|entry| !entry.missing));

        fs::remove_file(&first).expect("remove first");
        let third = output.join("third.mp4");
        fs::write(&third, b"not-a-valid-mp4-file").expect("third");
        let reconciled = library.scan().expect("reconcile");
        assert!(reconciled
            .iter()
            .any(|entry| entry.path == first && entry.missing));
        assert!(reconciled
            .iter()
            .any(|entry| entry.path == third && !entry.missing));
        assert!(!reconciled
            .iter()
            .any(|entry| entry.name.contains("partial")));
    }

    #[test]
    fn scan_indexes_legacy_and_one_level_game_clips() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        let game = output.join("Game One");
        fs::create_dir_all(&game).expect("game directory");
        let legacy = output.join("legacy.mp4");
        let game_clip = game.join("game.mp4");
        fs::write(&legacy, b"legacy").expect("legacy clip");
        fs::write(&game_clip, b"game").expect("game clip");

        let entries = ClipLibrary::open(output, catalog_path(&dir))
            .expect("open")
            .scan()
            .expect("scan");
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.path == legacy)
                .expect("legacy row")
                .game_name,
            None
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.path == game_clip)
                .expect("game row")
                .game_name
                .as_deref(),
            Some("Game One")
        );
    }

    #[test]
    fn deeper_nesting_is_ignored() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        let game = output.join("Game");
        let deeper = game.join("nested");
        fs::create_dir_all(&deeper).expect("nested directories");
        fs::write(game.join("direct.mp4"), b"direct").expect("direct clip");
        fs::write(deeper.join("ignored.mp4"), b"ignored").expect("deep clip");

        let entries = ClipLibrary::open(output, catalog_path(&dir))
            .expect("open")
            .scan()
            .expect("scan");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "direct.mp4");
        assert_eq!(entries[0].game_name.as_deref(), Some("Game"));
    }

    #[test]
    fn game_name_and_catalog_metadata_survive_scans_and_missing_rows() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        let game = output.join("Game");
        fs::create_dir_all(&game).expect("game directory");
        let path = game.join("clip.mp4");
        fs::write(&path, b"clip").expect("clip");
        let catalog = catalog_path(&dir);
        let mut library = ClipLibrary::open(output, catalog.clone()).expect("open");
        library.scan().expect("scan");
        library.set_protected(&path, true).expect("protect");

        fs::remove_file(&path).expect("remove clip");
        let entries = library.scan().expect("missing scan");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].missing);
        assert!(entries[0].protected);
        assert_eq!(entries[0].game_name.as_deref(), Some("Game"));

        let reloaded = ClipLibrary::open(dir.path.join("clips"), catalog).expect("reload");
        assert_eq!(reloaded.entries()[0].game_name.as_deref(), Some("Game"));
        assert!(reloaded.entries()[0].missing);
        assert!(reloaded.entries()[0].protected);
    }

    #[test]
    fn v1_catalog_is_migrated_without_losing_entry_metadata() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        let path = output.join("legacy.mp4");
        fs::write(&path, b"clip").expect("clip");
        let catalog = catalog_path(&dir);
        let old_catalog = serde_json::json!({
            "version": 1,
            "entries": [{
                "path": path.to_string_lossy(),
                "name": "legacy.mp4",
                "createdAtUnixMs": 123,
                "durationMs": 456,
                "sizeBytes": 999,
                "missing": false,
                "protected": true
            }]
        });
        fs::write(
            &catalog,
            serde_json::to_vec_pretty(&old_catalog).expect("render old catalog"),
        )
        .expect("old catalog");

        let mut library = ClipLibrary::open(output, catalog.clone()).expect("open v1");
        assert_eq!(library.entries()[0].game_name, None);
        let entries = library.scan().expect("migrate");
        assert_eq!(entries[0].created_at_unix_ms, Some(123));
        assert_eq!(entries[0].duration_ms, Some(456));
        assert!(entries[0].protected);

        let migrated: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(catalog).expect("read catalog"))
                .expect("parse catalog");
        assert_eq!(migrated["version"], serde_json::json!(2));
        assert_eq!(migrated["entries"][0]["gameName"], serde_json::Value::Null);
        assert_eq!(
            migrated["entries"][0]["createdAtUnixMs"],
            serde_json::json!(123)
        );
        assert_eq!(migrated["entries"][0]["durationMs"], serde_json::json!(456));
        assert_eq!(migrated["entries"][0]["protected"], serde_json::json!(true));
    }

    #[test]
    fn nested_actions_rename_and_delete_stay_in_the_game_directory() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        let game = output.join("Game");
        fs::create_dir_all(&game).expect("game directory");
        let old = game.join("old.mp4");
        fs::write(&old, b"clip").expect("clip");
        let mut library = ClipLibrary::open(output, catalog_path(&dir)).expect("open");
        library.scan().expect("scan");

        assert_eq!(library.checked_action_path(&old).expect("check"), old);
        let protected = library.set_protected(&old, true).expect("protect");
        let renamed = library.rename(&old, "renamed").expect("rename");
        assert_eq!(renamed.game_name.as_deref(), Some("Game"));
        assert!(renamed.protected);
        assert!(!old.exists());
        assert!(renamed.path.exists());
        assert_eq!(
            library
                .checked_action_path(&renamed.path)
                .expect("check renamed"),
            renamed.path
        );
        assert_eq!(protected.game_name.as_deref(), Some("Game"));

        library.delete(&renamed.path).expect("delete");
        assert!(!renamed.path.exists());
        assert!(library.entries().is_empty());
    }

    #[test]
    fn nested_temporary_files_are_cleaned_without_recursing_deeper() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        let game = output.join("Game");
        let deeper = game.join("nested");
        fs::create_dir_all(&deeper).expect("nested directories");
        let root_partial = output.join("root.mp4.part");
        let nested_partial = game.join("nested.mp4.part");
        let nested_delete = game.join(".nested.mp4.silk-delete");
        let deep_partial = deeper.join("deep.mp4.part");
        for path in [
            &root_partial,
            &nested_partial,
            &nested_delete,
            &deep_partial,
        ] {
            fs::write(path, b"temporary").expect("temporary file");
        }

        let _library = ClipLibrary::open(output, catalog_path(&dir)).expect("open");
        assert!(!root_partial.exists());
        assert!(!nested_partial.exists());
        assert!(!nested_delete.exists());
        assert!(deep_partial.exists());
    }

    #[test]
    fn quota_accounts_for_legacy_and_game_clips() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        let game = output.join("Game");
        fs::create_dir_all(&game).expect("game directory");
        let legacy_old = output.join("legacy-old.mp4");
        let game_old = game.join("game-old.mp4");
        let game_new = game.join("game-new.mp4");
        for path in [&legacy_old, &game_old, &game_new] {
            fs::write(path, b"abc").expect("clip");
        }

        let mut library = ClipLibrary::open(output.clone(), catalog_path(&dir)).expect("open");
        library.scan().expect("scan");
        for entry in &mut library.entries {
            entry.created_at_unix_ms = match entry.name.as_str() {
                "legacy-old.mp4" => Some(1),
                "game-old.mp4" => Some(2),
                "game-new.mp4" => Some(3),
                _ => None,
            };
        }
        save_catalog(&library.catalog_path, &library.entries).expect("catalog metadata");
        library.set_storage_policy(StoragePolicy {
            quota_bytes: Some(3),
            automatic_deletion_enabled: true,
        });

        let entries = library.scan().expect("enforce quota");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, game_new);
        assert_eq!(entries[0].game_name.as_deref(), Some("Game"));
        assert!(!legacy_old.exists());
        assert!(!game_old.exists());
        assert!(game_new.exists());
        let summary = library.storage_summary();
        assert_eq!(summary.total_bytes, 3);
    }

    #[test]
    fn rename_validates_names_and_preserves_the_clip_extension() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        let old = output.join("old.mp4");
        fs::write(&old, b"clip").expect("clip");
        let mut library = ClipLibrary::open(output.clone(), catalog_path(&dir)).expect("open");
        library.scan().expect("scan");

        let renamed = library.rename(&old, "new name").expect("rename");
        assert_eq!(renamed.name, "new name.mp4");
        assert!(!old.exists());
        assert!(output.join("new name.mp4").exists());
        assert!(matches!(
            library.rename(&renamed.path, "../escape"),
            Err(LibraryError::InvalidName { .. })
        ));
        assert!(matches!(
            library.rename(&renamed.path, "CON"),
            Err(LibraryError::InvalidName { .. })
        ));
    }

    #[test]
    fn delete_removes_files_and_catalog_rows() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        let path = output.join("delete-me.mp4");
        fs::write(&path, b"clip").expect("clip");
        let catalog = catalog_path(&dir);
        let mut library = ClipLibrary::open(output, catalog.clone()).expect("open");
        library.scan().expect("scan");
        library.delete(&path).expect("delete");
        assert!(!path.exists());
        assert!(library.entries().is_empty());
        let reloaded = ClipLibrary::open(dir.path.join("clips"), catalog).expect("reload");
        assert!(reloaded.entries().is_empty());
    }

    #[test]
    fn worker_reconciles_on_a_dedicated_bounded_bridge() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        fs::write(output.join("worker.mp4"), b"clip").expect("clip");
        let worker = LibraryWorker::new(output, catalog_path(&dir)).expect("worker");
        let entries = worker.scan().expect("scan");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "worker.mp4");
    }

    #[test]
    fn corrupt_catalog_is_rebuilt_from_filesystem_source_of_truth() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        fs::write(output.join("recover.mp4"), b"clip").expect("clip");
        let catalog = catalog_path(&dir);
        fs::write(&catalog, b"not-json").expect("corrupt catalog");

        let mut library = ClipLibrary::open(output, catalog).expect("open");
        let entries = library.scan().expect("rebuild");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "recover.mp4");
    }

    #[test]
    fn quota_removes_oldest_unprotected_files_only() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        fs::write(output.join("old.mp4"), b"old").expect("old");
        fs::write(output.join("protected.mp4"), b"keep").expect("protected");
        fs::write(output.join("new.mp4"), b"newest").expect("new");

        let mut library = ClipLibrary::open(output.clone(), catalog_path(&dir)).expect("open");
        library.scan().expect("scan");
        for entry in &mut library.entries {
            entry.created_at_unix_ms = match entry.name.as_str() {
                "old.mp4" => Some(1),
                "protected.mp4" => Some(2),
                "new.mp4" => Some(3),
                _ => None,
            };
            entry.protected = entry.name == "protected.mp4";
        }
        save_catalog(&library.catalog_path, &library.entries).expect("catalog metadata");
        library.set_storage_policy(StoragePolicy {
            quota_bytes: Some(7),
            automatic_deletion_enabled: true,
        });

        let entries = library.scan().expect("enforce quota");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "protected.mp4");
        assert!(entries[0].protected);
        assert!(!output.join("old.mp4").exists());
        assert!(!output.join("new.mp4").exists());
        assert_eq!(library.storage_summary().total_bytes, 4);
    }

    #[test]
    fn automatic_deletion_is_off_without_explicit_policy() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        let path = output.join("keep.mp4");
        fs::write(&path, b"keep").expect("clip");

        let mut library = ClipLibrary::open(output, catalog_path(&dir)).expect("open");
        library.set_storage_policy(StoragePolicy {
            quota_bytes: Some(1),
            automatic_deletion_enabled: false,
        });
        let entries = library.scan().expect("scan");
        assert_eq!(entries.len(), 1);
        assert!(path.exists());
        let summary = library.storage_summary();
        assert_eq!(summary.total_bytes, 4);
        assert!(!summary.automatic_deletion_enabled);
        assert_eq!(summary.deleted_count, 0);
    }

    #[test]
    fn action_paths_must_resolve_to_regular_indexed_files_in_output_directory() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        let path = output.join("clip.mp4");
        fs::write(&path, b"clip").expect("clip");
        let library = ClipLibrary::open(output, catalog_path(&dir)).expect("open");

        assert!(matches!(
            library.checked_action_path(&path),
            Err(LibraryError::NotIndexed { .. })
        ));
        assert!(matches!(
            library.checked_action_path(&dir.path.join("outside.mp4")),
            Err(LibraryError::InvalidPath { .. })
        ));
    }

    #[test]
    fn stale_delete_and_save_artifacts_are_removed_on_worker_open() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        let staged = output.join("clip.mp4.part");
        let deleted = output.join(".clip.mp4.silk-delete");
        fs::write(&staged, b"staged").expect("staged");
        fs::write(&deleted, b"deleted").expect("deleted");

        let _library = ClipLibrary::open(output, catalog_path(&dir)).expect("open");
        assert!(!staged.exists());
        assert!(!deleted.exists());
    }

    #[test]
    fn parses_mp4_duration_v0_and_v1() {
        let v0_bytes = mp4_with_duration_v0(1_000, 2_500);
        assert_eq!(parse_mp4_duration(&v0_bytes), Some(2_500));

        let v1_bytes = mp4_with_duration_v1(1_000, 5_000);
        assert_eq!(parse_mp4_duration(&v1_bytes), Some(5_000));

        let v0_custom_scale = mp4_with_duration_v0(600, 1_800);
        assert_eq!(parse_mp4_duration(&v0_custom_scale), Some(3_000));
    }

    #[test]
    fn parses_mp4_with_unknown_boxes_and_complex_hierarchy() {
        let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isommp41");
        let free = make_box(b"free", b"free-space-padding-data");
        let mdat = make_box(b"mdat", b"media-data-payload-bytes");
        let trak = make_box(b"trak", b"trak-child-atom-data");
        let mvhd = make_mvhd_v0(1_000, 3_500);
        let udta = make_box(b"udta", b"user-data-atom");
        let moov_payload = [trak, mvhd, udta].concat();
        let moov = make_box(b"moov", &moov_payload);
        let data = [ftyp, free, mdat, moov].concat();

        assert_eq!(parse_mp4_duration(&data), Some(3_500));
    }

    #[test]
    fn parses_mp4_extended_size_boxes() {
        // Extended size moov box
        let mvhd = make_mvhd_v0(1_000, 4_200);
        let moov_ext = make_box_ext(b"moov", &mvhd);
        assert_eq!(parse_mp4_duration(&moov_ext), Some(4_200));

        // Extended size mvhd box inside standard moov
        let mut mvhd_payload = Vec::new();
        mvhd_payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        mvhd_payload.extend_from_slice(&0_u32.to_be_bytes());
        mvhd_payload.extend_from_slice(&0_u32.to_be_bytes());
        mvhd_payload.extend_from_slice(&1_000_u32.to_be_bytes());
        mvhd_payload.extend_from_slice(&7_800_u32.to_be_bytes());
        mvhd_payload.extend_from_slice(&[0u8; 80]);
        let mvhd_ext = make_box_ext(b"mvhd", &mvhd_payload);
        let moov = make_box(b"moov", &mvhd_ext);
        assert_eq!(parse_mp4_duration(&moov), Some(7_800));
    }

    #[test]
    fn parses_mp4_size_to_eof_boxes() {
        let ftyp = make_box(b"ftyp", b"isom");
        let mvhd = make_mvhd_v0(1_000, 6_100);
        // moov box with size 0 extending to EOF
        let moov_to_eof = make_box_to_eof(b"moov", &mvhd);
        let data = [ftyp, moov_to_eof].concat();
        assert_eq!(parse_mp4_duration(&data), Some(6_100));

        // mvhd with size 0 inside moov
        let mut mvhd_payload = Vec::new();
        mvhd_payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        mvhd_payload.extend_from_slice(&0_u32.to_be_bytes());
        mvhd_payload.extend_from_slice(&0_u32.to_be_bytes());
        mvhd_payload.extend_from_slice(&1_000_u32.to_be_bytes());
        mvhd_payload.extend_from_slice(&9_200_u32.to_be_bytes());
        let mvhd_to_eof = make_box_to_eof(b"mvhd", &mvhd_payload);
        let moov = make_box(b"moov", &mvhd_to_eof);
        assert_eq!(parse_mp4_duration(&moov), Some(9_200));
    }

    #[test]
    fn rejects_malformed_truncated_and_zero_timescale_mp4() {
        assert_eq!(parse_mp4_duration(&[]), None);
        assert_eq!(parse_mp4_duration(b"short"), None);

        // Invalid box size (2..=7)
        let invalid_size_box = [0x00, 0x00, 0x00, 0x05, b'm', b'o', b'o', b'v'];
        assert_eq!(parse_mp4_duration(&invalid_size_box), None);

        // Extended box with invalid large size (< 16)
        let mut invalid_ext = vec![0x00, 0x00, 0x00, 0x01, b'm', b'o', b'o', b'v'];
        invalid_ext.extend_from_slice(&10_u64.to_be_bytes());
        assert_eq!(parse_mp4_duration(&invalid_ext), None);

        // Truncated declared box size
        let mut truncated = make_box(b"moov", b"too-short");
        truncated.truncate(truncated.len() - 3);
        assert_eq!(parse_mp4_duration(&truncated), None);

        // Zero timescale
        let zero_timescale = mp4_with_duration_v0(0, 1_000);
        assert_eq!(parse_mp4_duration(&zero_timescale), None);

        // Truncated mvhd payload (less than 20 bytes for v0)
        let mut short_mvhd_payload = vec![0x00, 0x00, 0x00, 0x00];
        short_mvhd_payload.extend_from_slice(&[0u8; 10]);
        let short_mvhd = make_box(b"mvhd", &short_mvhd_payload);
        let short_moov = make_box(b"moov", &short_mvhd);
        assert_eq!(parse_mp4_duration(&short_moov), None);

        // Unsupported mvhd version
        let mut unsupp_mvhd_payload = vec![0x02, 0x00, 0x00, 0x00];
        unsupp_mvhd_payload.extend_from_slice(&[0u8; 30]);
        let unsupp_mvhd = make_box(b"mvhd", &unsupp_mvhd_payload);
        let unsupp_moov = make_box(b"moov", &unsupp_mvhd);
        assert_eq!(parse_mp4_duration(&unsupp_moov), None);
    }

    #[test]
    fn scan_ignores_unindexed_mkv_files() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        let mp4_file = output.join("valid.mp4");
        let mkv_file = output.join("ignored.mkv");
        fs::write(&mp4_file, mp4_with_duration_v0(1_000, 1_000)).expect("mp4");
        fs::write(&mkv_file, b"mkv-data").expect("mkv");

        let mut library = ClipLibrary::open(output, catalog_path(&dir)).expect("open");
        let entries = library.scan().expect("scan");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, mp4_file);
        assert_eq!(entries[0].name, "valid.mp4");
    }

    #[test]
    fn stale_mkv_catalog_entries_remain_marked_missing() {
        let dir = TestDir::new();
        let output = dir.path.join("clips");
        fs::create_dir_all(&output).expect("output");
        let stale_mkv = output.join("stale.mkv");
        let catalog = catalog_path(&dir);
        let old_catalog = serde_json::json!({
            "version": 2,
            "entries": [{
                "path": stale_mkv.to_string_lossy(),
                "name": "stale.mkv",
                "createdAtUnixMs": 100,
                "durationMs": 200,
                "sizeBytes": 300,
                "missing": false,
                "protected": false
            }]
        });
        fs::write(
            &catalog,
            serde_json::to_vec_pretty(&old_catalog).expect("render catalog"),
        )
        .expect("catalog");

        let mut library = ClipLibrary::open(output, catalog).expect("open");
        let entries = library.scan().expect("scan");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "stale.mkv");
        assert!(entries[0].missing);
    }

    #[test]
    fn file_duration_handles_multi_megabyte_mdat_before_moov() {
        let dir = TestDir::new();
        let clip_path = dir.path.join("large_mdat.mp4");
        let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isommp41");
        let mvhd = make_mvhd_v0(1_000, 7_500);
        let moov = make_box(b"moov", &mvhd);

        let mdat_payload_len = 3 * 1024 * 1024_usize; // 3 MiB mdat
        let mdat_size = u32::try_from(mdat_payload_len + 8).expect("mdat size");
        let mut file = File::create(&clip_path).expect("create test file");
        file.write_all(&ftyp).expect("write ftyp");
        file.write_all(&mdat_size.to_be_bytes())
            .expect("write mdat size");
        file.write_all(b"mdat").expect("write mdat tag");
        let moov_pos = u64::try_from(ftyp.len() + 8 + mdat_payload_len).unwrap();
        file.seek(SeekFrom::Start(moov_pos)).expect("seek to moov");
        file.write_all(&moov).expect("write moov");
        file.flush().expect("flush");

        assert_eq!(duration_from_file(&clip_path), Some(7_500));

        let mut library = ClipLibrary::open(dir.path.clone(), catalog_path(&dir)).expect("open");
        let entries = library.scan().expect("scan");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].duration_ms, Some(7_500));
    }

    #[test]
    fn file_duration_handles_extended_size_mdat_before_moov() {
        let dir = TestDir::new();
        let clip_path = dir.path.join("ext_mdat.mp4");
        let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isommp41");
        let mvhd = make_mvhd_v1(1_000, 12_345);
        let moov = make_box(b"moov", &mvhd);

        let mdat_payload_len = 2 * 1024 * 1024_usize; // 2 MiB
        let large_size = u64::try_from(mdat_payload_len + 16).expect("large size");
        let mut file = File::create(&clip_path).expect("create test file");
        file.write_all(&ftyp).expect("write ftyp");
        file.write_all(&1_u32.to_be_bytes())
            .expect("write ext tag size");
        file.write_all(b"mdat").expect("write mdat tag");
        file.write_all(&large_size.to_be_bytes())
            .expect("write large size");
        let moov_pos = u64::try_from(ftyp.len() + 16 + mdat_payload_len).unwrap();
        file.seek(SeekFrom::Start(moov_pos)).expect("seek to moov");
        file.write_all(&moov).expect("write moov");
        file.flush().expect("flush");

        assert_eq!(duration_from_file(&clip_path), Some(12_345));
    }

    #[test]
    fn file_duration_rejects_malformed_and_truncated_headers() {
        let dir = TestDir::new();

        // Empty file
        let empty_path = dir.path.join("empty.mp4");
        fs::write(&empty_path, b"").expect("write empty");
        assert_eq!(duration_from_file(&empty_path), None);

        // Header shorter than 8 bytes
        let short_header = dir.path.join("short_header.mp4");
        fs::write(&short_header, b"ftyp").expect("write short header");
        assert_eq!(duration_from_file(&short_header), None);

        // Invalid box size in 2..=7
        let invalid_size_path = dir.path.join("invalid_size.mp4");
        let mut invalid_size_data = Vec::new();
        invalid_size_data.extend_from_slice(&5_u32.to_be_bytes());
        invalid_size_data.extend_from_slice(b"ftyp");
        fs::write(&invalid_size_path, &invalid_size_data).expect("write invalid size");
        assert_eq!(duration_from_file(&invalid_size_path), None);

        // Truncated extended header (less than 16 bytes)
        let trunc_ext_path = dir.path.join("trunc_ext.mp4");
        let mut trunc_ext_data = Vec::new();
        trunc_ext_data.extend_from_slice(&1_u32.to_be_bytes());
        trunc_ext_data.extend_from_slice(b"mdat");
        trunc_ext_data.extend_from_slice(&[0u8; 4]);
        fs::write(&trunc_ext_path, &trunc_ext_data).expect("write trunc ext");
        assert_eq!(duration_from_file(&trunc_ext_path), None);

        // Extended size < 16
        let bad_ext_size_path = dir.path.join("bad_ext_size.mp4");
        let mut bad_ext_data = Vec::new();
        bad_ext_data.extend_from_slice(&1_u32.to_be_bytes());
        bad_ext_data.extend_from_slice(b"mdat");
        bad_ext_data.extend_from_slice(&10_u64.to_be_bytes());
        fs::write(&bad_ext_size_path, &bad_ext_data).expect("write bad ext");
        assert_eq!(duration_from_file(&bad_ext_size_path), None);

        // Box size declares more bytes than file length
        let trunc_box_path = dir.path.join("trunc_box.mp4");
        let mut trunc_box_data = Vec::new();
        trunc_box_data.extend_from_slice(&100_u32.to_be_bytes());
        trunc_box_data.extend_from_slice(b"ftyp");
        trunc_box_data.extend_from_slice(b"short");
        fs::write(&trunc_box_path, &trunc_box_data).expect("write trunc box");
        assert_eq!(duration_from_file(&trunc_box_path), None);

        // Zero timescale in file
        let zero_timescale_path = dir.path.join("zero_timescale.mp4");
        fs::write(&zero_timescale_path, mp4_with_duration_v0(0, 1_000))
            .expect("write zero timescale");
        assert_eq!(duration_from_file(&zero_timescale_path), None);

        // File without moov
        let no_moov_path = dir.path.join("no_moov.mp4");
        let ftyp = make_box(b"ftyp", b"isom");
        let mdat = make_box(b"mdat", b"payload");
        fs::write(&no_moov_path, [ftyp, mdat].concat()).expect("write no moov");
        assert_eq!(duration_from_file(&no_moov_path), None);

        // Non-mp4 file extension
        let mkv_path = dir.path.join("valid.mkv");
        fs::write(&mkv_path, mp4_with_duration_v0(1_000, 2_000)).expect("write mkv");
        assert_eq!(duration_from_file(&mkv_path), None);
    }

    #[test]
    fn file_duration_rejects_oversized_moov() {
        let dir = TestDir::new();

        // 32-bit oversized moov > MAX_HEADER_BYTES
        let oversized_32_path = dir.path.join("oversized_32.mp4");
        let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isommp41");
        let declared_moov_size = (MAX_HEADER_BYTES + 4096) as u32;
        let mut file = File::create(&oversized_32_path).expect("create file");
        file.write_all(&ftyp).expect("write ftyp");
        file.write_all(&declared_moov_size.to_be_bytes())
            .expect("write moov size");
        file.write_all(b"moov").expect("write moov tag");
        let end_pos = u64::try_from(ftyp.len()).unwrap() + u64::from(declared_moov_size);
        file.set_len(end_pos).expect("set file len");
        file.flush().expect("flush");

        assert_eq!(duration_from_file(&oversized_32_path), None);

        // Extended 64-bit oversized moov > MAX_HEADER_BYTES
        let oversized_ext_path = dir.path.join("oversized_ext.mp4");
        let large_moov_size = (MAX_HEADER_BYTES + 8192) as u64;
        let mut file = File::create(&oversized_ext_path).expect("create file");
        file.write_all(&ftyp).expect("write ftyp");
        file.write_all(&1_u32.to_be_bytes()).expect("write ext tag");
        file.write_all(b"moov").expect("write moov tag");
        file.write_all(&large_moov_size.to_be_bytes())
            .expect("write large size");
        let end_pos = u64::try_from(ftyp.len()).unwrap() + large_moov_size;
        file.set_len(end_pos).expect("set file len");
        file.flush().expect("flush");

        assert_eq!(duration_from_file(&oversized_ext_path), None);

        // Size-to-EOF (size=0) oversized moov where remaining file > MAX_HEADER_BYTES
        let oversized_eof_path = dir.path.join("oversized_eof.mp4");
        let mut file = File::create(&oversized_eof_path).expect("create file");
        file.write_all(&ftyp).expect("write ftyp");
        file.write_all(&0_u32.to_be_bytes())
            .expect("write zero size");
        file.write_all(b"moov").expect("write moov tag");
        let end_pos = u64::try_from(ftyp.len()).unwrap() + (MAX_HEADER_BYTES as u64) + 1024;
        file.set_len(end_pos).expect("set file len");
        file.flush().expect("flush");

        assert_eq!(duration_from_file(&oversized_eof_path), None);
    }
}
