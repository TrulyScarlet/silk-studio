use super::d3d12_resize_replay::{color, mode, next_format, next_frame};
use super::*;
use capture_hook_prototype::d3d12_cross_capture::{CrossCapture, SOURCE_ID};

fn loss_source() -> (
    capture_hook_prototype::d3d12_cross_capture::CrossSource,
    VideoCaptureConfig,
) {
    let (mut source, mut config) = mode(false, false);
    source.rearm = false;
    source.bootstrap = false;
    source.present_hook = false;
    source.factory_hook = false;
    source.fault = Some("remove-on-command".into());
    config.source_id = SOURCE_ID.into();
    (source, config)
}
fn recovering_source() -> (
    capture_hook_prototype::d3d12_cross_capture::CrossSource,
    VideoCaptureConfig,
) {
    let (mut source, config) = loss_source();
    source.fault = Some("remove-and-recreate-on-command".into());
    (source, config)
}
#[test]
#[ignore = "actual hardware producer loss reconstructs same HWND across two epochs and preserves original local pixels"]
fn producer_loss_reconstructs_adapter_hardware() {
    reconstructed_adapter(2);
}
fn reconstructed_adapter(rounds: usize) {
    let (source, config) = recovering_source();
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture.start(config).unwrap();
    next_format(&mut capture, (318, 178));
    let original_context = capture.gpu_frame_context().unwrap();
    let original = next_frame(&mut capture);
    let retained = original_context.resolve(&original).unwrap();
    color(&retained, [0, 0, 255, 255]);
    let control = capture.control();
    let stats = capture.stats();
    let target = stats.owned_target().unwrap();
    let mut previous_pts = original.pts;
    for (index, (width, height, expected)) in
        [(640, 360, [0, 255, 0, 255]), (800, 450, [255, 0, 0, 255])]
            .into_iter()
            .cycle()
            .take(rounds)
            .enumerate()
    {
        control.trigger_owned_loss().unwrap();
        next_format(&mut capture, (width, height));
        assert_eq!(stats.current_epoch(), index as u64 + 2);
        assert_eq!(stats.owned_target(), Some(target));
        assert!(original_context.resolve(&original).is_err());
        let frame = next_frame(&mut capture);
        assert_eq!((frame.width, frame.height), (width, height));
        assert!(frame.pts > previous_pts);
        previous_pts = frame.pts;
        let context = capture.gpu_frame_context().unwrap();
        let lease = context.resolve(&frame).unwrap();
        color(&lease, expected);
        color(&retained, [0, 0, 255, 255]);
    }
    capture.stop().unwrap();
    color(&retained, [0, 0, 255, 255]);
    assert!(stats.worker_exited.load(Ordering::Acquire));
    assert!(control.trigger_owned_loss().is_err());
}
#[test]
#[ignore = "hardware replay resumes and saves/decodes all three formats after actual producer removal and exact consumer retirement"]
fn producer_loss_reconstructed_replay_hardware() {
    let (source, config) = recovering_source();
    let capture = CrossCapture::new(source, StreamId(0));
    let control = capture.control();
    let output = Output::new();
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: config.source_id,
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
        Box::new(new_fallback_encoder(EncoderPreference::HardwareOnly)),
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    let stats = control.stats().unwrap();
    for (index, (width, height, expected)) in [
        (318, 178, [255, 0, 0]),
        (640, 360, [0, 255, 0]),
        (800, 450, [0, 0, 255]),
    ]
    .into_iter()
    .enumerate()
    {
        if index != 0 {
            control.trigger_owned_loss().unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut changed = index == 0;
        let mut events = Vec::new();
        loop {
            let fresh = engine.pump();
            changed |= fresh.iter().any(|event| matches!(event, recorder_engine::EngineEvent::Warning { code, message } if code == "CAPTURE_FORMAT_CHANGED" && message.contains(&format!("{width}x{height}"))));
            events.extend(fresh);
            assert_ne!(engine.state(), RecorderState::Error, "{events:?}");
            if changed
                && engine.state() == RecorderState::Ready
                && engine.buffer_duration_ms() >= 800
                && engine.buffered_packet_count() >= 10
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "loss reconstruction replay did not become ready: {events:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let clip = engine
            .save_replay(&output.0.join(format!("reconstructed-loss-{index}.mp4")))
            .unwrap();
        validate_mp4_dimensions(&clip.path, clip.size_bytes, (width as u16, height as u16));
        if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
            decode_mp4_color(&PathBuf::from(ffmpeg), &clip.path, Some(expected));
        }
        assert_eq!(stats.current_epoch(), index as u64 + 1);
        assert!(std::sync::Arc::ptr_eq(&stats, &control.stats().unwrap()));
    }
    let metrics = engine.encoder_metrics();
    assert_eq!(metrics.cpu_readback_inputs, 0);
    assert!(metrics.gpu_surface_inputs >= 10);
    assert_eq!(stats.cpu_readbacks.load(Ordering::Acquire), 0);
    engine.stop().unwrap();
    assert!(stats.worker_exited.load(Ordering::Acquire));
}
#[test]
#[ignore = "twenty same-process hardware losses reconstruct behind exact proofs with original pixels and unchanged budgets"]
fn producer_loss_reconstruction_twenty_epochs_hardware() {
    reconstructed_adapter(20);
}
fn loss_boundary(stop_at_barrier: bool) {
    let (source, config) = loss_source();
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture.start(config).unwrap();
    next_format(&mut capture, (318, 178));
    let context = capture.gpu_frame_context().unwrap();
    let frame = next_frame(&mut capture);
    let retained = context.resolve(&frame).unwrap();
    color(&retained, [0, 0, 255, 255]);
    let control = capture.control();
    let stats = capture.stats();
    assert!(control.resize(640, 360).is_err());
    assert!(control.replace_device(640, 360).is_err());
    control.trigger_owned_loss().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while stats.current_epoch() != 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        stats.source_live(),
        "producer exited before loss delivery boundary"
    );
    assert!(
        context.resolve(&frame).is_ok(),
        "loss invalidated delivered resolver gap before caller crossed boundary"
    );
    if !stop_at_barrier {
        let error = capture
            .next_event()
            .expect_err("unsupported automatic replacement delivered new media");
        assert!(error.to_string().contains("device lost"), "{error}");
        assert!(context.resolve(&frame).is_err());
        assert!(capture.gpu_frame_context().is_none());
    }
    let stop_started = Instant::now();
    capture.stop().unwrap();
    assert!(stop_started.elapsed() < Duration::from_secs(2));
    assert!(stats.worker_exited.load(Ordering::Acquire));
    assert!(!stats.source_live());
    color(&retained, [0, 0, 255, 255]);
    assert!(control.trigger_owned_loss().is_err());
}
#[test]
#[ignore = "owned hardware producer loss preserves adapter delivery gap then retires terminally without fallback"]
fn producer_loss_adapter_delivery_boundary_hardware() {
    loss_boundary(false);
}
#[test]
#[ignore = "owned hardware loss can stop while adapter waits on its delivered-frame boundary"]
fn producer_loss_adapter_stop_at_boundary_hardware() {
    loss_boundary(true);
}
#[test]
fn producer_loss_probe_rejects_non_manual_or_warp_sources_before_worker_start() {
    for fault in ["remove-on-command", "remove-and-recreate-on-command"] {
        for kind in 0..4 {
            let (mut source, config) = loss_source();
            source.fault = Some(fault.into());
            match kind {
                0 => source.warp = true,
                1 => source.present_hook = true,
                2 => source.bootstrap = true,
                _ => source.attached = true,
            }
            let mut capture = CrossCapture::new(source, StreamId(0));
            assert!(capture.start(config).is_err());
            assert!(capture.gpu_frame_context().is_none());
            assert!(capture.control().trigger_owned_loss().is_err());
        }
    }
}
#[test]
#[ignore = "automatic loss reconstruction cannot allocate around retained old contexts beyond the six-surface budget"]
fn producer_loss_reconstruction_retained_budget_hardware() {
    let (source, config) = recovering_source();
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture.start(config).unwrap();
    next_format(&mut capture, (318, 178));
    let first = capture.gpu_frame_context().unwrap();
    let _ = next_frame(&mut capture);
    let control = capture.control();
    control.trigger_owned_loss().unwrap();
    next_format(&mut capture, (640, 360));
    let second = capture.gpu_frame_context().unwrap();
    let _ = next_frame(&mut capture);
    control.trigger_owned_loss().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let error = loop {
        assert!(Instant::now() < deadline);
        match capture.next_event() {
            Err(error) => break error,
            Ok(VideoCaptureEvent::Frame(frame)) => {
                assert_eq!((frame.width, frame.height), (640, 360))
            }
            Ok(VideoCaptureEvent::FormatChanged { .. }) => {
                panic!("loss reconstruction bypassed retained-context budget")
            }
            Ok(_) => {}
        }
    };
    assert!(error.to_string().contains("six-surface"), "{error}");
    assert!(first.session_resource().is_none());
    assert!(second.session_resource().is_none());
    capture.stop().unwrap();
}
#[test]
#[ignore = "automatic loss reconstruction can stop while the caller holds the old delivered frame"]
fn producer_loss_reconstruction_stop_at_delivery_boundary_hardware() {
    let (source, config) = recovering_source();
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture.start(config).unwrap();
    next_format(&mut capture, (318, 178));
    let context = capture.gpu_frame_context().unwrap();
    let frame = next_frame(&mut capture);
    let retained = context.resolve(&frame).unwrap();
    let control = capture.control();
    let stats = capture.stats();
    control.trigger_owned_loss().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while stats.current_epoch() != 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(context.resolve(&frame).is_ok());
    let started = Instant::now();
    capture.stop().unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    color(&retained, [0, 0, 255, 255]);
    assert!(stats.worker_exited.load(Ordering::Acquire));
    assert!(control.trigger_owned_loss().is_err());
}
#[test]
#[ignore = "hardware RecorderEngine saves valid game-only fixture replay, then stops terminally on unannounced producer loss without spawning a new source"]
fn producer_loss_engine_no_silent_restart_hardware() {
    let (source, config) = loss_source();
    let capture = CrossCapture::new(source, StreamId(0));
    let control = capture.control();
    let output = Output::new();
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: config.source_id,
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
        Box::new(new_fallback_encoder(EncoderPreference::HardwareOnly)),
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    let started = Instant::now();
    let mut events = Vec::new();
    loop {
        events.extend(engine.pump());
        assert_ne!(engine.state(), RecorderState::Error, "{events:?}");
        if started.elapsed() >= Duration::from_secs(3)
            && engine.state() == RecorderState::Ready
            && engine.buffer_duration_ms() >= 800
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "loss fixture replay did not become ready: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let clip = engine
        .save_replay(&output.0.join("before-producer-loss.mp4"))
        .unwrap();
    validate_mp4_dimensions(&clip.path, clip.size_bytes, (318, 178));
    if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
        decode_mp4_color(&PathBuf::from(ffmpeg), &clip.path, Some([255, 0, 0]));
    }
    let original_stats = control.stats().unwrap();
    control.trigger_owned_loss().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        events.extend(engine.pump());
        if engine.state() == RecorderState::Error {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "producer loss did not stop engine: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(events.iter().any(|event| matches!(event, recorder_engine::EngineEvent::Warning { code, message } if code == "CAPTURE_BACKEND_FAILURE" && message.contains("device lost"))), "{events:?}");
    assert!(!events.iter().any(|event| matches!(event, recorder_engine::EngineEvent::RecoveryStarted { component } if component == "display")), "unsupported replacement silently restarted capture: {events:?}");
    assert!(std::sync::Arc::ptr_eq(
        &original_stats,
        &control.stats().unwrap()
    ));
    assert!(!original_stats.source_live());
    let metrics = engine.encoder_metrics();
    assert_eq!(metrics.cpu_readback_inputs, 0);
    assert!(metrics.gpu_surface_inputs >= 10);
    engine.stop().unwrap();
    assert!(control.trigger_owned_loss().is_err());
    println!("PASS: owned hardware replay saved before actual producer removal; delivered imports retired, engine reports terminal loss without retarget/fallback; not loss recovery or real-game acceptance");
}
