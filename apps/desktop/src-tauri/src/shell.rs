//! Testable desktop-shell logic: command execution and event forwarding
//! live here so the Tauri glue in `main.rs` stays minimal and the actual
//! IPC handler behavior is covered by automated tests.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use app_controller::{
    AudioDepsFactory, Command, ConsoleNotificationSink, Controller, ControllerEvent,
    ControllerSettings, DepsFactory, NotificationSink, SaveMetrics,
};
#[cfg(windows)]
use audio_wasapi::WasapiAudioCapture;
#[cfg(windows)]
use capture_windows::WgcDisplayCapture;
use chrono::Utc;
use clip_library::{ClipEntry, LibraryError, LibraryWorker, StoragePolicy, StorageSummary};
use configuration::{AppConfig, AudioSourceKind, HotkeysSettings};
#[cfg(windows)]
use encoder_api::VideoEncoderFactory;
use hotkeys::{
    validate_bindings, HotkeyAction, HotkeyBinding, HotkeyError, HotkeyEvent, HotkeyManager,
    SAVE_REPLAY_HOTKEY_ID, START_CAPTURE_HOTKEY_ID, STOP_CAPTURE_HOTKEY_ID,
};
use media_types::{AudioDeviceInfo, StreamId, VideoSourceInfo, WindowSourceInfo};
use muxer::Mp4Muxer;
use recorder_engine::AudioInput;
use serde::Serialize;
use serde_json::Value;

const MAX_DIAGNOSTIC_PACKAGE_BYTES: usize = 8 * 1024 * 1024;
const MANAGED_CLIP_DIRECTORY_NAME: &str = "Silk";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeMediaMode {
    /// Environment-controlled mode used by deterministic shell and native
    /// smoke tests.
    Environment,
    /// Real Windows capture and encoding used by the configured application.
    Native,
}

#[cfg(windows)]
fn native_backend_requested(mode: RuntimeMediaMode, environment_variable: &str) -> bool {
    mode == RuntimeMediaMode::Native || std::env::var(environment_variable).as_deref() == Ok("1")
}

use test_support::{
    AudioStep, MockAudioEncoder, MockVideoEncoder, ScriptedAudioCapture, ScriptedVideoCapture,
    VideoStep,
};

/// Everything the window manages; plain data plus the controller.
pub struct ShellState {
    pub controller: Mutex<Controller>,
    pub hotkeys: Mutex<Option<HotkeyManager>>,
    pub library: Mutex<Option<LibraryWorker>>,
    settings: Mutex<AppConfig>,
    config_path: PathBuf,
    minimize_to_tray: AtomicBool,
    notifications_enabled: AtomicBool,
    clip_sound_enabled: AtomicBool,
    exit_requested: AtomicBool,
    last_foreground_process: Mutex<Option<String>>,
}

impl ShellState {
    #[allow(dead_code)]
    pub fn new(controller: Controller, hotkeys: Option<HotkeyManager>) -> Self {
        Self::with_app_config(
            controller,
            hotkeys,
            AppConfig::default(),
            development_config_path(),
            true,
        )
    }

    #[allow(dead_code)]
    pub fn with_minimize_to_tray(
        controller: Controller,
        hotkeys: Option<HotkeyManager>,
        minimize_to_tray: bool,
    ) -> Self {
        Self::with_app_config(
            controller,
            hotkeys,
            AppConfig::default(),
            development_config_path(),
            minimize_to_tray,
        )
    }

    pub fn with_app_config(
        controller: Controller,
        hotkeys: Option<HotkeyManager>,
        settings: AppConfig,
        config_path: PathBuf,
        minimize_to_tray: bool,
    ) -> Self {
        let notifications_enabled = settings.application.notifications_enabled;
        let clip_sound_enabled = settings.application.clip_sound_enabled;
        let managed_output_dir = managed_clip_directory(Path::new(&settings.output.directory));
        if let Err(error) = std::fs::create_dir_all(&managed_output_dir) {
            diagnostics::warn(
                "library",
                &format!(
                    "managed clip directory could not be prepared at {}: {error}",
                    managed_output_dir.display()
                ),
            );
        }
        let library = match LibraryWorker::new_with_policy(
            managed_output_dir,
            library_catalog_path(&config_path),
            storage_policy_from_config(&settings),
        ) {
            Ok(worker) => Some(worker),
            Err(error) => {
                diagnostics::warn(
                    "library",
                    &format!("clip library worker could not start: {error}"),
                );
                None
            }
        };
        Self {
            controller: Mutex::new(controller),
            hotkeys: Mutex::new(hotkeys),
            library: Mutex::new(library),
            settings: Mutex::new(settings),
            config_path,
            minimize_to_tray: AtomicBool::new(minimize_to_tray),
            notifications_enabled: AtomicBool::new(notifications_enabled),
            clip_sound_enabled: AtomicBool::new(clip_sound_enabled),
            exit_requested: AtomicBool::new(false),
            last_foreground_process: Mutex::new(None),
        }
    }

    pub fn request_exit(&self) {
        self.exit_requested.store(true, Ordering::Release);
    }

    pub(crate) fn close_action(&self) -> CloseAction {
        close_action(
            self.minimize_to_tray.load(Ordering::Acquire),
            self.exit_requested.swap(false, Ordering::Acquire),
        )
    }

    pub(crate) fn notifications_enabled(&self) -> bool {
        self.notifications_enabled.load(Ordering::Acquire)
    }

    pub(crate) fn clip_sound_enabled(&self) -> bool {
        self.clip_sound_enabled.load(Ordering::Acquire)
    }
}

/// The configured directory is a user-selected base. Silk owns only the
/// child directory below it, so changing the setting never makes the selected
/// folder itself the application's media root.
pub(crate) fn managed_clip_directory(base_directory: &Path) -> PathBuf {
    base_directory.join(MANAGED_CLIP_DIRECTORY_NAME)
}

