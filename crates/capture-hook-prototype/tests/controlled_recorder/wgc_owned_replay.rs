use super::*;
use capture_hook_prototype::d3d12_cross_process::Peer;
use capture_hook_prototype::d3d12_protocol::{WindowAction, Wire};
use capture_hook_prototype::transport::Message;
use capture_windows::DisplayCapture;
use std::sync::Arc;
struct FormatProbe {
    capture: DisplayCapture,
    dimensions: Arc<std::sync::Mutex<Option<(u32, u32)>>>,
}
impl VideoCapture for FormatProbe {
    fn enumerate_sources(&self) -> capture_api::Result<Vec<media_types::VideoSourceInfo>> {
        self.capture.enumerate_sources()
    }
    fn start(&mut self, config: VideoCaptureConfig) -> capture_api::Result<()> {
        self.capture.start(config)?;
        assert_eq!(self.capture.backend_name(), Some("wgc-window"));
        Ok(())
    }
    fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
        let event = self.capture.next_event()?;
        if let VideoCaptureEvent::FormatChanged { width, height } = &event {
            *self.dimensions.lock().unwrap() = Some((*width, *height));
        }
        Ok(event)
    }
    fn stop(&mut self) -> capture_api::Result<()> {
        self.capture.stop()
    }
    fn gpu_frame_context(&self) -> Option<encoder_api::GpuFrameContext> {
        self.capture.gpu_frame_context()
    }
}

fn record(warp: bool) {
    let peer = Peer::launch_bootstrap(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        warp,
        None,
    )
    .unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing owned greeting")
    };
    peer.approve(hello).unwrap();
    let target = hello.session.target;
    let source_id = format!(
        "game:pid:{}:created:{}:hwnd:{}",
        target.pid, target.creation_time, target.window
    );
    let dimensions = Arc::new(std::sync::Mutex::new(None));
    let capture = FormatProbe {
        capture: DisplayCapture::new(StreamId(0)),
        dimensions: dimensions.clone(),
    };
    let output = Output::new();
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: source_id,
            follow_source_dimensions: true,
            retention_ms: 400,
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
        Box::new(new_fallback_encoder(if warp {
            EncoderPreference::SoftwareOnly
        } else {
            EncoderPreference::HardwareOnly
        })),
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(20);
    let mut events = Vec::new();
    loop {
        events.extend(engine.pump());
        assert_ne!(engine.state(), RecorderState::Error, "{events:?}");
        if started.elapsed() > Duration::from_secs(3)
            && engine.state() == RecorderState::Ready
            && engine.buffer_duration_ms() >= 800
            && engine.buffered_packet_count() >= 10
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "WGC owned replay did not become ready: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let clip = engine.save_replay(&output.0.join("wgc-owned.mp4")).unwrap();
    let (width, height) = dimensions
        .lock()
        .unwrap()
        .expect("missing actual WGC dimensions");
    validate_mp4_dimensions(
        &clip.path,
        clip.size_bytes,
        ((width / 2 * 2) as u16, (height / 2 * 2) as u16),
    );
    if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
        decode_mp4_color(&PathBuf::from(ffmpeg), &clip.path, Some([255, 0, 0]));
    }
    let metrics = engine.encoder_metrics();
    println!("WGC replay diagnostics: actual={}x{}, GPU inputs={}, encoder readbacks={}, backend={:?}, events={events:?}",width,height,metrics.gpu_surface_inputs,metrics.cpu_readback_inputs,metrics.active_backend);
    if !warp {
        assert_eq!(metrics.cpu_readback_inputs, 0);
        assert!(metrics.gpu_surface_inputs >= 10);
    }
    peer.window_action(WindowAction::Minimize).unwrap();
    loop {
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Frame(_)) => {}
            Wire::Control(Message::Report { presented, .. })
                if presented == WindowAction::Minimize as u64 =>
            {
                break
            }
            other => panic!("unexpected minimize packet: {other:?}"),
        }
    }
    std::thread::sleep(Duration::from_millis(150));
    let stop_started = Instant::now();
    engine.stop().unwrap();
    assert!(
        stop_started.elapsed() < Duration::from_secs(2),
        "engine stop blocked on minimized WGC source"
    );
    println!("PASS: production isolated-window WGC -> {} -> saved decoded red owned-window replay, {}ms. No monitor fallback or real-game claim.",if warp {"software H.264"} else {"hardware-only H.264"},clip.duration_ms);
}
#[test]
#[ignore = "production WGC isolated owned window replay and software decode"]
fn wgc_owned_replay_warp() {
    record(true);
}
#[test]
#[ignore = "production WGC isolated owned window replay and hardware-only decode"]
fn wgc_owned_replay_hardware() {
    record(false);
}

