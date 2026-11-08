//! Signed Windows updates. Network, verification and shutdown run off media/UI threads.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

const MAX_INSTALLER_BYTES: usize = 128 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    current_version: String,
    version: Option<String>,
    notes: Option<String>,
    phase: String,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}

pub struct UpdateRuntime {
    busy: AtomicBool,
    status: Mutex<UpdateStatus>,
    pending: Mutex<Option<Update>>,
}

impl UpdateRuntime {
    pub fn new(version: String) -> Self {
        Self {
            busy: AtomicBool::new(false),
            status: Mutex::new(UpdateStatus {
                current_version: version,
                version: None,
                notes: None,
                phase: "idle".into(),
                downloaded: 0,
                total: None,
                error: None,
            }),
            pending: Mutex::new(None),
        }
    }

    fn begin(&self) -> Result<Operation<'_>, String> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "An update operation is already running.".to_string())?;
        Ok(Operation(&self.busy))
    }

    fn change(&self, action: impl FnOnce(&mut UpdateStatus)) {
        if let Ok(mut status) = self.status.lock() {
            action(&mut status);
        }
    }

    fn fail(&self, error: &str) {
        self.change(|s| {
            s.phase = "error".into();
            s.error = Some(error.into());
        });
        diagnostics::warn("updates", error);
    }
}

struct Operation<'a>(&'a AtomicBool);
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub fn get_update_status(state: State<'_, UpdateRuntime>) -> Result<UpdateStatus, String> {
    state
        .status
        .lock()
        .map(|s| s.clone())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<(), String> {
    let runtime = app.state::<UpdateRuntime>();
    let _operation = runtime.begin()?;
    runtime.change(|s| {
        s.phase = "checking".into();
        s.error = None;
    });
    let result: Result<(), String> = async {
        let update = app
            .updater_builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())?;
        if let Some(update) = &update {
            validate_download_url(&update.download_url, &update.version)?;
        }
        runtime.change(|s| {
            s.phase = if update.is_some() {
                "available"
            } else {
                "current"
            }
            .into();
            s.version = update.as_ref().map(|u| u.version.clone());
            s.notes = update
                .as_ref()
                .and_then(|u| u.body.clone())
                .map(|s| s.chars().take(8000).collect());
            s.downloaded = 0;
            s.total = None;
        });
        *runtime.pending.lock().map_err(|e| e.to_string())? = update;
        Ok(())
    }
    .await;
    if let Err(error) = &result {
        runtime.fail(error);
    }
    result
}

fn validate_download_url(url: &reqwest::Url, version: &str) -> Result<(), String> {
    #[cfg(feature = "updater-test-server")]
    if url.scheme() == "http" && url.host_str() == Some("127.0.0.1") {
        return Ok(());
    }
    let expected = format!(
        "/TrulyScarlet/silk-studio/releases/download/v{version}/Silk_{version}_x64-setup.exe"
    );
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != expected
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("The update does not point to an official Silk release.".into());
    }
    Ok(())
}

fn check_download_size(length: u64) -> Result<(), String> {
    if length > MAX_INSTALLER_BYTES as u64 {
        Err("The update exceeds Silk's 128 MB installer limit.".into())
    } else {
        Ok(())
    }
}

