#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Silk desktop shell with the native Windows capture and encoding path.
//!
//! The UI owns no media processing: it sends allowlisted commands and
//! receives plain-data events. All handler logic lives in `shell`, which
//! is unit-tested without launching a window.

mod file_drag;
mod hud;
mod sharing;
mod shell;
mod sound;
mod theme_icon;
mod tray;
mod updates;
mod window_presentation;

#[cfg(test)]
mod shell_tests;

use std::path::{Path, PathBuf};

use app_controller::{Command, ControllerEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

// Windows toasts are separate top-level visual feedback and require their own
// in-game presentation test before they can be re-enabled.
const WINDOWS_TOAST_VALIDATED: bool = false;

pub(crate) struct TauriEventSink<R: Runtime>(AppHandle<R>);

impl<R: Runtime> shell::EventSink for TauriEventSink<R> {
    fn emit(&self, event: &ControllerEvent) {
        if let Some(hud) = self.0.try_state::<hud::HudRuntime>() {
            hud.try_dispatch_event(event);
        }
        let _ = self.0.emit("controller-event", event);
        tray::update_from_event(&self.0, event);

        let (notifications_enabled, clip_sound_enabled, clip_sound_volume) = self
            .0
            .try_state::<shell::ShellState>()
            .map(|state| {
                (
                    state.notifications_enabled(),
                    state.clip_sound_enabled(),
                    state.clip_sound_volume(),
                )
            })
            .unwrap_or((true, true, 100));

        if sound::should_play_clip_sound(event, clip_sound_enabled) {
            let _ = sound::play_clip_saved_sound(clip_sound_volume);
        }

        notify_from_event(&self.0, event, notifications_enabled);
    }
}

fn notify_from_event<R: Runtime>(
    app: &AppHandle<R>,
    event: &ControllerEvent,
    notifications_enabled: bool,
) {
    // A toast is another visible top-level window during gameplay. Keep it
    // suspended along with the native HUD until the active-cue gate passes.
    if !should_send_visual_notification(notifications_enabled) {
        return;
    }
    let Some((title, body)) = notification_content(event) else {
        return;
    };

    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        diagnostics::warn(
            "notifications",
            &format!("desktop notification failed: {error}"),
        );
    }
}

fn should_send_visual_notification(notifications_enabled: bool) -> bool {
    WINDOWS_TOAST_VALIDATED && notifications_enabled
}

pub(crate) fn notification_content(event: &ControllerEvent) -> Option<(&'static str, String)> {
    match event {
        ControllerEvent::ClipSaved {
            path,
            duration_ms,
            size_bytes,
        } => Some((
            "Replay saved",
            format!("{path} ({duration_ms} ms, {size_bytes} bytes)"),
        )),
        ControllerEvent::SaveFailed { code, message } => {
            Some(("Replay save failed", format!("[{code}] {message}")))
        }
        ControllerEvent::CommandRejected { command, reason } if command == "save_replay" => {
            Some(("Replay save failed", reason.clone()))
        }
        _ => None,
    }
}

#[tauri::command]
fn ping(message: String) -> String {
    shell::ping_message(&message)
}

#[tauri::command]
fn recorder_command(
    app: AppHandle,
    state: State<'_, shell::ShellState>,
    command: Command,
) -> Result<(), String> {
    shell::execute_command(&state, &TauriEventSink(app), &command)
}

#[tauri::command]
async fn pick_clip_directory(app: AppHandle) -> Result<Option<String>, String> {
    let initial_directory = shell::app_settings(&app.state::<shell::ShellState>())
        .map(|settings| PathBuf::from(settings.output.directory))?;
    let selected = app
        .dialog()
        .file()
        .set_directory(initial_directory)
        .set_title("Choose Silk clip directory")
        .blocking_pick_folder();
    selected
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|error| format!("could not read selected clip directory: {error}"))
        })
        .transpose()
}

#[tauri::command]
fn configure_hotkeys(
    state: State<'_, shell::ShellState>,
    settings: configuration::HotkeysSettings,
) -> Result<(), String> {
    shell::apply_hotkey_settings(&state, &settings)
}