#[cfg(windows)]
fn foreground_process_name() -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    };

    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return None;
    }
    let mut process_id = 0_u32;
    if unsafe { GetWindowThreadProcessId(window, &mut process_id) } == 0
        || process_id == unsafe { GetCurrentProcessId() }
    {
        return None;
    }

    let title = {
        let title_len = unsafe { GetWindowTextLengthW(window) };
        if title_len > 0 {
            let mut title_buf = vec![0_u16; (title_len as usize) + 1];
            let copied =
                unsafe { GetWindowTextW(window, title_buf.as_mut_ptr(), title_buf.len() as i32) };
            if copied > 0 {
                String::from_utf16_lossy(&title_buf[..copied as usize])
                    .trim()
                    .to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    };
    let title_opt = if title.is_empty() {
        None
    } else {
        Some(title.as_str())
    };

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return None;
    }

    let mut buffer = vec![0_u16; 32_768];
    let mut length = u32::try_from(buffer.len()).ok()?;
    let found =
        unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) != 0 };
    unsafe {
        CloseHandle(process);
    }
    if !found || length == 0 {
        return None;
    }

    let path = PathBuf::from(OsString::from_wide(&buffer[..length as usize]));
    path.file_stem()
        .map(|name| {
            let raw = name.to_string_lossy();
            app_controller::clean_game_name(&raw, title_opt)
        })
        .filter(|name| !name.is_empty())
}

impl ShellState {
    fn observe_foreground_process(&self) {
        #[cfg(windows)]
        if let Some(process_name) = foreground_process_name() {
            if let Ok(mut previous) = self.last_foreground_process.lock() {
                *previous = Some(process_name);
            }
        }
    }

    fn clip_attribution(&self) -> Option<app_controller::ClipAttribution> {
        #[cfg(windows)]
        {
            self.last_foreground_process
                .lock()
                .ok()
                .and_then(|process_name| process_name.clone())
                .map(|game_name| app_controller::ClipAttribution {
                    game_name: Some(game_name),
                })
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

fn development_config_path() -> PathBuf {
    std::env::temp_dir().join("silk-dev-config.json")
}

fn library_catalog_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("library.json")
}

fn storage_policy_from_config(settings: &AppConfig) -> StoragePolicy {
    let quota_bytes = settings.storage.quota_enabled.then(|| {
        settings
            .storage
            .quota_gigabytes
            .saturating_mul(1024 * 1024 * 1024)
    });
    StoragePolicy {
        quota_bytes,
        automatic_deletion_enabled: settings.storage.automatic_deletion_enabled,
    }
}

/// Apply the opt-in per-user Windows startup registration. The registry value
/// is deliberately scoped to HKCU and contains only the current executable.
pub(crate) fn apply_startup_registration(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        if cfg!(test) {
            return Ok(());
        }
        set_start_with_windows(enabled)
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Ok(())
    }
}

#[cfg(windows)]
fn set_start_with_windows(enabled: bool) -> Result<(), String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    let key_path: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value_name: Vec<u16> = "Silk".encode_utf16().chain(std::iter::once(0)).collect();
    let startup_command = if enabled {
        let executable = std::env::current_exe()
            .map_err(|error| format!("could not resolve the current executable: {error}"))?;
        Some(
            format!("\"{}\"", executable.display())
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let mut key: HKEY = null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "could not open the per-user startup key (Windows error {status})"
        ));
    }

    let status = if enabled {
        let command = startup_command
            .as_ref()
            .expect("startup command is present when enabled");
        unsafe {
            RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                u32::try_from(command.len() * std::mem::size_of::<u16>())
                    .map_err(|_| "startup command is too long".to_string())?,
            )
        }
    } else {
        unsafe { RegDeleteValueW(key, value_name.as_ptr()) }
    };
    unsafe {
        RegCloseKey(key);
    }
    if status == ERROR_SUCCESS || (!enabled && status == ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(format!(
            "could not {} per-user startup registration (Windows error {status})",
            if enabled { "enable" } else { "disable" }
        ))
    }
}

