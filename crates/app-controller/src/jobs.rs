//! Bounded save-job dispatch (BUF-008).
//!
//! `SaveWorker` owns container I/O on one dedicated thread. The small
//! `SaveJobManager` remains as a deterministic queue primitive for callers
//! that only need to rehearse submission ordering.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use media_types::MediaSnapshot;
use muxer::{ClipMetadata, Muxer, MuxerError, SaveOptions};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SAVE_QUEUE_BOUND: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveJob {
    pub clip_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JobQueueError {
    #[error("save queue is full ({bound} pending); try again shortly")]
    QueueFull { bound: usize },

    #[error("save worker is stopped")]
    WorkerStopped,
}

/// Owned save work handed to the bounded disk worker.
#[derive(Debug)]
pub struct SaveWork {
    pub snapshot: MediaSnapshot,
    pub clip_path: PathBuf,
    pub minimum_free_space_bytes: u64,
    pub options: SaveOptions,
}

/// Result emitted after one save worker job finishes.
#[derive(Debug)]
pub enum SaveResult {
    Completed(ClipMetadata),
    Failed { path: PathBuf, error: MuxerError },
}

/// Bounded save-pipeline telemetry. Updates happen on the worker or submitter
/// for a short mutex section and never span snapshot or file I/O.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveMetrics {
    pub saves_queued: u64,
    pub saves_completed: u64,
    pub saves_failed: u64,
    pub queue_full: u64,
    pub mux_failures: u64,
    pub total_save_duration_ms: u64,
    pub last_save_duration_ms: u64,
    pub available_disk_space_bytes: Option<u64>,
}

const FIXED_OUTPUT_OVERHEAD_BYTES: u64 = 1024 * 1024;
const PACKET_OUTPUT_OVERHEAD_BYTES: u64 = 64;

type SpaceProbe = Box<dyn Fn(&Path) -> io::Result<u64> + Send + 'static>;

/// One bounded save worker. The worker owns the muxer and is the only thread
/// allowed to perform container I/O; capture and controller threads exchange
/// immutable snapshots with it.
pub struct SaveWorker {
    sender: Option<SyncSender<SaveWork>>,
    results: Option<Receiver<SaveResult>>,
    worker: Option<JoinHandle<()>>,
    metrics: Arc<Mutex<SaveMetrics>>,
}

impl SaveWorker {
    pub fn new(muxer: Box<dyn Muxer>) -> io::Result<Self> {
        Self::with_space_probe(muxer, Box::new(available_space_bytes))
    }

    fn with_space_probe(muxer: Box<dyn Muxer>, space_probe: SpaceProbe) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(SAVE_QUEUE_BOUND);
        let (result_sender, results) = mpsc::sync_channel(SAVE_QUEUE_BOUND);
        let metrics = Arc::new(Mutex::new(SaveMetrics::default()));
        let worker_metrics = Arc::clone(&metrics);
        let worker = thread::Builder::new()
            .name("silk-save-worker".to_string())
            .spawn(move || {
                enter_background_processing_mode();
                let mut muxer = muxer;
                while let Ok(work) = receiver.recv() {
                    let started = Instant::now();
                    let SaveWork {
                        snapshot,
                        clip_path,
                        minimum_free_space_bytes,
                        options,
                    } = work;
                    let result = match space_probe(&clip_path) {
                        Ok(available_bytes) => {
                            update_metrics(&worker_metrics, |metrics| {
                                metrics.available_disk_space_bytes = Some(available_bytes);
                            });
                            let required_bytes =
                                required_free_space(&snapshot, minimum_free_space_bytes);
                            if available_bytes < required_bytes {
                                SaveResult::Failed {
                                    path: clip_path.clone(),
                                    error: MuxerError::InsufficientDiskSpace {
                                        path: clip_path,
                                        available_bytes,
                                        required_bytes,
                                    },
                                }
                            } else {
                                match muxer.write_snapshot(&snapshot, &clip_path, &options) {
                                    Ok(metadata) => SaveResult::Completed(metadata),
                                    Err(error) => SaveResult::Failed {
                                        path: clip_path,
                                        error,
                                    },
                                }
                            }
                        }
                        Err(source) => SaveResult::Failed {
                            path: clip_path.clone(),
                            error: MuxerError::OutputDirectoryUnavailable {
                                path: clip_path,
                                source,
                            },
                        },
                    };
                    let duration_ms = started.elapsed().as_millis() as u64;
                    update_metrics(&worker_metrics, |metrics| {
                        metrics.last_save_duration_ms = duration_ms;
                        metrics.total_save_duration_ms =
                            metrics.total_save_duration_ms.saturating_add(duration_ms);
                        match &result {
                            SaveResult::Completed(_) => {
                                metrics.saves_completed = metrics.saves_completed.saturating_add(1);
                            }
                            SaveResult::Failed { error, .. } => {
                                metrics.saves_failed = metrics.saves_failed.saturating_add(1);
                                if !matches!(
                                    error,
                                    MuxerError::InsufficientDiskSpace { .. }
                                        | MuxerError::OutputDirectoryUnavailable { .. }
                                ) {
                                    metrics.mux_failures = metrics.mux_failures.saturating_add(1);
                                }
                            }
                        }
                    });
                    if result_sender.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            results: Some(results),
            worker: Some(worker),
            metrics,
        })
    }