#[tauri::command]
fn get_hotkey_settings(
    state: State<'_, shell::ShellState>,
) -> Result<configuration::HotkeysSettings, String> {
    shell::hotkey_settings(&state)
}

#[tauri::command]
fn get_settings(state: State<'_, shell::ShellState>) -> Result<configuration::AppConfig, String> {
    shell::app_settings(&state)
}

#[tauri::command]
fn configure_settings(
    app: AppHandle,
    state: State<'_, shell::ShellState>,
    settings: configuration::AppConfig,
) -> Result<(), String> {
    shell::apply_app_settings(&state, &settings)?;
    theme_icon::apply_theme_icon(&app, settings.application.theme);
    if let Some(hud) = app.try_state::<hud::HudRuntime>() {
        hud.reconfigure(settings.overlay);
    }
    Ok(())
}

#[tauri::command]
fn update_theme_icon(
    app: AppHandle,
    state: State<'_, shell::ShellState>,
    theme: String,
) -> Result<(), String> {
    // Keep the IPC payload compatible, but never let a late frontend effect
    // overwrite the native icon with an older theme after a newer settings save.
    let _ = theme;
    let t = shell::app_settings(&state)?.application.theme;
    theme_icon::apply_theme_icon(&app, t);

    if let Some(window) = app.get_webview_window("main") {
        let (r, g, b) = match t {
            configuration::Theme::Studio => (9, 9, 12),
            configuration::Theme::Classic => (14, 15, 11),
            configuration::Theme::Ember => (28, 18, 12),
            configuration::Theme::Vamp => (14, 14, 17),
        };
        let _ = window.set_background_color(Some(tauri::window::Color(r, g, b, 255)));
    }

    Ok(())
}

#[tauri::command]
fn get_display_sources() -> Result<Vec<media_types::VideoSourceInfo>, String> {
    shell::display_sources()
}

#[tauri::command]
fn get_window_sources() -> Result<Vec<media_types::WindowSourceInfo>, String> {
    shell::window_sources()
}

#[tauri::command]
fn get_audio_devices() -> Result<Vec<media_types::AudioDeviceInfo>, String> {
    shell::audio_devices()
}

#[tauri::command]
fn get_recorder_status(
    state: State<'_, shell::ShellState>,
) -> Result<shell::RecorderStatus, String> {
    shell::recorder_status(&state)
}

#[tauri::command]
fn list_clips(state: State<'_, shell::ShellState>) -> Result<Vec<clip_library::ClipEntry>, String> {
    shell::library_scan(&state)
}

#[tauri::command]
fn rename_clip(
    state: State<'_, shell::ShellState>,
    path: PathBuf,
    name: String,
) -> Result<clip_library::ClipEntry, String> {
    shell::library_rename(&state, path, name)
}

#[tauri::command]
fn delete_clip(state: State<'_, shell::ShellState>, path: PathBuf) -> Result<(), String> {
    shell::library_delete(&state, path)
}

#[tauri::command]
fn reveal_clip(state: State<'_, shell::ShellState>, path: PathBuf) -> Result<(), String> {
    let checked_path = shell::library_action_path(&state, path)?;
    tauri_plugin_opener::reveal_item_in_dir(&checked_path)
        .map_err(|error| format!("[LIBRARY_REVEAL_FAILED] could not reveal clip: {error}"))
}

#[tauri::command]
fn open_clip(state: State<'_, shell::ShellState>, path: PathBuf) -> Result<(), String> {
    let checked_path = shell::library_action_path(&state, path)?;
    tauri_plugin_opener::open_path(&checked_path, None::<&str>)
        .map_err(|error| format!("[LIBRARY_OPEN_FAILED] could not open clip: {error}"))
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    tauri_plugin_opener::open_url(&url, None::<&str>)
        .map_err(|error| format!("could not open url: {error}"))
}

#[tauri::command]
fn set_clip_protected(
    state: State<'_, shell::ShellState>,
    path: PathBuf,
    protected: bool,
) -> Result<clip_library::ClipEntry, String> {
    shell::library_protect(&state, path, protected)
}

