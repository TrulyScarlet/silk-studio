use super::*;
use capture_hook_prototype::d3d12_cross_capture::{
    CrossCapture, CrossSource, DEVICE_REPLACE_SOURCE_ID, REARM_SOURCE_ID,
};
use gpu_windows::{BgraReadback, GpuTextureSlot};
use recorder_engine::EngineEvent;
fn source(warp: bool) -> CrossSource {
    CrossSource {
        executable: PathBuf::from(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        warp,
        fault: None,
        present_hook: true,
        factory_hook: true,
        bootstrap: true,
        legacy_factory: false,
        attached: false,
        late_resize: false,
        automatic_resize: false,
        rearm: true,
        device_replace: false,
    }
}
fn config() -> VideoCaptureConfig {
    VideoCaptureConfig {
        source_id: REARM_SOURCE_ID.into(),
        target_fps: 30,
    }
}
pub(super) fn mode(warp: bool, device_replace: bool) -> (CrossSource, VideoCaptureConfig) {
    let mut source = source(warp);
    source.device_replace = device_replace;
    let mut config = config();
    if device_replace {
        config.source_id = DEVICE_REPLACE_SOURCE_ID.into();
    }
    (source, config)
}
fn transition(
    control: &capture_hook_prototype::d3d12_cross_capture::CrossControl,
    dimensions: (u32, u32),
    device_replace: bool,
) {
    if device_replace {
        control.replace_device(dimensions.0, dimensions.1).unwrap();
    } else {
        control.resize(dimensions.0, dimensions.1).unwrap();
    }
}
pub(super) fn next_frame(capture: &mut CrossCapture) -> media_types::VideoFrame {
    loop {
        if let VideoCaptureEvent::Frame(frame) = capture.next_event().unwrap() {
            return frame;
        }
    }
}
pub(super) fn next_format(capture: &mut CrossCapture, dimensions: (u32, u32)) {
    loop {
        if let VideoCaptureEvent::FormatChanged { width, height } = capture.next_event().unwrap() {
            assert_eq!((width, height), dimensions);
            return;
        }
    }
}
pub(super) fn color(lease: &encoder_api::GpuFrameLease, expected: [u8; 4]) {
    let bytes = BgraReadback::default()
        .read_scaled(lease.downcast_ref::<GpuTextureSlot>().unwrap(), 64, 36)
        .unwrap();
    assert!(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == expected));
}

#[test]
#[ignore = "owned WARP live resize explicitly tests delivered-frame resolver gap"]
fn resize_delivery_barrier_warp() {
    delivery_barrier(false);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "observed resize must preserve the delivered-frame resolver gap"]