    pub fn submit(&self, work: SaveWork) -> Result<(), JobQueueError> {
        let Some(sender) = self.sender.as_ref() else {
            update_metrics(&self.metrics, |metrics| {
                metrics.saves_failed = metrics.saves_failed.saturating_add(1);
            });
            return Err(JobQueueError::WorkerStopped);
        };
        sender.try_send(work).map_err(|error| {
            update_metrics(&self.metrics, |metrics| {
                metrics.saves_failed = metrics.saves_failed.saturating_add(1);
                if matches!(&error, TrySendError::Full(_)) {
                    metrics.queue_full = metrics.queue_full.saturating_add(1);
                }
            });
            match error {
                TrySendError::Full(_) => JobQueueError::QueueFull {
                    bound: SAVE_QUEUE_BOUND,
                },
                TrySendError::Disconnected(_) => JobQueueError::WorkerStopped,
            }
        })?;
        update_metrics(&self.metrics, |metrics| {
            metrics.saves_queued = metrics.saves_queued.saturating_add(1);
        });
        Ok(())
    }

    pub fn metrics(&self) -> SaveMetrics {
        self.metrics
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or_default()
    }

    pub fn drain(&self) -> Vec<SaveResult> {
        self.results
            .as_ref()
            .map(|results| results.try_iter().collect())
            .unwrap_or_default()
    }

    /// Stop accepting work, finish queued saves, and return all completions.
    pub fn shutdown(&mut self) -> Vec<SaveResult> {
        self.sender.take();
        let mut results = Vec::new();
        if let Some(worker) = self.worker.take() {
            // The result channel is bounded too. Drain while waiting so a
            // burst of completed jobs cannot deadlock shutdown.
            while !worker.is_finished() {
                results.extend(self.drain());
                thread::sleep(Duration::from_millis(1));
            }
            let _ = worker.join();
        }
        results.extend(self.drain());
        results
    }
}

impl Drop for SaveWorker {
    fn drop(&mut self) {
        self.sender.take();
        // No caller is available to consume events during an abnormal drop;
        // close the result channel so the worker can exit without blocking.
        self.results.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Keep synchronous container and filesystem work below the capture/game
/// scheduling path. Windows background processing mode also lowers the thread's
/// I/O priority; other platforms retain the existing worker behavior.
#[cfg(windows)]
fn enter_background_processing_mode() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
    };

    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

#[cfg(not(windows))]
fn enter_background_processing_mode() {}

fn update_metrics(metrics: &Arc<Mutex<SaveMetrics>>, update: impl FnOnce(&mut SaveMetrics)) {
    if let Ok(mut metrics) = metrics.lock() {
        update(&mut metrics);
    }
}

fn required_free_space(snapshot: &MediaSnapshot, minimum_free_space_bytes: u64) -> u64 {
    let payload_bytes = snapshot
        .streams
        .iter()
        .flat_map(|stream| stream.packets.iter())
        .map(|packet| packet.payload.len() as u64)
        .sum::<u64>();
    let packet_overhead =
        (snapshot.packet_count() as u64).saturating_mul(PACKET_OUTPUT_OVERHEAD_BYTES);
    minimum_free_space_bytes
        .saturating_add(FIXED_OUTPUT_OVERHEAD_BYTES)
        .saturating_add(payload_bytes)
        .saturating_add(packet_overhead)
}

fn available_space_bytes(path: &Path) -> io::Result<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

        let mut directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        while !directory.exists() {
            let Some(parent) = directory.parent() else {
                directory = PathBuf::from(".");
                break;
            };
            if parent == directory {
                directory = PathBuf::from(".");
                break;
            }
            directory = parent.to_path_buf();
        }

        let wide: Vec<u16> = directory
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut available = 0_u64;
        let success = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(available)
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(u64::MAX)
    }
}

#[derive(Debug, Default)]
pub struct SaveJobManager {
    queue: Mutex<VecDeque<SaveJob>>,
}