#[tauri::command]
fn get_storage_summary(
    state: State<'_, shell::ShellState>,
) -> Result<clip_library::StorageSummary, String> {
    shell::library_storage_summary(&state)
}

#[tauri::command]
fn get_diagnostic_info() -> shell::DiagnosticBuildInfo {
    shell::diagnostic_build_info()
}

#[tauri::command]
fn export_diagnostics(
    app: AppHandle,
    state: State<'_, shell::ShellState>,
) -> Result<PathBuf, String> {
    let settings = shell::app_settings(&state)?;
    let log_directory = app.path().app_log_dir().map_err(|error| {
        format!("[DIAGNOSTIC_EXPORT_FAILED] log directory unavailable: {error}")
    })?;
    shell::export_diagnostics(&settings, &log_directory)
}

#[tauri::command]
fn get_clip_frame_rate(path: String) -> Result<f64, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let reader = mp4::Mp4Reader::read_header(std::io::BufReader::new(file), size)
        .map_err(|e| e.to_string())?;
    Ok(reader
        .tracks()
        .values()
        .find(|track| matches!(track.track_type(), Ok(mp4::TrackType::Video)))
        .map(|track| track.frame_rate())
        .filter(|fps| fps.is_finite() && *fps > 0.0)
        .unwrap_or(60.0))
}