async fn download(update: &Update, runtime: &UpdateRuntime) -> Result<Vec<u8>, String> {
    validate_download_url(&update.download_url, &update.version)?;
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .connect_timeout(Duration::from_secs(15))
        .user_agent("Silk-Updater")
        .redirect(reqwest::redirect::Policy::limited(5));
    // The test feature is never enabled in distributable builds.
    if !cfg!(feature = "updater-test-server") {
        builder = builder.https_only(true);
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let mut response = client
        .get(update.download_url.clone())
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    let total = response.content_length();
    if let Some(total) = total {
        check_download_size(total)?;
    }
    runtime.change(|s| {
        s.phase = "downloading".into();
        s.downloaded = 0;
        s.total = total;
    });
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        check_download_size(bytes.len() as u64 + chunk.len() as u64)?;
        bytes.extend_from_slice(&chunk);
        runtime.change(|s| s.downloaded = bytes.len() as u64);
    }
    if bytes.is_empty() {
        return Err("The update download was empty.".into());
    }
    if total.is_some_and(|total| total != bytes.len() as u64) {
        return Err("The update download was incomplete. Please try again.".into());
    }
    runtime.change(|s| s.phase = "verifying".into());
    update
        .verify(&bytes)
        .map_err(|e| format!("Update verification failed: {e}"))?;
    Ok(bytes)
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let runtime = app.state::<UpdateRuntime>();
    let _operation = runtime.begin()?;
    let result: Result<(), String> = async {
        let update = runtime
            .pending
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or("Check for a new version before installing an update.")?;
        runtime.change(|s| {
            s.phase = "downloading".into();
            s.downloaded = 0;
            s.total = None;
            s.error = None;
        });
        // Capture continues while downloading. Only verified bytes can reach shutdown.
        let bytes = download(&update, &runtime).await?;
        let install_app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let shell = install_app.state::<crate::shell::ShellState>();
            let was_running = crate::shell::recorder_status(&shell)
                .map(|s| {
                    matches!(
                        s.state.as_str(),
                        "Ready" | "Buffering" | "Saving" | "Starting"
                    )
                })
                .unwrap_or(false);
            shell.set_updating(true);
            let result = (|| {
                install_app
                    .state::<crate::sharing::ShareRuntime>()
                    .ensure_idle()?;
                crate::shell::prepare_for_update(
                    &shell,
                    &crate::TauriEventSink(install_app.clone()),
                )?;
                install_app
                    .state::<UpdateRuntime>()
                    .change(|s| s.phase = "installing".into());
                // Tauri verifies again, launches NSIS /UPDATE /P /R, then exits.
                update
                    .install(bytes)
                    .map_err(|e| format!("Could not start the update installer: {e}"))
            })();
            // Only reached on a failure on Windows. Keep the old app usable.
            shell.set_updating(false);
            if result.is_err() && was_running {
                let _ = crate::shell::execute_command(
                    &shell,
                    &crate::TauriEventSink(install_app.clone()),
                    &app_controller::Command::StartCapture,
                );
            }
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    if let Err(error) = &result {
        runtime.fail(error);
    }
    result
}

pub fn start_checks(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        let mut last_check: Option<Instant> = None;
        loop {
            let enabled = crate::shell::app_settings(&app.state::<crate::shell::ShellState>())
                .map(|s| s.application.check_for_updates)
                .unwrap_or(false);
            if enabled && last_check.is_none_or(|last| last.elapsed() >= CHECK_INTERVAL) {
                let _ = tauri::async_runtime::block_on(check_for_updates(app.clone()));
                last_check = Some(Instant::now());
            }
            if !enabled {
                last_check = None;
            }
            std::thread::sleep(Duration::from_secs(30));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_versioned_official_installer() {
        let good = "https://github.com/TrulyScarlet/silk-studio/releases/download/v0.2.0/Silk_0.2.0_x64-setup.exe";
        assert!(validate_download_url(&good.parse().unwrap(), "0.2.0").is_ok());
        for bad in [
            good.replace("https:", "http:"),
            good.replace("github.com", "github.com.evil.test"),
            good.replace("TrulyScarlet", "someone-else"),
            good.replace("v0.2.0/", "v0.1.0/"),
            format!("{good}?other=1"),
            good.replace("https://", "https://user@"),
            good.replace("_x64-setup.exe", "_x64-setup.zip"),
        ] {
            assert!(
                validate_download_url(&bad.parse().unwrap(), "0.2.0").is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn download_limit_and_single_operation_are_enforced() {
        assert!(check_download_size(MAX_INSTALLER_BYTES as u64).is_ok());
        assert!(check_download_size(MAX_INSTALLER_BYTES as u64 + 1).is_err());
        let runtime = UpdateRuntime::new("0.1.0".into());
        let operation = runtime.begin().unwrap();
        assert!(runtime.begin().is_err());
        drop(operation);
        assert!(runtime.begin().is_ok());
    }
}