impl SaveJobManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Submit a clip for writing. Serialized: callers must treat jobs as
    /// pending until `process_next` returns them.
    pub fn submit(&self, job: SaveJob) -> Result<(), JobQueueError> {
        let mut guard = self.queue.lock().map_err(|_| JobQueueError::QueueFull {
            bound: SAVE_QUEUE_BOUND,
        })?;
        if guard.len() >= SAVE_QUEUE_BOUND {
            return Err(JobQueueError::QueueFull {
                bound: SAVE_QUEUE_BOUND,
            });
        }
        guard.push_back(job);
        Ok(())
    }

    /// Pop the next job in submission order.
    pub fn process_next(&self) -> Option<SaveJob> {
        self.queue.lock().ok().and_then(|mut g| g.pop_front())
    }

    pub fn pending_count(&self) -> usize {
        self.queue.lock().map(|g| g.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_types::{EncodedPacket, MediaType, PacketPayload, StreamDescriptor, StreamId};
    use std::sync::Arc;
    use test_support::{MockMuxer, TempDir};

    fn snapshot() -> MediaSnapshot {
        MediaSnapshot {
            origin_pts: 0,
            time_base: media_types::TimeBase::MILLISECOND,
            streams: vec![media_types::StreamPackets {
                descriptor: StreamDescriptor {
                    stream_id: StreamId(0),
                    media_type: MediaType::Video,
                    time_base: media_types::TimeBase::MILLISECOND,
                    name: None,
                    codec: "h264".to_string(),
                    extradata: None,
                    width: None,
                    height: None,
                    sample_rate: None,
                    channels: None,
                    pixel_format: None,
                },
                packets: vec![Arc::new(EncodedPacket {
                    stream_id: StreamId(0),
                    media_type: MediaType::Video,
                    pts: 0,
                    dts: 0,
                    duration: 33,
                    time_base: media_types::TimeBase::MILLISECOND,
                    is_keyframe: true,
                    sequence: 0,
                    payload: PacketPayload::from(&[0x01_u8][..]),
                })],
            }],
            captured_at_unix_ms: 0,
        }
    }

    fn job(n: u8) -> SaveJob {
        SaveJob {
            clip_path: PathBuf::from(format!("clip-{n}.mp4")),
        }
    }

    #[test]
    fn preserves_submission_order() {
        let mgr = SaveJobManager::new();
        for n in 0..4 {
            mgr.submit(job(n)).expect("submit");
        }
        assert_eq!(mgr.pending_count(), 4);
        for n in 0..4 {
            let next = mgr.process_next().expect("job present");
            assert_eq!(next.clip_path, PathBuf::from(format!("clip-{n}.mp4")));
        }
        assert!(mgr.process_next().is_none());
    }

    #[test]
    fn enforces_bound() {
        let mgr = SaveJobManager::new();
        for n in 0..SAVE_QUEUE_BOUND {
            assert!(mgr.submit(job(n as u8)).is_ok());
        }
        assert!(matches!(
            mgr.submit(job(u8::MAX)),
            Err(JobQueueError::QueueFull { bound }) if bound == SAVE_QUEUE_BOUND
        ));
    }

    #[test]
    fn save_worker_reports_completion_without_blocking_submitter() {
        let dir = TempDir::new("save-worker").expect("temp dir");
        let path = dir.path().join("worker.mp4");
        let mut worker = SaveWorker::new(Box::new(MockMuxer::default())).expect("worker");
        worker
            .submit(SaveWork {
                snapshot: snapshot(),
                clip_path: path.clone(),
                minimum_free_space_bytes: 0,
                options: SaveOptions::default(),
            })
            .expect("submit");

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut results = Vec::new();
        while std::time::Instant::now() < deadline && results.is_empty() {
            results.extend(worker.drain());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(results.as_slice(), [SaveResult::Completed(_)]));
        assert!(path.exists());
        let metrics = worker.metrics();
        assert_eq!(metrics.saves_queued, 1);
        assert_eq!(metrics.saves_completed, 1);
        assert_eq!(metrics.saves_failed, 0);
        assert!(metrics.last_save_duration_ms <= metrics.total_save_duration_ms);
        assert!(metrics.available_disk_space_bytes.is_some());
        assert!(worker.shutdown().is_empty());
    }

    #[test]
    fn rejects_save_when_space_probe_is_below_requirement() {
        let dir = TempDir::new("save-worker-low-space").expect("temp dir");
        let path = dir.path().join("worker.mp4");
        let mut worker =
            SaveWorker::with_space_probe(Box::new(MockMuxer::default()), Box::new(|_| Ok(0)))
                .expect("worker");
        worker
            .submit(SaveWork {
                snapshot: snapshot(),
                clip_path: path.clone(),
                minimum_free_space_bytes: 0,
                options: SaveOptions::default(),
            })
            .expect("submit");

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut results = Vec::new();
        while std::time::Instant::now() < deadline && results.is_empty() {
            results.extend(worker.drain());
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(
            results.as_slice(),
            [SaveResult::Failed {
                error: MuxerError::InsufficientDiskSpace {
                    available_bytes: 0,
                    ..
                },
                ..
            }]
        ));
        assert!(!path.exists());
        let metrics = worker.metrics();
        assert_eq!(metrics.saves_queued, 1);
        assert_eq!(metrics.saves_failed, 1);
        assert_eq!(metrics.mux_failures, 0);
        assert_eq!(metrics.available_disk_space_bytes, Some(0));
        assert!(worker.shutdown().is_empty());
    }
}