#[tauri::command]
fn get_clip_keyframes(path: String) -> Result<Vec<f64>, String> {
    let file = std::fs::File::open(&path).map_err(|e| format!("Failed to open clip: {e}"))?;
    let size = file
        .metadata()
        .map_err(|e| format!("Failed to read clip metadata: {e}"))?
        .len();
    let reader = mp4::Mp4Reader::read_header(std::io::BufReader::new(file), size)
        .map_err(|e| format!("Failed to read MP4 header: {e}"))?;

    for track in reader.tracks().values() {
        if matches!(track.track_type(), Ok(mp4::TrackType::Video)) {
            return Ok(track.keyframe_timestamps_seconds());
        }
    }
    Ok(Vec::new())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipAudioTrackInfo {
    pub track_id: u32,
    pub name: String,
    pub channels: u16,
    pub sample_rate: u32,
}

#[tauri::command]
fn get_clip_audio_tracks(path: String) -> Result<Vec<ClipAudioTrackInfo>, String> {
    let file = std::fs::File::open(&path).map_err(|e| format!("Failed to open clip: {e}"))?;
    let size = file
        .metadata()
        .map_err(|e| format!("Failed to read clip metadata: {e}"))?
        .len();
    let reader = mp4::Mp4Reader::read_header(std::io::BufReader::new(file), size)
        .map_err(|e| format!("Failed to read MP4 header: {e}"))?;

    let mut tracks = Vec::new();
    let mut sorted_track_ids: Vec<u32> = reader.tracks().keys().copied().collect();
    sorted_track_ids.sort_unstable();

    let mut audio_num = 1_usize;
    for track_id in sorted_track_ids {
        if let Some(track) = reader.tracks().get(&track_id) {
            if matches!(track.track_type(), Ok(mp4::TrackType::Audio)) {
                let name = track.track_name().trim();
                let clean_name = if name.is_empty() {
                    format!("Audio Track {audio_num}")
                } else {
                    name.to_string()
                };
                tracks.push(ClipAudioTrackInfo {
                    track_id,
                    name: clean_name,
                    channels: track.channel_count(),
                    sample_rate: track.timescale(),
                });
                audio_num += 1;
            }
        }
    }
    Ok(tracks)
}

#[tauri::command]
async fn extract_clip_audio_track(clip_path: String, track_id: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = Path::new(&clip_path);
        if !source.exists() {
            return Err(format!("Clip not found: {clip_path}"));
        }

        let file_stem = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("track");
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(clip_path.as_bytes());
        let hash = format!("{:x}", hasher.finalize());
        let short_hash = &hash[..10];

        let temp_dir = std::env::temp_dir().join("silk-audio-cache");
        let _ = std::fs::create_dir_all(&temp_dir);
        let output_path = temp_dir.join(format!("{file_stem}_track_{track_id}_{short_hash}.m4a"));

        if output_path.exists() && output_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            return Ok(output_path.to_string_lossy().to_string());
        }

        mp4::extract_audio_track_file(source, &output_path, track_id)
            .map_err(|e| format!("Failed to extract audio track: {e}"))?;

        Ok(output_path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrimClipRequest {
    pub source_path: String,
    pub output_path: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub included_audio_tracks: Option<Vec<usize>>,
}

#[tauri::command]
async fn trim_clip(request: TrimClipRequest) -> Result<String, String> {
    if request.start_seconds < 0.0 || request.end_seconds <= request.start_seconds {
        return Err("Invalid trim range: end time must be greater than start time".to_string());
    }
    let source = Path::new(&request.source_path);
    if !source.exists() {
        return Err(format!("Source clip not found: {}", request.source_path));
    }
    let output = Path::new(&request.output_path);

    // 1. Primary path: Native pure-Rust MP4 trimming with zero external toolchain dependencies
    let native_trim_res = mp4::trim_mp4_file(
        source,
        output,
        request.start_seconds,
        request.end_seconds,
        request.included_audio_tracks.as_deref(),
    );

    match native_trim_res {
        Ok(_) => return Ok(request.output_path),
        Err(native_err) => {
            diagnostics::warn(
                "main",
                &format!("native mp4 trim fallback triggered: {native_err}"),
            );
        }
    }

    // 2. Fallback path: External FFmpeg if present on system
    let ffmpeg_bin = std::env::var("SILK_FFMPEG_PATH")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            let output = std::process::Command::new("where.exe")
                .arg("ffmpeg.exe")
                .output()
                .ok()?;
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.lines().next().map(|line| PathBuf::from(line.trim()))
            } else {
                None
            }
        })
        .unwrap_or_else(|| PathBuf::from("ffmpeg.exe"));

    let mut cmd = std::process::Command::new(&ffmpeg_bin);
    cmd.arg("-y")
        .arg("-ss")
        .arg(format!("{:.3}", request.start_seconds))
        .arg("-to")
        .arg(format!("{:.3}", request.end_seconds))
        .arg("-i")
        .arg(&request.source_path)
        .arg("-c")
        .arg("copy");

    match request.included_audio_tracks {
        None => {
            // Keep all video and audio streams
            cmd.arg("-map").arg("0");
        }
        Some(ref tracks) if tracks.is_empty() => {
            // Video only, strip all audio
            cmd.arg("-map").arg("0:v:0");
        }
        Some(ref tracks) => {
            cmd.arg("-map").arg("0:v:0");
            for track_idx in tracks {
                // Use optional '?' so non-existent streams don't abort the command
                cmd.arg("-map").arg(format!("0:a:{}?", track_idx));
            }
        }
    }

    cmd.arg(&request.output_path);

    let output = cmd.output().map_err(|e| {
        format!(
            "Failed to execute FFmpeg for trimming: {e}. Please ensure FFmpeg is installed or set SILK_FFMPEG_PATH."
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("FFmpeg trim failed: {stderr}"));
    }

    Ok(request.output_path)
}

#[tauri::command]
fn test_clip_sound(volume: u16) -> Result<(), String> {
    sound::play_clip_saved_sound(volume)
}

#[tauri::command]
async fn test_overlay(app: AppHandle) -> Result<(), String> {
    let Some(hud) = app.try_state::<hud::HudRuntime>() else {
        return Err("native HUD runtime is unavailable".to_string());
    };
    hud.show_test_cue()
}

