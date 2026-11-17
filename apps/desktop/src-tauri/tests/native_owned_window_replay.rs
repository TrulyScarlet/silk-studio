//! Non-injected owned-window replay acceptance; no game selection required.
#![cfg(windows)]

use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent};
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
use windows::core::w;
use windows::Win32::Foundation::{FILETIME, HWND};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::WindowsAndMessaging::*;

#[test]
#[ignore = "helper for owned-window acceptance; opens a controlled window"]
fn owned_window_child() {
    let Ok(directory) = std::env::var("SILK_OWNED_REPLAY_CHILD") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Silk owned replay acceptance"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            60,
            60,
            640,
            480,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    };
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .unwrap();
    }
    let creation = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    std::fs::write(
        directory.join("source.txt"),
        format!(
            "game:pid:{}:created:{creation}:hwnd:{}",
            std::process::id(),
            window.0 as usize
        ),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut tick = 0;
    while Instant::now() < deadline && !directory.join("stop").exists() {
        if directory.join("exclude").exists() && !directory.join("excluded").exists() {
            unsafe {
                SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE).unwrap();
            }
            std::fs::write(directory.join("excluded"), "excluded").unwrap();
        }
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            // Animate owned content so saved replay readiness does not depend
            // on unrelated desktop activity.
            let title: Vec<u16> = format!("Silk owned replay frame {tick}\0")
                .encode_utf16()
                .collect();
            SetWindowTextW(window, windows::core::PCWSTR(title.as_ptr())).unwrap();
        }
        tick += 1;
        std::thread::sleep(Duration::from_millis(30));
    }
    let _ = unsafe { DestroyWindow(window) };
}