pub fn set_diagnostic_level(level: configuration::DiagnosticLogLevel) {
    let level = match level {
        configuration::DiagnosticLogLevel::Trace => diagnostics::Level::Trace,
        configuration::DiagnosticLogLevel::Debug => diagnostics::Level::Debug,
        configuration::DiagnosticLogLevel::Info => diagnostics::Level::Info,
        configuration::DiagnosticLogLevel::Warn => diagnostics::Level::Warn,
        configuration::DiagnosticLogLevel::Error => diagnostics::Level::Error,
    };
    diagnostics::set_level(level);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticGraphicsAdapter {
    pub name: String,
    pub vendor_id: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEncoderBackend {
    pub backend_name: String,
    pub codec: String,
    pub hardware_accelerated: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub supported_fps: Vec<u32>,
    pub supported_pixel_formats: Vec<media_types::PixelFormat>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticBuildInfo {
    pub app_name: String,
    pub app_version: String,
    pub build_commit: Option<String>,
    pub build_date: Option<String>,
    pub rust_version: Option<String>,
    pub target: String,
    pub operating_system: String,
    pub os_version: Option<String>,
    pub architecture: String,
    pub graphics_adapters: Vec<DiagnosticGraphicsAdapter>,
    pub gpu_driver_versions: Vec<String>,
    pub wgc_supported: bool,
    pub encoder_backends: Vec<DiagnosticEncoderBackend>,
    pub fidelity_capabilities: Vec<app_controller::FidelityCapability>,
    pub ffmpeg_available: bool,
    pub ffprobe_available: bool,
    pub probe_warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticPackage {
    schema_version: u32,
    generated_at: String,
    media_included: bool,
    build: DiagnosticBuildInfo,
    settings: Value,
    recent_logs: Vec<Value>,
    recent_errors: Vec<Value>,
}

/// Return build identity and best-effort local capability probes. Probes are
/// deliberately diagnostic-only and never participate in the media pipeline.
pub fn diagnostic_build_info() -> DiagnosticBuildInfo {
    let (graphics_adapters, wgc_supported, encoder_backends, mut probe_warnings) =
        native_diagnostic_capabilities();
    let gpu_driver_versions = detect_gpu_driver_versions(&mut probe_warnings);
    DiagnosticBuildInfo {
        app_name: "Silk".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        build_commit: option_env!("SILK_BUILD_COMMIT").map(str::to_string),
        build_date: option_env!("SILK_BUILD_DATE").map(str::to_string),
        rust_version: option_env!("SILK_RUST_VERSION").map(str::to_string),
        target: option_env!("SILK_BUILD_TARGET")
            .unwrap_or("unknown")
            .to_string(),
        operating_system: std::env::consts::OS.to_string(),
        os_version: detect_os_version(),
        architecture: std::env::consts::ARCH.to_string(),
        graphics_adapters,
        gpu_driver_versions,
        wgc_supported,
        encoder_backends,
        fidelity_capabilities: app_controller::pipeline_fidelity_capabilities(),
        ffmpeg_available: command_available("ffmpeg"),
        ffprobe_available: command_available("ffprobe"),
        probe_warnings: {
            if !cfg!(windows) {
                probe_warnings
                    .push("Windows capability probes were skipped on this target".to_string());
            }
            probe_warnings
        },
    }
}

#[cfg(windows)]
fn native_diagnostic_capabilities() -> (
    Vec<DiagnosticGraphicsAdapter>,
    bool,
    Vec<DiagnosticEncoderBackend>,
    Vec<String>,
) {
    let mut warnings = Vec::new();
    let graphics_adapters = match capture_windows::enumerate_adapters() {
        Ok(adapters) => adapters
            .into_iter()
            .map(|adapter| DiagnosticGraphicsAdapter {
                name: adapter.name,
                vendor_id: adapter.vendor_id,
            })
            .collect(),
        Err(error) => {
            warnings.push(format!("DXGI adapter probe failed: {error}"));
            Vec::new()
        }
    };
    let wgc_supported = capture_windows::is_wgc_supported();
    let encoder_backends = match encoder_media_foundation::MediaFoundationFactory::new().discover()
    {
        Ok(candidates) => candidates
            .into_iter()
            .map(|candidate| DiagnosticEncoderBackend {
                backend_name: candidate.capabilities.backend_name,
                codec: candidate.capabilities.codec,
                hardware_accelerated: candidate.capabilities.hardware_accelerated,
                max_width: candidate.capabilities.max_width,
                max_height: candidate.capabilities.max_height,
                supported_fps: candidate.capabilities.supported_fps,
                supported_pixel_formats: candidate.capabilities.supported_pixel_formats,
            })
            .collect(),
        Err(error) => {
            warnings.push(format!("Media Foundation encoder probe failed: {error}"));
            Vec::new()
        }
    };
    (graphics_adapters, wgc_supported, encoder_backends, warnings)
}

#[cfg(not(windows))]
fn native_diagnostic_capabilities() -> (
    Vec<DiagnosticGraphicsAdapter>,
    bool,
    Vec<DiagnosticEncoderBackend>,
    Vec<String>,
) {
    (Vec::new(), false, Vec::new(), Vec::new())
}

#[cfg(windows)]
fn silent_process_command(program: &str) -> ProcessCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut command = ProcessCommand::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn silent_process_command(program: &str) -> ProcessCommand {
    ProcessCommand::new(program)
}

fn command_available(command: &str) -> bool {
    silent_process_command(command)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn detect_os_version() -> Option<String> {
    let output = silent_process_command("cmd")
        .args(["/C", "ver"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(128).collect())
}

#[cfg(windows)]
fn detect_gpu_driver_versions(warnings: &mut Vec<String>) -> Vec<String> {
    let output = match silent_process_command("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ProgressPreference='SilentlyContinue'; Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion | ConvertTo-Json -Compress",
        ])
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            warnings.push(format!(
                "GPU driver probe failed with exit code {:?}",
                output.status.code()
            ));
            return Vec::new();
        }
        Err(error) => {
            warnings.push(format!("GPU driver probe could not start: {error}"));
            return Vec::new();
        }
    };
    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("GPU driver probe returned invalid JSON: {error}"));
            return Vec::new();
        }
    };
    let entries = match value {
        Value::Array(entries) => entries,
        Value::Object(entry) => vec![Value::Object(entry)],
        _ => Vec::new(),
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.get("Name").and_then(Value::as_str)?;
            let version = entry.get("DriverVersion").and_then(Value::as_str)?;
            Some(format!("{name}: {version}"))
        })
        .collect()
}

#[cfg(not(windows))]
fn detect_gpu_driver_versions(_warnings: &mut Vec<String>) -> Vec<String> {
    Vec::new()
}

#[cfg(not(windows))]
fn detect_os_version() -> Option<String> {
    None
}

fn diagnostic_redaction_patterns(settings: &AppConfig) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut add = |value: Option<&str>| {
        if let Some(value) = value.filter(|value| value.chars().count() > 4) {
            if !patterns.iter().any(|pattern| pattern == value) {
                patterns.push(value.to_string());
            }
        }
    };
    add(Some(&settings.output.directory));
    add(Some(&settings.capture.source_id));
    for track in &settings.audio.tracks {
        add(track.device_id.as_deref());
    }
    for variable in ["USERPROFILE", "TEMP", "TMP", "APPDATA", "LOCALAPPDATA"] {
        add(std::env::var_os(variable)
            .as_deref()
            .and_then(|value| value.to_str()));
    }
    patterns
}

/// Build and atomically write a user-requested diagnostic package under the
/// application log directory. The package never contains clip payloads.
pub fn export_diagnostics(settings: &AppConfig, log_directory: &Path) -> Result<PathBuf, String> {
    write_diagnostic_package(settings, log_directory, diagnostic_build_info())
}

fn write_diagnostic_package(
    settings: &AppConfig,
    log_directory: &Path,
    build: DiagnosticBuildInfo,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(log_directory).map_err(|error| {
        format!("[DIAGNOSTIC_EXPORT_FAILED] could not create log directory: {error}")
    })?;
    let patterns = diagnostic_redaction_patterns(settings);
    let redactor = diagnostics::redaction::Redactor::new(patterns.clone());
    let settings = serde_json::to_value(settings)
        .map(|value| redactor.redact_value(&value))
        .map_err(|error| {
            format!("[DIAGNOSTIC_EXPORT_FAILED] could not serialize settings: {error}")
        })?;
    let recent_logs = diagnostics::recent_records(
        &log_directory.join("silk.jsonl"),
        diagnostics::DEFAULT_EXPORT_LOG_LINES,
        diagnostics::DEFAULT_MAX_ROTATED_FILES,
        &patterns,
    );
    let recent_errors = recent_logs
        .iter()
        .filter(|record| {
            matches!(
                record.get("level").and_then(Value::as_str),
                Some("WARN" | "ERROR")
            )
        })
        .cloned()
        .collect();
    let package = DiagnosticPackage {
        schema_version: 1,
        generated_at: Utc::now().to_rfc3339(),
        media_included: false,
        build,
        settings,
        recent_logs,
        recent_errors,
    };
    let package = redactor.redact_value(&serde_json::to_value(package).map_err(|error| {
        format!("[DIAGNOSTIC_EXPORT_FAILED] could not serialize package: {error}")
    })?);
    let bytes = serde_json::to_vec_pretty(&package)
        .map_err(|error| format!("[DIAGNOSTIC_EXPORT_FAILED] could not encode package: {error}"))?;
    if bytes.len() > MAX_DIAGNOSTIC_PACKAGE_BYTES {
        return Err(format!(
            "[DIAGNOSTIC_EXPORT_FAILED] package exceeds the {} MiB bound",
            MAX_DIAGNOSTIC_PACKAGE_BYTES / (1024 * 1024)
        ));
    }
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S-%3f");
    let base_name = format!("silk-diagnostics-{timestamp}");

    for suffix in 0..100_u32 {
        let name = if suffix == 0 {
            format!("{base_name}.json")
        } else {
            format!("{base_name}-{suffix}.json")
        };
        let path = log_directory.join(name);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "[DIAGNOSTIC_EXPORT_FAILED] could not create package: {error}"
                ))
            }
        };
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "[DIAGNOSTIC_EXPORT_FAILED] could not write package: {error}"
            ));
        }
        return Ok(path);
    }

    Err("[DIAGNOSTIC_EXPORT_FAILED] could not allocate a unique package name".to_string())
}