#[tauri::command]
fn app_window_minimize(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        win.minimize().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn app_window_toggle_maximize(app: AppHandle) -> Result<bool, String> {
    if let Some(win) = app.get_webview_window("main") {
        let is_max = win.is_maximized().unwrap_or(false);
        if is_max {
            win.unmaximize().map_err(|e| e.to_string())?;
            Ok(false)
        } else {
            win.maximize().map_err(|e| e.to_string())?;
            Ok(true)
        }
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn app_window_close(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        win.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn app_window_set_fullscreen(app: AppHandle, fullscreen: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("main window unavailable")?;
    let (send, mut receive) = tauri::async_runtime::channel(1);
    app.run_on_main_thread(move || {
        let result = window_presentation::set_fullscreen(&window, fullscreen);
        let _ = send.try_send(result);
    })
    .map_err(|error| error.to_string())?;
    receive
        .recv()
        .await
        .ok_or("fullscreen transition was interrupted")?
}
#[tauri::command]
fn app_window_is_fullscreen(app: AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_fullscreen().ok())
        .unwrap_or(false)
}

#[tauri::command]
fn app_window_is_maximized(app: AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_maximized().ok())
        .unwrap_or(false)
}

fn main() {
    // Claim the instance slot before Tauri creates the WebView window so a
    // second launch exits without flashing a duplicate app window.
    let instance_guard = match app_controller::SingleInstanceGuard::try_acquire(
        "dev.silk.project.single_instance",
    ) {
        Ok(guard) => guard,
        Err(_) => std::process::exit(0),
    };

    let context = tauri::generate_context!();
    if let Err(error) = theme_icon::initialize_taskbar_identity(&context.config().identifier) {
        eprintln!("could not initialize taskbar identity: {error}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            ping,
            updates::get_update_status,
            updates::check_for_updates,
            updates::install_update,
            sharing::prepare_clip_share,
            sharing::clip_share_status,
            sharing::cancel_clip_share,
            sharing::drag_clip_share,
            sharing::reveal_clip_share,
            recorder_command,
            pick_clip_directory,
            configure_hotkeys,
            get_hotkey_settings,
            get_settings,
            configure_settings,
            get_display_sources,
            get_window_sources,
            get_audio_devices,
            get_recorder_status,
            list_clips,
            rename_clip,
            delete_clip,
            reveal_clip,
            open_clip,
            set_clip_protected,
            get_storage_summary,
            get_diagnostic_info,
            export_diagnostics,
            test_overlay,
            test_clip_sound,
            trim_clip,
            get_clip_keyframes,
            get_clip_frame_rate,
            get_clip_audio_tracks,
            extract_clip_audio_track,
            open_url,
            app_window_minimize,
            app_window_toggle_maximize,
            app_window_close,
            app_window_is_maximized,
            app_window_set_fullscreen,
            app_window_is_fullscreen,
            update_theme_icon
        ])
        .setup(move |app| {
            let log_path = app
                .path()
                .app_log_dir()
                .ok()
                .map(|directory| directory.join("silk.jsonl"));
            diagnostics::init(diagnostics::LoggerConfig {
                file_path: log_path,
                also_stderr: cfg!(debug_assertions),
                ..diagnostics::LoggerConfig::default()
            });

            app.manage(instance_guard);
            app.manage(window_presentation::FullscreenState::default());
            app.manage(sharing::ShareRuntime::default());
            app.manage(updates::UpdateRuntime::new(app.package_info().version.to_string()));

            let config_path = match app.path().app_config_dir() {
                Ok(directory) => directory.join("config.json"),
                Err(error) => {
                    diagnostics::warn(
                        "configuration",
                        &format!(
                            "application config directory is unavailable; using temporary path: {error}"
                        ),
                    );
                    std::env::temp_dir().join("silk-config.json")
                }
            };
            let settings = shell::load_startup_config(&config_path);
            shell::set_diagnostic_level(settings.diagnostics.logging_level);
            if let Err(error) =
                shell::apply_startup_registration(settings.application.start_with_windows)
            {
                diagnostics::warn("startup", &error);
            }
            let controller = shell::build_controller_from_config(&settings);
            let hotkeys = shell::build_hotkeys_for(&settings.hotkeys);
            app.manage(shell::ShellState::with_app_config(
                controller,
                hotkeys,
                settings.clone(),
                config_path,
                settings.application.minimize_to_tray,
            ));
            sound::initialize();
            updates::start_checks(app.handle().clone());
            app.manage(hud::HudRuntime::init(settings.overlay));
            tray::setup(app)?;
            theme_icon::apply_theme_icon(app.handle(), settings.application.theme);
            // Publish native icons and Shell identity before Explorer sees the window.
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }

            if let Some(window) = app.get_webview_window("main") {
                let (r, g, b) = match settings.application.theme {
                    configuration::Theme::Studio => (9, 9, 12),
                    configuration::Theme::Classic => (14, 15, 11),
                    configuration::Theme::Ember => (28, 18, 12),
                    configuration::Theme::Vamp => (14, 14, 17),
                };
                let _ = window.set_background_color(Some(tauri::window::Color(r, g, b, 255)));
            }

            // Background pump loop for instant global hotkey dispatch and save execution
            let poll_app = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    if let Some(state) = poll_app.try_state::<shell::ShellState>() {
                        let _ = shell::execute_command(
                            &state,
                            &TauriEventSink(poll_app.clone()),
                            &Command::Poll,
                        );
                    }
                }
            });

            // Start capture automatically on launch so gameplay buffering is active immediately.
            // If the display/graphics pipeline is still settling or waking, retry with backoff.
            let start_app = app.handle().clone();
            std::thread::spawn(move || {
                for attempt in 0..5 {
                    if attempt > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(500 * attempt));
                    }
                    if let Some(state) = start_app.try_state::<shell::ShellState>() {
                        let is_running = shell::recorder_status(&state)
                            .map(|s| s.state == "Buffering" || s.state == "Ready" || s.state == "Starting" || s.state == "Saving")
                            .unwrap_or(false);
                        if is_running {
                            break;
                        }
                        let _ = shell::execute_command(
                            &state,
                            &TauriEventSink(start_app.clone()),
                            &Command::StartCapture,
                        );
                    }
                }
            });

            Ok(())
        })
        .on_window_event(tray::handle_window_event)
        .build(context)
        .expect("error while building the Silk desktop shell")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                theme_icon::cleanup_theme_icon(app);
            }
        });
}