#[test]
#[ignore = "explicit user-selected real game window replay test; OS WGC only; hardware encoder; saves and decodes MP4"]
fn wgc_real_game_replay_hardware() {
    let get = |name| std::env::var(name).map_err(|_| format!("explicit {name} opt-in is required"));
    let pid: u32 = get("SILK_REAL_GAME_PID").unwrap().parse().unwrap();
    let hwnd: usize = get("SILK_REAL_GAME_HWND").unwrap().parse().unwrap();
    let creation: u64 = get("SILK_REAL_GAME_CREATION").unwrap().parse().unwrap();
    let source_id = format!("game:pid:{pid}:created:{creation}:hwnd:{hwnd}");
    let dimensions = Arc::new(std::sync::Mutex::new(None));
    let capture = FormatProbe {
        capture: DisplayCapture::new(StreamId(0)),
        dimensions: dimensions.clone(),
    };
    let output = Output::new();
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: source_id,
            follow_source_dimensions: true,
            retention_ms: 1000,
            video_encoder: VideoEncoderConfig {
                width: 0,
                height: 0,
                fps: 60,
                codec: "h264".into(),
                keyframe_interval_seconds: 1.,
                ..Default::default()
            },
            ..Default::default()
        },
        Box::new(capture),
        Box::new(new_fallback_encoder(EncoderPreference::HardwareOnly)),
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(20);
    let mut events = Vec::new();
    loop {
        events.extend(engine.pump());
        assert_ne!(engine.state(), RecorderState::Error, "{events:?}");
        if started.elapsed() > Duration::from_secs(3)
            && engine.state() == RecorderState::Ready
            && engine.buffer_duration_ms() >= 1500
            && engine.buffered_packet_count() >= 30
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "real game replay did not become ready: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let clip = engine
        .save_replay(&output.0.join("real-game-replay.mp4"))
        .unwrap();
    let (width, height) = dimensions
        .lock()
        .unwrap()
        .expect("missing actual WGC dimensions");
    validate_mp4_dimensions(
        &clip.path,
        clip.size_bytes,
        ((width / 2 * 2) as u16, (height / 2 * 2) as u16),
    );
    if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
        let mut command = std::process::Command::new(ffmpeg);
        command
            .args(["-nostdin", "-v", "error", "-threads", "1", "-i"])
            .arg(&clip.path)
            .args(["-c:v", "rawvideo", "-f", "null", "-"]);
        let mut child = command.spawn().unwrap();
        assert!(
            child.wait().unwrap().success(),
            "FFmpeg failed to decode real game MP4 replay"
        );
    }
    let metrics = engine.encoder_metrics();
    println!(
        "PASS: real-game WGC -> hardware H.264 -> MP4 replay saved {}ms ({}x{}), gpu_surface_inputs={}, cpu_readback_inputs={}",
        clip.duration_ms, width, height, metrics.gpu_surface_inputs, metrics.cpu_readback_inputs
    );
    assert_eq!(metrics.cpu_readback_inputs, 0);
    assert!(metrics.gpu_surface_inputs >= 30);
    engine.stop().unwrap();
}