struct Fixture {
    child: std::process::Child,
    directory: PathBuf,
    source: String,
    window: HWND,
}
impl Fixture {
    fn start(directory: PathBuf) -> Self {
        std::fs::create_dir(&directory).unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "owned_window_child", "--nocapture"])
            .env("SILK_OWNED_REPLAY_CHILD", &directory)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut fixture = Self {
            child,
            directory,
            source: String::new(),
            window: HWND::default(),
        };
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Ok(source) = std::fs::read_to_string(fixture.directory.join("source.txt")) {
                if let Some(handle) = source
                    .rsplit(":hwnd:")
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                {
                    fixture.window = HWND(handle as *mut _);
                    fixture.source = source;
                    break;
                }
            }
            assert!(Instant::now() < deadline, "owned fixture startup timeout");
            assert!(
                fixture.child.try_wait().unwrap().is_none(),
                "fixture exited before ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        fixture
    }
    fn close(&mut self) {
        std::fs::write(self.directory.join("stop"), "stop").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "fixture shutdown timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct WgcOnly {
    inner: DisplayCapture,
    dimensions: Arc<AtomicU64>,
}
impl VideoCapture for WgcOnly {
    fn enumerate_sources(&self) -> capture_api::Result<Vec<media_types::VideoSourceInfo>> {
        self.inner.enumerate_sources()
    }
    fn start(&mut self, config: VideoCaptureConfig) -> capture_api::Result<()> {
        self.inner.start(config)?;
        assert_eq!(self.inner.backend_name(), Some("wgc-window"));
        Ok(())
    }
    fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
        let event = self.inner.next_event()?;
        if let VideoCaptureEvent::FormatChanged { width, height } = event {
            eprintln!("owned WGC format: {width}x{height}");
            self.dimensions.store(
                (u64::from(width) << 32) | u64::from(height),
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

fn decoder() -> PathBuf {
    let path = PathBuf::from(
        std::env::var("SILK_OWNED_REPLAY_FFMPEG").expect("approved FFmpeg path required"),
    );
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
            assert!(status.success(), "replay decode failed");
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("decoder timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn ready(engine: &mut RecorderEngine, dimensions: &AtomicU64, expected_width: u32) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let events = engine.pump();
        assert_ne!(engine.state(), RecorderState::Error, "{events:?}");
        let size = dimensions.load(Ordering::Acquire);
        if engine.state() == RecorderState::Ready
            && engine.buffer_duration_ms() >= 800
            && (size >> 32 >= u64::from(expected_width.saturating_sub(80)))
            && (size >> 32 <= u64::from(expected_width))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "replay readiness timeout: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "interactive WGC + hardware H264 + approved FFmpeg; owned windows only"]
fn owned_window_replays_survive_resize_focus_and_restart() {
    let decoder = decoder();
    let root = std::env::temp_dir().join("opencode").join(format!(
        "silk-owned-replay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut fixture = Fixture::start(root.join("first"));
    let dimensions = Arc::new(AtomicU64::new(0));
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: fixture.source.clone(),
            follow_source_dimensions: true,
            retention_ms: 1000,
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
        Box::new(WgcOnly {
            inner: DisplayCapture::new(StreamId(0)),
            dimensions: dimensions.clone(),
        }),
        Box::new(encoder_media_foundation::new_fallback_encoder(
            EncoderPreference::HardwareOnly,
        )),
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    struct FocusWindow(HWND);
    impl Drop for FocusWindow {
        fn drop(&mut self) {
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }
    let focus_window = FocusWindow(unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Silk owned focus-change fixture"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            1050,
            60,
            320,
            240,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    });
    let mut confirmed_focus_changes = 0;
    let mut clips = Vec::new();
    for phase in 0..6 {
        if phase > 0 {
            unsafe {
                let _ = ShowWindow(fixture.window, SW_MINIMIZE);
            }
            let until = Instant::now() + Duration::from_millis(400);
            while Instant::now() < until {
                engine.pump();
                std::thread::sleep(Duration::from_millis(10));
            }
            unsafe {
                let _ = ShowWindow(fixture.window, SW_RESTORE);
                let (width, height) = if phase % 2 == 1 {
                    (800, 600)
                } else {
                    (960, 720)
                };
                SetWindowPos(
                    fixture.window,
                    None,
                    60 + phase * 10,
                    60,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .unwrap();
            }
        }
        ready(
            &mut engine,
            &dimensions,
            if phase == 0 {
                640
            } else if phase % 2 == 1 {
                800
            } else {
                960
            },
        );
        unsafe {
            let _ = SetForegroundWindow(focus_window.0);
        }
        if unsafe { GetForegroundWindow() } == focus_window.0 {
            confirmed_focus_changes += 1;
        }
        // Keep pumping while another owned window is foreground. WGC must
        // remain bound to the selected target, unlike monitor cropping.
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            engine.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        for save in 0..3 {
            let path = root.join(format!("phase-{phase}-save-{save}.mp4"));
            let clip = engine.save_replay(&path).unwrap();
            assert!(clip.size_bytes > 0 && clip.duration_ms > 0);
            clips.push(path);
            engine.pump();
        }
    }
    let metrics = engine.encoder_metrics();
    assert_eq!(metrics.cpu_readback_inputs, 0);
    assert!(metrics.gpu_surface_inputs >= 10);
    engine.stop().unwrap();
    let stale = fixture.source.clone();
    fixture.close();
    let mut replacement = Fixture::start(root.join("replacement"));
    assert_ne!(replacement.source, stale);
    let mut capture = DisplayCapture::new(StreamId(0));
    assert!(matches!(
        capture.start(VideoCaptureConfig {
            source_id: stale,
            target_fps: 30
        }),
        Err(VideoCaptureError::SourceUnavailable)
    ));
    assert_eq!(capture.backend_name(), None);
    capture
        .start(VideoCaptureConfig {
            source_id: replacement.source.clone(),
            target_fps: 30,
        })
        .unwrap();
    assert_eq!(capture.backend_name(), Some("wgc-window"));
    capture.stop().unwrap();
    // Exclusion is set by the owner process, never by altering another app.
    std::fs::write(replacement.directory.join("exclude"), "exclude").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !replacement.directory.join("excluded").exists() {
        assert!(
            Instant::now() < deadline,
            "owned exclusion acknowledgement timeout"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        capture.start(VideoCaptureConfig {
            source_id: replacement.source.clone(),
            target_fps: 30,
        }),
        Err(VideoCaptureError::PermissionDenied)
    ));
    assert_eq!(capture.backend_name(), None);
    assert!(capture.gpu_frame_context().is_none());
    let guarded_source = replacement.source.replacen("game:", "game-guarded:", 1);
    assert!(matches!(
        capture.start(VideoCaptureConfig {
            source_id: guarded_source,
            target_fps: 30,
        }),
        Err(VideoCaptureError::PermissionDenied)
    ));
    assert_eq!(capture.backend_name(), None);
    assert!(capture.gpu_frame_context().is_none());
    replacement.close();
    for clip in &clips {
        decode(&decoder, clip);
    }
    let evidence = serde_json::json!({ "clips_saved_and_decoded": clips.len(), "resize_restore_cycles": 5,
        "route": "wgc-window", "stale_identity_rejected": true, "fresh_identity_started": true,
        "external_owned_exclusion_denied_without_fallback": true,
        "encoder_readbacks": metrics.cpu_readback_inputs, "gpu_inputs": metrics.gpu_surface_inputs,
        "confirmed_other_window_foreground_phases": confirmed_focus_changes,
        "audio_tested": false, "real_game_tested": false });
    std::fs::write(
        root.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    println!(
        "PASS: owned-window replay evidence: {} {evidence}",
        root.display()
    );
}