#[cfg(test)]
mod main_tests {
    use app_controller::ControllerEvent;

    use super::{notification_content, should_send_visual_notification};

    #[test]
    fn unvalidated_visual_feedback_suppresses_windows_toasts_even_when_requested() {
        assert!(!should_send_visual_notification(true));
        assert!(!should_send_visual_notification(false));
    }

    #[test]
    fn direct_mp4_notification_content_shows_on_clip_saved() {
        assert_eq!(
            notification_content(&ControllerEvent::ClipSaved {
                path: "C:/clips/Silk/replay.mp4".to_string(),
                duration_ms: 1_250,
                size_bytes: 4_096,
            }),
            Some((
                "Replay saved",
                "C:/clips/Silk/replay.mp4 (1250 ms, 4096 bytes)".to_string()
            ))
        );
        assert_eq!(
            notification_content(&ControllerEvent::SaveFailed {
                code: "INSUFFICIENT_DISK_SPACE".to_string(),
                message: "free space is below the safety margin".to_string(),
            }),
            Some((
                "Replay save failed",
                "[INSUFFICIENT_DISK_SPACE] free space is below the safety margin".to_string()
            ))
        );
        assert_eq!(
            notification_content(&ControllerEvent::CommandRejected {
                command: "save_replay".to_string(),
                reason: "buffer not ready yet (state 'Buffering'); replay needs a primed buffer"
                    .to_string(),
            }),
            Some((
                "Replay save failed",
                "buffer not ready yet (state 'Buffering'); replay needs a primed buffer"
                    .to_string()
            ))
        );
        assert_eq!(
            notification_content(&ControllerEvent::CommandRejected {
                command: "start_capture".to_string(),
                reason: "already capturing".to_string(),
            }),
            None
        );
        assert_eq!(
            notification_content(&ControllerEvent::StatusChanged {
                state: "Ready".to_string(),
            }),
            None
        );
    }
}
