use super::*;
use capture_hook_prototype::d3d12_cross_process::Peer;
use capture_hook_prototype::d3d12_protocol::Wire;
use capture_windows::DisplayCapture;
use std::sync::Arc;

struct GuardedFormatProbe {
    capture: DisplayCapture,
    dimensions: Arc<std::sync::Mutex<Option<(u32, u32)>>>,
}

impl VideoCapture for GuardedFormatProbe {
    fn enumerate_sources(&self) -> capture_api::Result<Vec<media_types::VideoSourceInfo>> {
        self.capture.enumerate_sources()
    }
    fn start(&mut self, config: VideoCaptureConfig) -> capture_api::Result<()> {
        self.capture.start(config)?;
        assert_eq!(self.capture.backend_name(), Some("guarded-window"));
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

fn record_guarded() {
    let peer = Peer::launch_bootstrap(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        false,
        None,
    )
    .unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing owned greeting")
    };
    peer.approve(hello).unwrap();
    let target = hello.session.target;
    let source_id = format!(
        "game-guarded:pid:{}:created:{}:hwnd:{}",
        target.pid, target.creation_time, target.window
    );
    let dimensions = Arc::new(std::sync::Mutex::new(None));
    let capture = GuardedFormatProbe {
        capture: DisplayCapture::new(StreamId(0)),
        dimensions: dimensions.clone(),
    };
    let output = Output::new();
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: source_id,
            follow_source_dimensions: true,
            retention_ms: 1500,
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
            && engine.buffer_duration_ms() >= 1000
            && engine.buffered_packet_count() >= 15
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Guarded replay did not become ready: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let clip = engine
        .save_replay(&output.0.join("guarded-owned.mp4"))
        .unwrap();
    let (width, height) = dimensions
        .lock()
        .unwrap()
        .expect("missing actual guarded dimensions");
    validate_mp4_dimensions(
        &clip.path,
        clip.size_bytes,
        ((width / 2 * 2) as u16, (height / 2 * 2) as u16),
    );
    let metrics = engine.encoder_metrics();
    println!(
        "Guarded replay diagnostics: actual={}x{}, GPU inputs={}, encoder readbacks={}, backend={:?}",
        width, height, metrics.gpu_surface_inputs, metrics.cpu_readback_inputs, metrics.active_backend
    );
    if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
        decode_mp4_color(&PathBuf::from(ffmpeg), &clip.path, None);
    }
    assert_eq!(metrics.cpu_readback_inputs, 0);
    assert!(metrics.gpu_surface_inputs >= 5);
    engine.stop().unwrap();
    drop(peer);
}

#[test]
fn hardware_guarded_window_replay_delivers_pure_gpu_frames() {
    record_guarded();
}
