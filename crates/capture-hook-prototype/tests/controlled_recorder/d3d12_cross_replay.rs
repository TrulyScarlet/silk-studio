use super::*;
use capture_hook_prototype::d3d12_cross_capture::{
    CrossCapture, CrossSource, SOURCE_ID as CROSS_ID,
};

fn cross_source(warp: bool, fault: Option<&str>) -> CrossSource {
    CrossSource {
        executable: PathBuf::from(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        warp,
        fault: fault.map(str::to_owned),
        present_hook: false,
        factory_hook: false,
        bootstrap: false,
        legacy_factory: false,
        attached: false,
        late_resize: false,
        automatic_resize: false,
        rearm: false,
        device_replace: false,
    }
}
fn cross_config() -> VideoCaptureConfig {
    VideoCaptureConfig {
        source_id: CROSS_ID.into(),
        target_fps: 30,
    }
}
#[test]
fn invalid_sources_and_missing_child_fail_before_media_delivery() {
    for (attached, legacy) in [(false, false), (true, true)] {
        let mut source = cross_source(true, None);
        source.late_resize = true;
        source.attached = attached;
        source.legacy_factory = legacy;
        source.bootstrap = true;
        source.present_hook = true;
        source.factory_hook = true;
        let mut capture = CrossCapture::new(source, StreamId(0));
        assert!(capture
            .start(VideoCaptureConfig {
                source_id: capture_hook_prototype::d3d12_cross_capture::LATE_RESIZE_SOURCE_ID
                    .into(),
                target_fps: 30
            })
            .is_err());
        assert!(capture.gpu_frame_context().is_none());
    }
    for (bootstrap, legacy, rearm) in [
        (false, false, false),
        (false, true, false),
        (true, false, true),
    ] {
        let mut source = cross_source(true, None);
        source.attached = true;
        source.present_hook = true;
        source.factory_hook = true;
        source.bootstrap = bootstrap;
        source.legacy_factory = legacy;
        source.rearm = rearm;
        let mut capture = CrossCapture::new(source, StreamId(0));
        assert!(capture
            .start(VideoCaptureConfig {
                source_id: capture_hook_prototype::d3d12_cross_capture::ATTACHED_SOURCE_ID.into(),
                target_fps: 30
            })
            .is_err());
        assert!(capture.gpu_frame_context().is_none());
    }
    let mut legacy = cross_source(true, None);
    legacy.legacy_factory = true;
    let mut legacy = CrossCapture::new(legacy, StreamId(0));
    assert!(legacy.start(cross_config()).is_err());
    assert!(legacy.gpu_frame_context().is_none());
    let mut invalid = cross_source(true, None);
    invalid.factory_hook = true;
    invalid.present_hook = false;
    let mut invalid = CrossCapture::new(invalid, StreamId(0));
    assert!(invalid
        .start(VideoCaptureConfig {
            source_id: capture_hook_prototype::d3d12_cross_capture::FACTORY_SOURCE_ID.into(),
            target_fps: 30
        })
        .is_err());
    assert!(invalid.gpu_frame_context().is_none());
    let mut source = cross_source(true, None);
    source
        .executable
        .set_file_name("silk-missing-d3d12-fixture.exe");
    let mut capture = CrossCapture::new(source, StreamId(0));
    for (id, fps) in [("auto", 30), ("game:auto", 30), (CROSS_ID, 144)] {
        assert!(capture
            .start(VideoCaptureConfig {
                source_id: id.into(),
                target_fps: fps
            })
            .is_err());
    }
    for _ in 0..2 {
        assert!(capture.start(cross_config()).is_err());
        assert!(capture.stats().worker_exited.load(Ordering::Acquire));
        capture.stop().unwrap();
    }
}
fn record(warp: bool) {
    record_mode(warp, false);
}
fn record_mode(warp: bool, present_hook: bool) {
    record_route(warp, present_hook, false);
}
fn record_route(warp: bool, present_hook: bool, factory_hook: bool) {
    record_worker_route(warp, present_hook, factory_hook, false);
}
fn record_worker_route(warp: bool, present_hook: bool, factory_hook: bool, bootstrap: bool) {
    record_factory_route(warp, present_hook, factory_hook, bootstrap, false);
}
fn record_factory_route(
    warp: bool,
    present_hook: bool,
    factory_hook: bool,
    bootstrap: bool,
    legacy: bool,
) {
    record_attachment_route(warp, present_hook, factory_hook, bootstrap, legacy, false);
}
fn record_attachment_route(
    warp: bool,
    present_hook: bool,
    factory_hook: bool,
    bootstrap: bool,
    legacy: bool,
    attached: bool,
) {
    record_late_route(
        warp,
        present_hook,
        factory_hook,
        bootstrap,
        legacy,
        attached,
        false,
    );
}
fn record_late_route(
    warp: bool,
    present_hook: bool,
    factory_hook: bool,
    bootstrap: bool,
    legacy: bool,
    attached: bool,
    late: bool,
) {
    let output = Output::new();
    let mut source = cross_source(warp, None);
    source.present_hook = present_hook;
    source.factory_hook = factory_hook;
    source.bootstrap = bootstrap;
    source.legacy_factory = legacy;
    source.attached = attached;
    source.late_resize = late;
    let source_id = if late {
        capture_hook_prototype::d3d12_cross_capture::LATE_RESIZE_SOURCE_ID
    } else if attached {
        capture_hook_prototype::d3d12_cross_capture::ATTACHED_SOURCE_ID
    } else if bootstrap {
        capture_hook_prototype::d3d12_cross_capture::BOOTSTRAP_SOURCE_ID
    } else if factory_hook {
        capture_hook_prototype::d3d12_cross_capture::FACTORY_SOURCE_ID
    } else if present_hook {
        capture_hook_prototype::d3d12_cross_capture::HOOK_SOURCE_ID
    } else {
        CROSS_ID
    };
    let capture = CrossCapture::new(source, StreamId(0));
    let control = capture.control();
    let preference = if warp {
        EncoderPreference::SoftwareOnly
    } else {
        EncoderPreference::HardwareOnly
    };
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: source_id.into(),
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
        Box::new(new_fallback_encoder(preference)),
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        events.extend(engine.pump());
        assert_ne!(engine.state(), RecorderState::Error, "{events:?}");
        if engine.state() == RecorderState::Ready
            && engine.buffer_duration_ms() >= 800
            && engine.buffered_packet_count() >= 10
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(engine.state(), RecorderState::Ready, "{events:?}");
    assert!(engine.buffer_duration_ms() >= 800);
    let clip = engine.save_replay(&output.clip()).unwrap();
    assert!((700..=2000).contains(&clip.duration_ms));
    let metrics = engine.encoder_metrics();
    let stats = control.stats().unwrap();
    assert_eq!(stats.cpu_readbacks.load(Ordering::Acquire), 0);
    assert!(stats.gpu_copies.load(Ordering::Acquire) >= 10);
    if warp {
        assert!(metrics.cpu_readback_inputs >= 10);
    } else {
        assert!(metrics.gpu_surface_inputs >= 10);
        assert_eq!(metrics.cpu_readback_inputs, 0);
    }
    engine.stop().unwrap();
    assert!(stats.worker_exited.load(Ordering::Acquire));
    assert!(!stats.source_live());
    validate_mp4_dimensions(&clip.path, clip.size_bytes, (318, 178));
    if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
        decode_mp4_color(&PathBuf::from(ffmpeg), &clip.path, Some([255, 0, 0]));
    }
    let route = if late {
        "normal remote DLL load; already-rendering chain; authoritative later ResizeBuffers1"
    } else if attached {
        "normal remote DLL load; early passive fixture; metadata-only selection"
    } else if bootstrap {
        "independent in-process worker; metadata-only selection"
    } else if factory_hook {
        "factory-observed queue + Present detour"
    } else if present_hook {
        "Present detour"
    } else {
        "manual capture boundary"
    };
    println!("PASS: exact owned D3D12 child ({route}) -> cross-process shared GPU textures/fences -> local D3D11 -> {:?} -> replay MP4, {}ms, capture_readbacks=0, GPU_inputs={}, encoder_readbacks={}, backend={:?}",preference,clip.duration_ms,metrics.gpu_surface_inputs,metrics.cpu_readback_inputs,metrics.active_backend);
}
#[test]
#[ignore = "owned cross-process WARP D3D12 GPU recorder with software encoder"]
fn cross_gpu_replay_warp() {
    record(true);
}
#[test]
#[ignore = "owned cross-process D3D12 hardware-only GPU recorder"]
fn cross_gpu_replay_hardware() {
    record(false);
}
#[cfg(feature = "d3d12-present-hook-fixture")]
#[test]
#[ignore = "actual owned D3D12 Present detour and cross-process software-encoder replay"]
fn owned_present_hook_replay_warp() {
    record_mode(true, true);
}
#[cfg(feature = "d3d12-present-hook-fixture")]
#[test]
#[ignore = "actual owned D3D12 Present detour and cross-process hardware-only GPU replay"]
fn owned_present_hook_replay_hardware() {
    record_mode(false, true);
}
#[cfg(feature = "d3d12-factory-hook-fixture")]
#[test]
#[ignore = "actual owned factory/Present detours and software-encoder replay"]
fn factory_hook_replay_warp() {
    record_route(true, true, true);
}
#[cfg(feature = "d3d12-factory-hook-fixture")]
#[test]
#[ignore = "actual owned factory/Present detours and hardware-only GPU replay"]
fn factory_hook_replay_hardware() {
    record_route(false, true, true);
}
#[cfg(feature = "d3d12-bootstrap-fixture")]
#[test]
#[ignore = "independent owned D3D12 capture worker and WARP/software replay"]
fn bootstrap_replay_warp() {
    record_worker_route(true, true, true, true);
}
#[cfg(feature = "d3d12-bootstrap-fixture")]
#[test]
#[ignore = "independent owned D3D12 capture worker and hardware-only replay"]
fn bootstrap_replay_hardware() {
    record_worker_route(false, true, true, true);
}
#[cfg(feature = "d3d12-remote-helper-fixture")]
#[test]
#[ignore = "actual normal DLL load into exact passive owned D3D12 child and saved/decoded software replay"]
fn attached_helper_replay_warp() {
    record_attachment_route(true, true, true, true, false, true);
}
#[cfg(feature = "d3d12-remote-helper-fixture")]
#[test]
#[ignore = "actual normal DLL load into exact passive owned D3D12 child and saved/decoded hardware replay"]
fn attached_helper_replay_hardware() {
    record_attachment_route(false, true, true, true, false, true);
}
#[cfg(feature = "d3d12-remote-helper-fixture")]
#[test]
#[ignore = "remote DLL legacy factory selection and saved/decoded WARP replay"]
fn attached_legacy_replay_warp() {
    record_attachment_route(true, true, true, true, true, true);
}
#[cfg(feature = "d3d12-remote-helper-fixture")]
#[test]
#[ignore = "remote DLL legacy factory selection and saved/decoded hardware replay"]
fn attached_legacy_replay_hardware() {
    record_attachment_route(false, true, true, true, true, true);
}
#[cfg(feature = "d3d12-late-resize-fixture")]
#[test]
#[ignore = "already-rendering WARP source, actual late ResizeBuffers1 discovery, MP4 save/decode"]
fn attached_late_resize_replay_warp() {
    record_late_route(true, true, true, true, false, true, true);
}
#[cfg(feature = "d3d12-late-resize-fixture")]
#[test]
#[ignore = "already-rendering hardware source, actual late ResizeBuffers1 discovery, MP4 save/decode"]
fn attached_late_resize_replay_hardware() {
    record_late_route(false, true, true, true, false, true, true);
}
#[cfg(feature = "d3d12-bootstrap-fixture")]
#[test]
#[ignore = "owned legacy CreateSwapChain observation and saved/decoded software replay"]
fn legacy_factory_replay_warp() {
    record_factory_route(true, true, true, true, true);
}
#[cfg(feature = "d3d12-bootstrap-fixture")]
#[test]
#[ignore = "owned legacy CreateSwapChain observation and saved/decoded hardware replay"]
fn legacy_factory_replay_hardware() {
    record_factory_route(false, true, true, true, true);
}