fn automatic_resize_delivery_barrier_warp() {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    let mut source = source(true);
    source.attached = true;
    source.late_resize = true;
    source.automatic_resize = true;
    source.rearm = false;
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture
        .start(VideoCaptureConfig {
            source_id: capture_hook_prototype::d3d12_cross_capture::AUTOMATIC_RESIZE_SOURCE_ID
                .into(),
            target_fps: 30,
        })
        .unwrap();
    next_format(&mut capture, (318, 178));
    let old_context = capture.gpu_frame_context().unwrap();
    let old_frame = next_frame(&mut capture);
    let stats = capture.stats();
    let target = stats.owned_target().unwrap();
    assert!(capture.control().resize(640, 360).is_err());
    std::thread::sleep(Duration::from_millis(100));
    unsafe {
        PostMessageW(
            Some(HWND(target.window as usize as *mut _)),
            capture_hook_prototype::d3d12_present_fixture::OWNED_GEOMETRY_MESSAGE,
            WPARAM(640),
            LPARAM(360),
        )
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while stats.current_epoch() != 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let retained = old_context
        .resolve(&old_frame)
        .expect("observed resize invalidated delivered-frame gap");
    color(&retained, [0, 0, 255, 255]);
    next_format(&mut capture, (640, 360));
    assert!(old_context.resolve(&old_frame).is_err());
    let new_frame = next_frame(&mut capture);
    assert_eq!((new_frame.width, new_frame.height), (640, 360));
    assert!(new_frame.pts > old_frame.pts);
    let fresh = capture
        .gpu_frame_context()
        .unwrap()
        .resolve(&new_frame)
        .unwrap();
    color(&fresh, [0, 255, 0, 255]);
    color(&retained, [0, 0, 255, 255]);
    capture.stop().unwrap();
    color(&fresh, [0, 255, 0, 255]);
    color(&retained, [0, 0, 255, 255]);
}
pub(super) fn delivery_barrier(device_replace: bool) {
    let (source, config) = mode(true, device_replace);
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture.start(config).unwrap();
    next_format(&mut capture, (318, 178));
    let old_context = capture.gpu_frame_context().unwrap();
    let frame = next_frame(&mut capture);
    let control = capture.control();
    let stats = capture.stats();
    std::thread::sleep(Duration::from_millis(100));
    transition(&control, (640, 360), device_replace);
    let deadline = Instant::now() + Duration::from_secs(5);
    while stats.current_epoch() != 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    // Old queued events are now rejected, but the already-delivered frame must
    // still resolve until the caller asks for the next event.
    let retained = old_context
        .resolve(&frame)
        .expect("resize invalidated delivered-frame resolver gap");
    color(&retained, [0, 0, 255, 255]);
    next_format(&mut capture, (640, 360));
    assert!(old_context.resolve(&frame).is_err());
    assert!(stats.retired_events.load(Ordering::Acquire) > 0);
    let new_context = capture.gpu_frame_context().unwrap();
    let new = next_frame(&mut capture);
    assert!(new.pts > frame.pts);
    assert_eq!((new.width, new.height), (640, 360));
    let lease = new_context.resolve(&new).unwrap();
    color(&lease, [0, 255, 0, 255]);
    color(&retained, [0, 0, 255, 255]);
    drop(retained);
    drop(old_context);
    drop(lease);
    drop(new_context);
    transition(&control, (800, 450), device_replace);
    next_format(&mut capture, (800, 450));
    let last_context = capture.gpu_frame_context().unwrap();
    let last = next_frame(&mut capture);
    let lease = last_context.resolve(&last).unwrap();
    color(&lease, [255, 0, 0, 255]);
    capture.stop().unwrap();
    color(&lease, [255, 0, 0, 255]);
    assert!(control.resize(640, 360).is_err());
}
#[test]
#[ignore = "retained old contexts must exhaust, not bypass, six-surface resize budget"]
fn resize_retained_context_budget_warp() {
    retained_budget(false);
}
pub(super) fn retained_budget(device_replace: bool) {
    let (source, config) = mode(true, device_replace);
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture.start(config).unwrap();
    next_format(&mut capture, (318, 178));
    let first = capture.gpu_frame_context().unwrap();
    let _ = next_frame(&mut capture);
    let control = capture.control();
    transition(&control, (640, 360), device_replace);
    next_format(&mut capture, (640, 360));
    let second = capture.gpu_frame_context().unwrap();
    let _ = next_frame(&mut capture);
    transition(&control, (800, 450), device_replace);
    // The mailbox request is asynchronous: a queued current-epoch frame may
    // arrive before the worker starts retirement. Only a third format would
    // prove that retained contexts bypassed the budget.
    let deadline = Instant::now() + Duration::from_secs(10);
    let error = loop {
        assert!(
            Instant::now() < deadline,
            "retained-context rejection deadline"
        );
        match capture.next_event() {
            Err(error) => break error,
            Ok(VideoCaptureEvent::FormatChanged { .. }) => {
                panic!("retained contexts bypassed resize budget")
            }
            Ok(VideoCaptureEvent::Frame(frame)) => {
                assert_eq!((frame.width, frame.height), (640, 360))
            }
            Ok(_) => {}
        }
    };
    assert!(error.to_string().contains("six-surface"), "{error}");
    assert!(first.session_resource().is_none());
    assert!(second.session_resource().is_none());
    capture.stop().unwrap();
}
pub(super) fn record(warp: bool, device_replace: bool, epochs: usize) {
    record_with_fault(warp, device_replace, epochs, None);
}
pub(super) fn record_with_fault(
    warp: bool,
    device_replace: bool,
    epochs: usize,
    fault: Option<&str>,
) {
    record_mode(warp, device_replace, epochs, fault, false);
}
fn record_mode(
    warp: bool,
    device_replace: bool,
    epochs: usize,
    fault: Option<&str>,
    automatic: bool,
) {
    let output = Output::new();
    let (mut source, mut config) = mode(warp, device_replace);
    if automatic {
        source.attached = true;
        source.late_resize = true;
        source.automatic_resize = true;
        source.rearm = false;
        config.source_id =
            capture_hook_prototype::d3d12_cross_capture::AUTOMATIC_RESIZE_SOURCE_ID.into();
    }
    source.fault = fault.map(str::to_owned);
    let capture = CrossCapture::new(source, StreamId(0));
    let control = capture.control();
    let preference = if warp {
        EncoderPreference::SoftwareOnly
    } else {
        EncoderPreference::HardwareOnly
    };
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
        Box::new(new_fallback_encoder(preference)),
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    for index in 0..epochs {
        let (width, height, expected) = if index == 0 {
            (318u32, 178u32, [255, 0, 0])
        } else if index % 2 == 1 {
            (640, 360, [0, 255, 0])
        } else if automatic {
            (800, 450, [0, 255, 0])
        } else {
            (800, 450, [0, 0, 255])
        };
        if index > 0 {
            if automatic {
                assert!(control.resize(width, height).is_err());
                #[cfg(feature = "d3d12-auto-resize-fixture")]
                {
                    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
                    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
                    let target = control.stats().unwrap().owned_target().unwrap();
                    unsafe {
                        PostMessageW(
                            Some(HWND(target.window as usize as *mut _)),
                            capture_hook_prototype::d3d12_present_fixture::OWNED_GEOMETRY_MESSAGE,
                            WPARAM(width as usize),
                            LPARAM(height as isize),
                        )
                        .unwrap();
                    }
                }
                #[cfg(not(feature = "d3d12-auto-resize-fixture"))]
                panic!("automatic fixture feature required");
            } else {
                transition(&control, (width, height), device_replace);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut changed = index == 0;
        let mut events = Vec::new();
        loop {
            let fresh = engine.pump();
            changed|=fresh.iter().any(|event|matches!(event,EngineEvent::Warning {code,message} if code=="CAPTURE_FORMAT_CHANGED" && message.contains(&format!("{width}x{height}"))));
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
                "resize epoch did not become ready: {events:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let clip = engine
            .save_replay(&output.0.join(format!("resize-{index}.mp4")))
            .unwrap();
        validate_mp4_dimensions(&clip.path, clip.size_bytes, (width as u16, height as u16));
        if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
            decode_mp4_color(&PathBuf::from(ffmpeg), &clip.path, Some(expected));
        }
        println!(
            "PASS: same-process {} epoch {} {}x{}, {}ms, {}",
            if automatic {
                "observed automatic resize"
            } else if device_replace {
                "device recreation"
            } else {
                "resize"
            },
            index + 1,
            width,
            height,
            clip.duration_ms,
            if warp { "WARP/software" } else { "hardware" }
        );
    }
    let metrics = engine.encoder_metrics();
    let stats = control.stats().unwrap();
    assert_eq!(stats.cpu_readbacks.load(Ordering::Acquire), 0);
    if !warp {
        assert_eq!(metrics.cpu_readback_inputs, 0);
        assert!(metrics.gpu_surface_inputs >= 10);
    }
    engine.stop().unwrap();
    assert!(stats.worker_exited.load(Ordering::Acquire));
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "owned automatic observed resize replay save/decode across three epochs"]
fn automatic_resize_replay_warp() {
    record_mode(true, false, 3, None, true);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "owned hardware automatic observed resize replay save/decode across three epochs"]
fn automatic_resize_replay_hardware() {
    record_mode(false, false, 3, None, true);
}
#[test]
#[ignore = "three live-resize replay epochs and decoded colors on owned WARP source"]
fn resize_replay_warp() {
    record(true, false, 3);
}
#[test]
#[ignore = "three live-resize replay epochs and decoded colors on owned hardware source"]
fn resize_replay_hardware() {
    record(false, false, 3);
}