#[cfg(all(test, windows))]
mod runtime_media_tests {
    use super::{native_backend_requested, RuntimeMediaMode};

    #[test]
    fn native_mode_forces_each_production_backend() {
        for variable in [
            "SILK_REAL_CAPTURE",
            "SILK_REAL_ENCODER",
            "SILK_REAL_AUDIO",
            "SILK_REAL_MIC",
        ] {
            assert!(native_backend_requested(RuntimeMediaMode::Native, variable));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseAction {
    Hide,
    Exit,
}

pub(crate) fn close_action(minimize_to_tray: bool, exit_requested: bool) -> CloseAction {
    if minimize_to_tray && !exit_requested {
        CloseAction::Hide
    } else {
        CloseAction::Exit
    }
}

/// Abstraction over "send this event to the UI". Production impl wraps the
/// Tauri AppHandle; tests collect into a Vec.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &ControllerEvent);
}

/// Round-trip probe for the IPC bridge (plan §4.4 item 10).
pub fn ping_message(message: &str) -> String {
    format!("pong:{message}")
}

/// Enumerate active desktop displays for capture settings.
pub fn display_sources() -> Result<Vec<VideoSourceInfo>, String> {
    #[cfg(windows)]
    {
        capture_windows::enumerate_displays().map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(vec![VideoSourceInfo {
            id: "display-1".to_string(),
            name: "Display 1 (Primary - 1920x1080)".to_string(),
            is_primary: true,
            width: 1_920,
            height: 1_080,
        }])
    }
}

/// Enumerate top-level application windows for window capture settings.
pub fn window_sources() -> Result<Vec<WindowSourceInfo>, String> {
    #[cfg(windows)]
    {
        capture_windows::enumerate_windows().map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

/// Enumerate active endpoints for the audio-track settings view. Enumeration
/// is diagnostic/configuration work and never runs on a media worker.
pub fn audio_devices() -> Result<Vec<AudioDeviceInfo>, String> {
    #[cfg(windows)]
    {
        let mut devices = Vec::new();
        for (kind, label) in [
            (audio_wasapi::WasapiCaptureKind::Loopback, "desktop output"),
            (audio_wasapi::WasapiCaptureKind::Microphone, "input device"),
        ] {
            match audio_wasapi::enumerate_devices(kind) {
                Ok(mut found) => devices.append(&mut found),
                Err(error) => diagnostics::warn(
                    "audio",
                    &format!("could not enumerate {label} endpoints: {error}"),
                ),
            }
        }
        Ok(devices)
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

/// Build the desktop controller. Scripted audio remains the default for
/// deterministic shell tests; the configured application path uses native
/// Windows media backends. The `SILK_REAL_*` flags remain available to the
/// environment-controlled shell smoke tests below.
#[allow(dead_code)]
pub fn build_controller(output_dir: PathBuf) -> Controller {
    let mut settings = ControllerSettings {
        output_dir,
        ..ControllerSettings::default()
    };
    // This helper is used by the deterministic shell lifecycle test. The
    // product path uses `build_controller_from_config`, where persisted track
    // enablement is authoritative.
    if let Some(microphone) = settings
        .audio_tracks
        .iter_mut()
        .find(|track| track.id == "microphone")
    {
        microphone.enabled = true;
    }
    build_controller_with_settings(settings)
}

/// Build the controller using persisted application settings. Production must
/// not silently publish deterministic mock packets as user clips, so the
/// configured path selects native Windows capture, encoding, and audio.
pub fn build_controller_from_config(settings: &AppConfig) -> Controller {
    build_controller_with_mode(
        controller_settings_from_config(settings),
        RuntimeMediaMode::Native,
    )
}

pub(crate) fn controller_settings_from_config(settings: &AppConfig) -> ControllerSettings {
    let (width, height, follow_source_dimensions) =
        match settings.capture.output_resolution.fixed_size() {
            Some((width, height)) => (width, height, false),
            // Dimensions are resolved from capture source metadata before encoder
            // validation. Zeroes make unresolved Native intent impossible to
            // mistake for a real 1920x1080 output.
            None => (0, 0, true),
        };
    let preset_level = match settings.encoding.quality_preset {
        configuration::QualityPreset::Low => 0,
        configuration::QualityPreset::Medium => 1,
        configuration::QualityPreset::High => 2,
    };
    ControllerSettings {
        output_dir: managed_clip_directory(Path::new(&settings.output.directory)),
        file_name_pattern: settings.output.file_name_pattern.clone(),
        organize_by_game: true,
        source_name: settings.capture.source_id.clone(),
        retention_ms: i64::from(settings.replay.duration_seconds).saturating_mul(1_000),
        video_encoder_config: encoder_api::VideoEncoderConfig {
            codec: settings.encoding.video_codec.clone(),
            width,
            height,
            fps: settings.capture.frame_rate,
            bitrate_kbps: settings.encoding.video_bitrate_kbps,
            preset_level,
            keyframe_interval_seconds: settings.encoding.keyframe_interval_seconds,
            pixel_format: media_types::PixelFormat::Nv12,
        },
        follow_source_dimensions,
        encoder_preference: match settings.encoding.encoder {
            configuration::EncoderSelection::Auto => encoder_api::EncoderPreference::Auto,
            configuration::EncoderSelection::Hardware => {
                encoder_api::EncoderPreference::HardwareOnly
            }
            configuration::EncoderSelection::Software => {
                encoder_api::EncoderPreference::SoftwareOnly
            }
        },
        audio_tracks: settings.audio.tracks.clone(),
        audio_bitrate_kbps: settings.encoding.audio_bitrate_kbps,
        output_options: muxer::SaveOptions::default(),
        minimum_free_space_bytes: ControllerSettings::default().minimum_free_space_bytes,
        fidelity_mode: settings.encoding.fidelity_mode,
    }
}

pub(crate) fn build_controller_with_settings(
    controller_settings: ControllerSettings,
) -> Controller {
    build_controller_with_mode(controller_settings, RuntimeMediaMode::Environment)
}

fn build_controller_with_mode(
    controller_settings: ControllerSettings,
    runtime_media_mode: RuntimeMediaMode,
) -> Controller {
    let scripted_frame_count = u32::try_from(
        controller_settings
            .retention_ms
            .max(400)
            .saturating_div(33)
            .saturating_add(120),
    )
    .unwrap_or(u32::MAX);
    let scripted_width = controller_settings.video_encoder_config.width;
    let scripted_height = controller_settings.video_encoder_config.height;
    let encoder_preference = controller_settings.encoder_preference;
    let sink: Arc<dyn NotificationSink> = Arc::new(ConsoleNotificationSink);
    let audio_factory: AudioDepsFactory = Box::new(move |plans| {
        #[cfg(windows)]
        let use_real_audio = native_backend_requested(runtime_media_mode, "SILK_REAL_AUDIO");
        #[cfg(windows)]
        let use_real_microphone = native_backend_requested(runtime_media_mode, "SILK_REAL_MIC");
        plans
            .iter()
            .enumerate()
            .map(|(index, plan)| {
                let (capture, use_native_aac): (Box<dyn audio_api::AudioCapture>, bool) = match plan
                    .source_kind
                {
                    AudioSourceKind::OutputLoopback => {
                        #[cfg(windows)]
                        if use_real_audio {
                            (Box::new(WasapiAudioCapture::loopback()), true)
                        } else {
                            (
                                Box::new(ScriptedAudioCapture::new(vec![AudioStep::Chunks(120)])),
                                false,
                            )
                        }
                        #[cfg(not(windows))]
                        {
                            (
                                Box::new(ScriptedAudioCapture::new(vec![AudioStep::Chunks(120)])),
                                false,
                            )
                        }
                    }
                    AudioSourceKind::Input => {
                        #[cfg(windows)]
                        if use_real_microphone {
                            (Box::new(WasapiAudioCapture::microphone()), true)
                        } else {
                            (
                                Box::new(ScriptedAudioCapture::new(vec![AudioStep::Chunks(120)])),
                                false,
                            )
                        }
                        #[cfg(not(windows))]
                        {
                            (
                                Box::new(ScriptedAudioCapture::new(vec![AudioStep::Chunks(120)])),
                                false,
                            )
                        }
                    }
                };
                let encoder: Box<dyn encoder_api::AudioEncoder> = if use_native_aac {
                    encoder_media_foundation::new_aac_encoder()
                } else {
                    Box::new(MockAudioEncoder::default())
                };
                let mut input = AudioInput::new(
                    StreamId(u32::try_from(index + 1).unwrap_or(u32::MAX)),
                    plan.name.clone(),
                    capture,
                    encoder,
                )
                .with_gain(plan.gain);
                input.capture_config.device_id = plan.device_id.clone();
                input
            })
            .collect()
    });

    let video_factory: DepsFactory = Box::new(move || {
        #[cfg(windows)]
        let use_real_capture = native_backend_requested(runtime_media_mode, "SILK_REAL_CAPTURE");
        #[cfg(windows)]
        let use_real_encoder = native_backend_requested(runtime_media_mode, "SILK_REAL_ENCODER");

        #[cfg(windows)]
        let video: Box<dyn capture_api::VideoCapture> = if use_real_capture {
            Box::new(WgcDisplayCapture::new(StreamId(0)))
        } else if use_real_encoder {
            Box::new(
                ScriptedVideoCapture::new(vec![VideoStep::Frames(scripted_frame_count)])
                    .with_dimensions(scripted_width, scripted_height)
                    .with_cpu_frames()
                    .with_continuous_frames(),
            )
        } else {
            Box::new(
                ScriptedVideoCapture::new(vec![VideoStep::Frames(scripted_frame_count)])
                    .with_dimensions(scripted_width, scripted_height),
            )
        };

        #[cfg(not(windows))]
        let video: Box<dyn capture_api::VideoCapture> = Box::new(
            ScriptedVideoCapture::new(vec![VideoStep::Frames(scripted_frame_count)])
                .with_dimensions(scripted_width, scripted_height),
        );

        #[cfg(windows)]
        let video_encoder: Box<dyn encoder_api::VideoEncoder> = if use_real_encoder {
            Box::new(encoder_media_foundation::new_fallback_encoder(
                encoder_preference,
            ))
        } else {
            Box::new(MockVideoEncoder::new())
        };

        #[cfg(not(windows))]
        let video_encoder: Box<dyn encoder_api::VideoEncoder> = Box::new(MockVideoEncoder::new());

        (
            video,
            video_encoder,
            Box::new(Mp4Muxer) as Box<dyn muxer::Muxer>,
        )
    });
    Controller::with_audio_factory(video_factory, audio_factory, sink, controller_settings)
}

/// Load the versioned application document used by the desktop process.
/// Missing files use defaults; invalid files are reported and left untouched
/// so a later settings save can replace them intentionally.
pub fn load_startup_config(path: &Path) -> AppConfig {
    match configuration::load(path) {
        Ok(loaded) => {
            for warning in loaded.warnings {
                diagnostics::warn("configuration", &warning);
            }
            if let Some(version) = loaded.migrated_from {
                if let Err(error) = configuration::save(path, &loaded.settings) {
                    diagnostics::warn(
                        "configuration",
                        &format!(
                            "configuration version {version} was loaded but could not be rewritten: {error}"
                        ),
                    );
                } else {
                    diagnostics::info(
                        "configuration",
                        &format!("migrated configuration from version {version}"),
                    );
                }
            }
            loaded.settings
        }
        Err(error) => {
            diagnostics::error(
                "configuration",
                &format!("could not load configuration; using defaults: {error}"),
            );
            AppConfig::default()
        }
    }
}

/// Register configured global chords. Invalid settings are reported without
/// preventing the recorder from starting.
pub fn build_hotkeys_for(settings: &configuration::HotkeysSettings) -> Option<HotkeyManager> {
    #[cfg(windows)]
    {
        let bindings = match hotkey_bindings(settings) {
            Ok(bindings) => bindings,
            Err(error) => {
                diagnostics::error("hotkeys", &error.to_string());
                return None;
            }
        };
        match HotkeyManager::new(bindings) {
            Ok(manager) => Some(manager),
            Err(error) => {
                diagnostics::warn("hotkeys", &error.to_string());
                None
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = settings;
        None
    }
}

/// Convert validated configuration into the stable binding IDs used by the
/// native registration loop. This remains platform-neutral for deterministic
/// settings tests.
pub fn hotkey_bindings(settings: &HotkeysSettings) -> Result<Vec<HotkeyBinding>, HotkeyError> {
    let mut bindings = vec![HotkeyBinding::new(
        SAVE_REPLAY_HOTKEY_ID,
        HotkeyAction::SaveReplay,
        &settings.save_replay,
    )?];
    if let Some(chord) = settings.start_capture.as_deref() {
        bindings.push(HotkeyBinding::new(
            START_CAPTURE_HOTKEY_ID,
            HotkeyAction::StartCapture,
            chord,
        )?);
    }
    if let Some(chord) = settings.stop_capture.as_deref() {
        bindings.push(HotkeyBinding::new(
            STOP_CAPTURE_HOTKEY_ID,
            HotkeyAction::StopCapture,
            chord,
        )?);
    }
    validate_bindings(&bindings)?;
    Ok(bindings)
}

/// Return the currently loaded hotkey settings for the settings view.
pub fn hotkey_settings(state: &ShellState) -> Result<HotkeysSettings, String> {
    state
        .settings
        .lock()
        .map(|settings| settings.hotkeys.clone())
        .map_err(|error| format!("settings lock poisoned: {error}"))
}

/// Return the full settings document currently owned by the shell.
pub fn app_settings(state: &ShellState) -> Result<AppConfig, String> {
    state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .map_err(|error| format!("settings lock poisoned: {error}"))
}

/// Validate, apply, and atomically persist all settings currently exposed by
/// the S7 settings view. Runtime state is rolled back if persistence fails.
pub fn apply_app_settings(state: &ShellState, settings: &AppConfig) -> Result<(), String> {
    let bindings = hotkey_bindings(&settings.hotkeys).map_err(|error| error.to_string())?;
    let mut stored_settings = state
        .settings
        .lock()
        .map_err(|error| format!("settings lock poisoned: {error}"))?;
    let previous_hotkeys = stored_settings.hotkeys.clone();
    let previous_settings = stored_settings.clone();

    let issues = configuration::validate_settings(settings);
    if !issues.is_empty() {
        return Err(configuration::ConfigError::Validation(issues).to_string());
    }
    let managed_output_dir = managed_clip_directory(Path::new(&settings.output.directory));
    std::fs::create_dir_all(&managed_output_dir).map_err(|error| {
        format!(
            "could not prepare managed clip directory {}: {error}",
            managed_output_dir.display()
        )
    })?;

    let previous_bindings =
        hotkey_bindings(&previous_hotkeys).map_err(|error| error.to_string())?;
    let created_manager = apply_bindings(state, bindings)?;
    let previous_controller = controller_settings_from_config(&previous_settings);
    let next_controller = controller_settings_from_config(settings);

    if let Err(error) = state
        .controller
        .lock()
        .map_err(|lock_error| format!("controller lock poisoned: {lock_error}"))?
        .update_settings(next_controller.clone())
    {
        let rollback = restore_hotkeys(state, previous_bindings, created_manager);
        return Err(with_rollback(
            "could not apply recorder settings",
            error,
            rollback,
        ));
    }

    let library_changed = (|| {
        library_set_output_directory(
            state,
            managed_clip_directory(Path::new(&settings.output.directory)),
        )?;
        library_set_storage_policy(state, storage_policy_from_config(settings)).map(|_| ())
    })();
    if let Err(error) = library_changed {
        let rollback = rollback_runtime(
            state,
            &previous_settings,
            previous_controller,
            true,
            created_manager,
            previous_bindings,
        );
        return Err(with_rollback(
            "could not apply library settings",
            error,
            rollback,
        ));
    }

    if let Err(error) = configuration::save(&state.config_path, settings) {
        let rollback = rollback_runtime(
            state,
            &previous_settings,
            previous_controller,
            true,
            created_manager,
            previous_bindings,
        );
        return Err(with_rollback(
            "could not persist application settings",
            error.to_string(),
            rollback,
        ));
    }

    state
        .minimize_to_tray
        .store(settings.application.minimize_to_tray, Ordering::Release);
    state.notifications_enabled.store(
        settings.application.notifications_enabled,
        Ordering::Release,
    );
    state
        .clip_sound_enabled
        .store(settings.application.clip_sound_enabled, Ordering::Release);
    set_diagnostic_level(settings.diagnostics.logging_level);
    if let Err(error) = apply_startup_registration(settings.application.start_with_windows) {
        diagnostics::warn("startup", &error);
    }
    *stored_settings = settings.clone();
    Ok(())
}

/// Validate and submit a live binding replacement to the dedicated hotkey
/// thread, then persist the full versioned application document atomically.
pub fn apply_hotkey_settings(state: &ShellState, settings: &HotkeysSettings) -> Result<(), String> {
    let mut next = app_settings(state)?;
    next.hotkeys = settings.clone();
    apply_app_settings(state, &next)
}

fn apply_bindings(state: &ShellState, bindings: Vec<HotkeyBinding>) -> Result<bool, String> {
    let mut hotkey_guard = state
        .hotkeys
        .lock()
        .map_err(|error| format!("hotkey lock poisoned: {error}"))?;
    if let Some(manager) = hotkey_guard.as_ref() {
        manager
            .set_bindings(bindings)
            .map_err(|error| error.to_string())?;
        Ok(false)
    } else {
        let manager = HotkeyManager::new(bindings).map_err(|error| error.to_string())?;
        *hotkey_guard = Some(manager);
        Ok(true)
    }
}

fn restore_hotkeys(
    state: &ShellState,
    previous_bindings: Vec<HotkeyBinding>,
    created_manager: bool,
) -> Result<(), String> {
    let mut hotkey_guard = state
        .hotkeys
        .lock()
        .map_err(|error| format!("hotkey lock poisoned during rollback: {error}"))?;
    if created_manager {
        *hotkey_guard = None;
        Ok(())
    } else if let Some(manager) = hotkey_guard.as_ref() {
        manager
            .set_bindings(previous_bindings)
            .map_err(|error| error.to_string())
    } else {
        Err("global hotkey manager disappeared during rollback".to_string())
    }
}

fn library_set_output_directory(state: &ShellState, output_dir: PathBuf) -> Result<(), String> {
    let library_guard = state
        .library
        .lock()
        .map_err(|error| format!("library lock poisoned: {error}"))?;
    if let Some(library) = library_guard.as_ref() {
        library
            .set_output_directory(output_dir)
            .map_err(|error| format_library_error(&error))?;
    }
    Ok(())
}

fn library_set_storage_policy(
    state: &ShellState,
    policy: StoragePolicy,
) -> Result<StorageSummary, String> {
    let library_guard = state
        .library
        .lock()
        .map_err(|error| format!("library lock poisoned: {error}"))?;
    let Some(library) = library_guard.as_ref() else {
        return Err("[LIBRARY_WORKER_STOPPED] clip library is unavailable".to_string());
    };
    library
        .set_storage_policy(policy)
        .map_err(|error| format_library_error(&error))
}

fn rollback_runtime(
    state: &ShellState,
    previous_settings: &AppConfig,
    previous_controller: ControllerSettings,
    restore_library: bool,
    created_manager: bool,
    previous_bindings: Vec<HotkeyBinding>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if restore_library {
        if let Err(error) = library_set_output_directory(
            state,
            managed_clip_directory(Path::new(&previous_settings.output.directory)),
        ) {
            errors.push(error);
        }
        if let Err(error) =
            library_set_storage_policy(state, storage_policy_from_config(previous_settings))
        {
            errors.push(error);
        }
    }
    match state.controller.lock() {
        Ok(mut controller) => {
            if let Err(error) = controller.update_settings(previous_controller) {
                errors.push(error);
            }
        }
        Err(error) => errors.push(format!("controller lock poisoned during rollback: {error}")),
    }
    if let Err(error) = restore_hotkeys(state, previous_bindings, created_manager) {
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn with_rollback(
    action: &str,
    error: impl std::fmt::Display,
    rollback: Result<(), String>,
) -> String {
    match rollback {
        Ok(()) => format!("{action}: {error}"),
        Err(rollback_error) => {
            format!("{action}: {error}; runtime rollback failed: {rollback_error}")
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    pub state: String,
    pub buffer_duration_ms: i64,
    pub buffered_packet_count: usize,
    pub audio_stream_count: usize,
    pub save_metrics: SaveMetrics,
    pub fidelity: app_controller::FidelityStatus,
}

pub fn recorder_status(state: &ShellState) -> Result<RecorderStatus, String> {
    let controller = state
        .controller
        .lock()
        .map_err(|error| format!("controller lock poisoned: {error}"))?;
    Ok(RecorderStatus {
        state: controller.state().to_string(),
        buffer_duration_ms: controller.buffer_duration_ms(),
        buffered_packet_count: controller.buffered_packet_count(),
        audio_stream_count: controller.audio_stream_count(),
        save_metrics: controller.save_metrics(),
        fidelity: controller.fidelity_status(),
    })
}

pub fn library_scan(state: &ShellState) -> Result<Vec<ClipEntry>, String> {
    with_library(state, |library| library.scan())
}

pub fn library_rename(
    state: &ShellState,
    path: PathBuf,
    name: String,
) -> Result<ClipEntry, String> {
    with_library(state, move |library| library.rename(path, name))
}

pub fn library_delete(state: &ShellState, path: PathBuf) -> Result<(), String> {
    with_library(state, move |library| library.delete(path))
}

pub fn library_protect(
    state: &ShellState,
    path: PathBuf,
    protected: bool,
) -> Result<ClipEntry, String> {
    with_library(state, move |library| library.set_protected(path, protected))
}

pub fn library_action_path(state: &ShellState, path: PathBuf) -> Result<PathBuf, String> {
    with_library(state, move |library| library.checked_action_path(path))
}

pub fn library_storage_summary(state: &ShellState) -> Result<StorageSummary, String> {
    with_library(state, |library| library.storage_summary())
}

fn with_library<T>(
    state: &ShellState,
    operation: impl FnOnce(&LibraryWorker) -> Result<T, LibraryError>,
) -> Result<T, String> {
    let library_guard = state
        .library
        .lock()
        .map_err(|error| format!("library lock poisoned: {error}"))?;
    let Some(library) = library_guard.as_ref() else {
        return Err("[LIBRARY_WORKER_STOPPED] clip library is unavailable".to_string());
    };
    operation(library).map_err(|error| format_library_error(&error))
}

fn format_library_error(error: &LibraryError) -> String {
    format!("[{}] {error}", error.code())
}

pub(crate) fn emit_event(sink: &dyn EventSink, event: &ControllerEvent) {
    match event {
        ControllerEvent::SaveQueued { path } => {
            diagnostics::info("controller", &format!("save queued: {path}"));
        }
        ControllerEvent::ClipSaved {
            path,
            duration_ms,
            size_bytes,
        } => {
            diagnostics::info(
                "controller",
                &format!("clip saved: {path} ({duration_ms} ms, {size_bytes} bytes)"),
            );
        }
        ControllerEvent::SaveFailed { code, message } => {
            diagnostics::error("controller", &format!("save failed [{code}]: {message}"));
        }
        ControllerEvent::CommandRejected { command, reason } if command == "save_replay" => {
            diagnostics::warn(
                "controller",
                &format!("save replay command rejected: {reason}"),
            );
        }
        ControllerEvent::Warning { code, message } => {
            diagnostics::warn("controller", &format!("warning [{code}]: {message}"));
        }
        ControllerEvent::StatusChanged { state } => {
            diagnostics::info("controller", &format!("status changed: {state}"));
        }
        _ => {}
    }
    sink.emit(event);
}

/// Execute one allowlisted command and forward every produced event.
/// This is exactly what the `recorder_command` Tauri handler runs.
pub fn execute_command(
    state: &ShellState,
    sink: &dyn EventSink,
    command: &Command,
) -> Result<(), String> {
    state.observe_foreground_process();
    let attribution = state.clip_attribution();
    let hotkey_events = {
        let hotkeys_guard = state
            .hotkeys
            .lock()
            .map_err(|err| format!("hotkey lock poisoned: {err}"))?;
        hotkeys_guard
            .as_ref()
            .map(HotkeyManager::drain)
            .unwrap_or_default()
    };
    let events_to_emit = {
        let mut guard = state
            .controller
            .lock()
            .map_err(|err| format!("controller lock poisoned: {err}"))?;
        let mut events = dispatch_hotkey_events(&mut guard, hotkey_events, attribution.as_ref());
        let hotkey_already_saved = events
            .iter()
            .any(|e| matches!(e, ControllerEvent::SaveQueued { .. }));
        if !(command == &Command::SaveReplay && hotkey_already_saved) {
            events.extend(guard.execute_with_attribution(command, attribution.as_ref()));
        }
        events
    };
    for event in &events_to_emit {
        emit_event(sink, event);
    }
    Ok(())
}

pub(crate) fn dispatch_hotkey_events(
    controller: &mut Controller,
    hotkey_events: Vec<HotkeyEvent>,
    attribution: Option<&app_controller::ClipAttribution>,
) -> Vec<ControllerEvent> {
    let mut controller_events = Vec::new();
    let mut save_dispatched = false;
    for event in hotkey_events {
        match event {
            HotkeyEvent::Activated(action) => {
                if action == hotkeys::HotkeyAction::SaveReplay {
                    if save_dispatched {
                        continue;
                    }
                    save_dispatched = true;
                }
                diagnostics::info(
                    "hotkeys",
                    &format!("global hotkey activated: {}", action.as_str()),
                );
                controller_events.extend(
                    controller.execute_with_attribution(&command_for_hotkey(action), attribution),
                );
            }
            HotkeyEvent::RegistrationFailed {
                action,
                chord,
                os_error,
            } => controller_events.push(ControllerEvent::Warning {
                code: "HOTKEY_REGISTRATION_FAILED".to_string(),
                message: format!(
                    "could not register {} hotkey '{}' (Windows error {})",
                    action.as_str(),
                    chord,
                    os_error
                ),
            }),
            HotkeyEvent::EventQueueFull => controller_events.push(ControllerEvent::Warning {
                code: "HOTKEY_EVENT_QUEUE_FULL".to_string(),
                message: "global hotkey events were dropped because the queue is full".to_string(),
            }),
            HotkeyEvent::Stopped => {}
        }
    }
    controller_events
}

fn command_for_hotkey(action: HotkeyAction) -> Command {
    match action {
        HotkeyAction::SaveReplay => Command::SaveReplay,
        HotkeyAction::StartCapture => Command::StartCapture,
        HotkeyAction::StopCapture => Command::StopCapture,
    }
}

/// Buffered packet count, exposed for smoke checks and tests.
#[allow(dead_code)] // consumed by tests and future diagnostics surface
pub fn buffered_packet_count(state: &ShellState) -> usize {
    state
        .controller
        .lock()
        .map(|c| c.buffered_packet_count())
        .unwrap_or(0)
}

/// Number of audio sources that have produced synchronized output.
#[allow(dead_code)] // consumed by shell tests and the future status surface
pub fn audio_stream_count(state: &ShellState) -> usize {
    state
        .controller
        .lock()
        .map(|controller| controller.audio_stream_count())
        .unwrap_or(0)
}

/// Recorder state label, for smoke checks and tests.
#[allow(dead_code)] // consumed by tests and the future status timer
pub fn state(state: &ShellState) -> String {
    state
        .controller
        .lock()
        .map(|c| c.state().to_string())
        .unwrap_or_else(|_| "Stopped".to_string())
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    fn test_build_info() -> DiagnosticBuildInfo {
        DiagnosticBuildInfo {
            app_name: "Silk".to_string(),
            app_version: "test".to_string(),
            build_commit: None,
            build_date: None,
            rust_version: None,
            target: "test-target".to_string(),
            operating_system: "test-os".to_string(),
            os_version: None,
            architecture: "test-arch".to_string(),
            graphics_adapters: Vec::new(),
            gpu_driver_versions: Vec::new(),
            wgc_supported: false,
            encoder_backends: Vec::new(),
            fidelity_capabilities: app_controller::pipeline_fidelity_capabilities(),
            ffmpeg_available: false,
            ffprobe_available: false,
            probe_warnings: Vec::new(),
        }
    }

    #[test]
    fn diagnostic_package_excludes_media_and_redacts_user_values() {
        let directory = std::env::temp_dir().join(format!(
            "silk-diagnostic-export-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create test directory");
        let user_path = r"C:\Users\bob\Videos\Silk";
        std::fs::write(
            directory.join("silk.jsonl"),
            format!(
                "{{\"level\":\"ERROR\",\"component\":\"test\",\"message\":\"failed at {user_path}\\\\clip.mp4\"}}\n"
            ),
        )
        .expect("write test log");
        let mut settings = AppConfig::default();
        settings.output.directory = user_path.to_string();

        let path = write_diagnostic_package(&settings, &directory, test_build_info())
            .expect("export package");
        let output = std::fs::read_to_string(path).expect("read package");
        assert!(!output.contains(user_path));
        assert!(output.contains("\"mediaIncluded\": false"));
        assert!(output.contains("\"recentErrors\""));

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn diagnostic_build_info_uses_stable_wire_names() {
        let value = serde_json::to_value(test_build_info()).expect("serialize build info");
        assert_eq!(value["appVersion"], "test");
        assert_eq!(value["wgcSupported"], false);
        assert!(value.get("gpuDriverVersions").is_some());
        assert!(value.get("fidelityCapabilities").is_some());
        let caps = value["fidelityCapabilities"]
            .as_array()
            .expect("capabilities array");
        assert_eq!(caps.len(), 3);
        assert_eq!(caps[0]["mode"], "standard");
        assert_eq!(caps[0]["availability"], "available");
        assert_eq!(caps[1]["mode"], "clarity");
        assert_eq!(caps[1]["availability"], "runtime_checked");
        assert_eq!(caps[2]["mode"], "archival_444");
        assert_eq!(caps[2]["availability"], "unsupported");
    }
}