#[cfg(feature = "d3d12-bootstrap-fixture")]
#[test]
#[ignore = "owned independent bootstrap producer exits with pending recorder events"]
fn bootstrap_dead_producer_warp() {
    let mut source = cross_source(true, Some("exit-after-six"));
    source.present_hook = true;
    source.factory_hook = true;
    source.bootstrap = true;
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture
        .start(VideoCaptureConfig {
            source_id: capture_hook_prototype::d3d12_cross_capture::BOOTSTRAP_SOURCE_ID.into(),
            target_fps: 30,
        })
        .unwrap();
    let context = capture.gpu_frame_context().unwrap();
    let stats = capture.stats();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !stats.worker_exited.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(capture.next_event().is_err());
    assert!(stats.retired_events.load(Ordering::Acquire) > 0);
    assert!(context.session_resource().is_none());
    assert!(capture.gpu_frame_context().is_none());
    capture.stop().unwrap();
}

#[test]
#[ignore = "owned D3D12 WARP child deliberately exits with pending media"]
fn dead_producer_discards_pending_frames_warp() {
    let mut capture = CrossCapture::new(cross_source(true, Some("exit-after-six")), StreamId(0));
    capture.start(cross_config()).unwrap();
    let context = capture.gpu_frame_context().unwrap();
    let stats = capture.stats();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !stats.worker_exited.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(capture.next_event().is_err());
    assert!(stats.retired_events.load(Ordering::Acquire) > 0);
    assert!(context.session_resource().is_none());
    assert!(capture.gpu_frame_context().is_none());
    capture.stop().unwrap();
}
#[test]
#[ignore = "owned D3D12 WARP retained GPU leases and bounded controller restarts"]
fn cross_retained_leases_and_restart_warp() {
    use gpu_windows::{BgraReadback, GpuTextureSlot};
    let mut capture = CrossCapture::new(cross_source(true, None), StreamId(0));
    capture.start(cross_config()).unwrap();
    let context = capture.gpu_frame_context().unwrap();
    assert!(matches!(
        capture.next_event().unwrap(),
        VideoCaptureEvent::FormatChanged { .. }
    ));
    let mut frames = Vec::new();
    let mut leases = Vec::new();
    for _ in 0..3 {
        let VideoCaptureEvent::Frame(frame) = capture.next_event().unwrap() else {
            panic!("missing cross GPU frame")
        };
        leases.push(context.resolve(&frame).unwrap());
        frames.push(frame);
    }
    assert!(context.resolve(&frames[0]).is_err());
    let stats = capture.stats();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(stats.gpu_copies.load(Ordering::Acquire), 3);
    assert!(stats.dropped.load(Ordering::Acquire) > 0);
    capture.stop().unwrap();
    assert!(context.resolve(&frames[2]).is_err());
    let pixels = BgraReadback::default()
        .read_scaled(
            leases[0].downcast_ref::<GpuTextureSlot>().unwrap(),
            318,
            178,
        )
        .unwrap();
    assert!(pixels
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| *p == [0, 0, 255, 255]));
    capture.start(cross_config()).unwrap();
    let second = capture.gpu_frame_context().unwrap();
    capture.stop().unwrap();
    let error = capture
        .start(cross_config())
        .expect_err("retained contexts bypassed bounded restart");
    assert!(error.to_string().contains("six-surface"), "{error}");
    drop(second);
    capture.start(cross_config()).unwrap();
    capture.stop().unwrap();
    drop(leases);
    drop(context);
}
