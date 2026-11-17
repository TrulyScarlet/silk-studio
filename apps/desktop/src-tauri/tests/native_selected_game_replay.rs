//! Explicit user-selected VALORANT WGC replay acceptance. Never injects or falls back.
#![cfg(windows)]
use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
use capture_windows::DisplayCapture;
use encoder_api::{EncoderPreference, VideoEncoderConfig};
use media_types::StreamId;
use recorder_engine::{EngineConfig, RecorderEngine, RecorderState};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

struct IsolatedCapture {
    inner: DisplayCapture,
    dimensions: Arc<AtomicU64>,
    expected_backend: &'static str,
    formats: Arc<AtomicU64>,
}
impl VideoCapture for IsolatedCapture {
    fn timestamp_origin(&self) -> Option<Instant> {
        self.inner.timestamp_origin()
    }
    fn active_source_id(&self) -> Option<String> {
        self.inner.active_source_id()
    }
    fn enumerate_sources(&self) -> capture_api::Result<Vec<media_types::VideoSourceInfo>> {
        self.inner.enumerate_sources()
    }
    fn start(&mut self, config: VideoCaptureConfig) -> capture_api::Result<()> {
        let waiting_for_auto = config.source_id == "game:auto";
        self.inner.start(config)?;
        if !waiting_for_auto && self.inner.backend_name() != Some(self.expected_backend) {
            self.inner.stop()?;
            return Err(capture_api::VideoCaptureError::Backend {
                details: format!(
                    "selected game did not use the explicitly requested {} backend",
                    self.expected_backend
                ),
            });
        }
        Ok(())
    }
    fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
        let event = self.inner.next_event()?;
        if matches!(event, VideoCaptureEvent::Frame(_)) {
            assert_eq!(self.inner.backend_name(), Some(self.expected_backend));
        }
        if let VideoCaptureEvent::FormatChanged { width, height } = &event {
            self.formats.fetch_add(1, Ordering::Relaxed);
            self.dimensions.store(
                (u64::from(*width) << 32) | u64::from(*height),
                Ordering::Release,
            );
        }
        Ok(event)
    }
    fn stop(&mut self) -> capture_api::Result<()> {
        self.inner.stop()
    }
    fn gpu_frame_context(&self) -> Option<encoder_api::GpuFrameContext> {
        self.inner.gpu_frame_context()
    }
}
fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("explicit {name} required"))
}
fn selected_source(guarded: bool) -> String {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    assert_eq!(
        required("SILK_GAME_REPLAY_APPROVAL"),
        if guarded {
            "valorant-guarded-only"
        } else {
            "valorant-wgc-only"
        }
    );
    let pid: u32 = required("SILK_GAME_PROBE_PID").parse().unwrap();
    let hwnd: u64 = required("SILK_GAME_PROBE_HWND").parse().unwrap();
    let created: u64 = required("SILK_GAME_PROBE_CREATION").parse().unwrap();
    assert!(pid != 0 && hwnd != 0 && created != 0);
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.unwrap();
    let _owned = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    let mut path = [0u16; 32768];
    let mut length = path.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    }
    .unwrap();
    let path = PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .eq_ignore_ascii_case("VALORANT-Win64-Shipping.exe"));
    // Production capture independently validates HWND/PID/creation time and
    // rejects capture exclusion. No foreground/desktop default exists here.
    let mode = if guarded { "game-guarded" } else { "game" };
    format!("{mode}:pid:{pid}:created:{created}:hwnd:{hwnd}")
}
fn approved_decoder() -> PathBuf {
    let path = PathBuf::from(required("SILK_GAME_REPLAY_FFMPEG"));
    let mut file = std::fs::File::open(&path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    assert_eq!(
        format!("{:X}", hash.finalize()),
        "57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A"
    );
    path
}
fn decode(decoder: &Path, clip: &Path) {
    let mut child = std::process::Command::new(decoder)
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-xerror",
            "-i",
        ])
        .arg(clip)
        .args([
            "-map",
            "0:v:0",
            "-fps_mode",
            "passthrough",
            "-enc_time_base",
            "demux",
            "-f",
            "null",
            "-",
        ])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "saved selected-game replay did not decode"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("decode deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "requires fresh explicit VALORANT selection/consent and approved decoder; non-injected WGC only"]
fn selected_valorant_wgc_hardware_replay() {
    record_selected_game(false, false, false);
}

#[test]
#[ignore = "explicit VALORANT guarded monitor-crop consent; records visible crop, not isolated WGC"]
fn selected_valorant_guarded_hardware_replay() {
    record_selected_game(true, false, false);
}

#[test]
#[ignore = "explicit selected VALORANT consent; starts unfocused, then restores/focuses game"]
fn manual_game_waits_for_first_focus() {
    record_selected_game(true, false, true);
}

#[test]
#[ignore = "explicit VALORANT consent; native automatic discovery and first-focus replay"]
fn automatic_game_waits_for_first_focus() {
    record_selected_game(true, true, true);
}

#[test]
#[ignore = "explicit VALORANT consent; automatic discovery with externally verified focus cycles"]
fn automatic_game_focus_replay() {
    record_selected_game(true, true, false);
}

fn record_selected_game(guarded: bool, automatic: bool, test_first_focus: bool) {
    let selected = selected_source(guarded);
    let source = if automatic {
        "game:auto".to_string()
    } else {
        selected.clone()
    };
    let hwnd = windows::Win32::Foundation::HWND(
        required("SILK_GAME_PROBE_HWND").parse::<usize>().unwrap() as *mut _,
    );
    struct WaitingWindow(windows::Win32::Foundation::HWND);
    impl Drop for WaitingWindow {
        fn drop(&mut self) {
            let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0) };
        }
    }
    let waiting_window = if test_first_focus {
        use windows::Win32::UI::WindowsAndMessaging::*;
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!("Silk first-focus waiting test"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                80,
                80,
                500,
                200,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let _ = unsafe { SetForegroundWindow(window) };
        Some(WaitingWindow(window))
    } else {
        None
    };
    if test_first_focus {
        assert_ne!(
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() },
            hwnd,
            "Keep a non-game window foreground at test startup"
        );
    }
    let route = if guarded {
        "guarded-window"
    } else {
        "wgc-window"
    };
    let decoder = approved_decoder();
    let output = PathBuf::from(required("SILK_GAME_PROBE_OUTPUT"))
        .canonicalize()
        .unwrap();
    let allowed = std::env::temp_dir()
        .join("opencode")
        .canonicalize()
        .unwrap();
    assert!(output.is_dir() && output.starts_with(allowed));
    diagnostics::init(diagnostics::LoggerConfig {
        file_path: Some(output.join("capture.jsonl")),
        ..Default::default()
    });
    let destination = output.join(if guarded {
        "valorant-guarded-replay.mp4"
    } else {
        "valorant-wgc-replay.mp4"
    });
    assert!(!destination.exists());
    let dimensions = Arc::new(AtomicU64::new(0));
    let formats = Arc::new(AtomicU64::new(0));
    let capture = IsolatedCapture {
        inner: DisplayCapture::new(StreamId(0)),
        dimensions: dimensions.clone(),
        expected_backend: route,
        formats: formats.clone(),
    };
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: source.clone(),
            follow_source_dimensions: true,
            retention_ms: if guarded { 25000 } else { 1000 },
            video_encoder: VideoEncoderConfig {
                width: 0,
                height: 0,
                fps: 30,
                codec: "h264".into(),
                keyframe_interval_seconds: 1.,
                ..Default::default()
            },
            ..Default::default()
        },
        Box::new(capture),
        Box::new(encoder_media_foundation::new_fallback_encoder(
            EncoderPreference::HardwareOnly,
        )),
        Box::new(muxer::Mp4Muxer),
    );
    if let Err(error) = engine.start() {
        let evidence = serde_json::json!({ "route": route, "source": source,
            "result": "replay_start_failed", "error": error.to_string(),
            "acceptance_passed": false, "injection": false, "monitor_fallback": false });
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join("replay-evidence.json"))
            .unwrap();
        serde_json::to_writer_pretty(&mut file, &evidence).unwrap();
        panic!("selected game replay acceptance failed without fallback: {error}");
    }
    if test_first_focus {
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            // Pump the owned setup window without involving the application UI.
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::*;
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            let observed = engine.pump();
            assert_ne!(
                engine.state(),
                RecorderState::Error,
                "waiting startup failed: {observed:?}"
            );
            assert_eq!(
                engine.buffered_packet_count(),
                0,
                "video buffered before first game focus"
            );
            assert_eq!(engine.buffer_duration_ms(), 0);
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::*;
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        let focus_deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } != hwnd
            && Instant::now() < focus_deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() },
            hwnd,
            "Windows denied test focus; first-focus acceptance cannot continue"
        );
        drop(waiting_window);
    }
    let started = Instant::now();
    std::fs::write(output.join("capture-started"), "started").unwrap();
    println!("CAPTURE STARTED: {route}; for guarded mode, focus VALORANT, Alt-Tab away for a few seconds, then return. Recording 25 seconds.");
    let mut events = Vec::new();
    loop {
        events.extend(engine.pump());
        assert_ne!(engine.state(), RecorderState::Error, "{events:?}");
        if started.elapsed() >= Duration::from_secs(if guarded { 25 } else { 4 })
            && engine.state() == RecorderState::Ready
            && engine.buffer_duration_ms() >= 800
            && engine.buffered_packet_count() >= 10
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(if guarded { 45 } else { 25 }),
            "selected game replay readiness deadline: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let clip = engine.save_replay(&destination).unwrap();
    if guarded && std::env::var("SILK_EXPECT_CONTINUOUS_FOCUS").as_deref() == Ok("1") {
        assert_eq!(
            formats.load(Ordering::Relaxed),
            1,
            "focus transitions changed the encoder format/epoch"
        );
        assert!(
            clip.duration_ms >= 24_000,
            "focus transitions lost replay history: {}ms",
            clip.duration_ms
        );
    }
    let metrics = engine.encoder_metrics();
    assert_eq!(metrics.cpu_readback_inputs, 0);
    assert!(metrics.gpu_surface_inputs >= 10);
    engine.stop().unwrap();
    decode(&decoder, &destination);
    let packed = dimensions.load(Ordering::Acquire);
    assert!(packed != 0);
    let evidence = serde_json::json!({ "route": route, "source": source,
        "width": packed >> 32, "height": packed as u32, "duration_ms": clip.duration_ms,
        "size_bytes": clip.size_bytes, "gpu_inputs": metrics.gpu_surface_inputs,
        "encoder_readbacks": metrics.cpu_readback_inputs, "backend": metrics.active_backend,
        "decoded": true, "injection": false, "monitor_fallback": false, "explicit_monitor_crop": guarded,
        "game_pixels_verified": false, "game_capture_acceptance_passed": false,
        "format_events": formats.load(Ordering::Relaxed),
        "automatic_discovery": automatic, "first_focus_wait_verified": test_first_focus,
        "audio_tested": false, "alt_tab_tested": false, "fullscreen_mode_verified": false });
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("replay-evidence.json"))
        .unwrap();
    serde_json::to_writer_pretty(&mut file, &evidence).unwrap();
    println!(
        "EXPORT VERIFIED ONLY (game pixels require separate inspection): selected VALORANT {route} replay saved/decoded: {} {evidence}",
        destination.display()
    );
}
