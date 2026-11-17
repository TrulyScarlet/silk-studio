use super::d3d12_resize_replay::record_with_fault;
use super::d3d12_resize_replay::{
    color, delivery_barrier, mode, next_format, next_frame, record, retained_budget,
};
use super::*;
use capture_hook_prototype::d3d12_cross_capture::CrossCapture;

#[test]
#[ignore = "owned WARP device recreation preserves delivered-frame resolver boundary"]
fn replacement_delivery_barrier_warp() {
    delivery_barrier(true);
}
#[test]
#[ignore = "owned WARP device recreation obeys six-surface retained-context budget"]
fn replacement_retained_context_budget_warp() {
    retained_budget(true);
}
#[test]
#[ignore = "three recreated-device GPU replay epochs with software encoding and decode"]
fn replacement_replay_warp() {
    record(true, true, 3);
}
#[test]
#[ignore = "three recreated-device GPU replay epochs with hardware encoding and decode"]
fn replacement_replay_hardware() {
    record(false, true, 3);
}
#[test]
#[ignore = "three owned replay epochs across quiescent RemoveDevice on WARP"]
fn replacement_quiescent_removal_replay_warp() {
    record_with_fault(true, true, 3, Some("replace-remove-quiescent"));
}
#[test]
#[ignore = "three owned replay epochs across quiescent RemoveDevice on hardware"]
fn replacement_quiescent_removal_replay_hardware() {
    record_with_fault(false, true, 3, Some("replace-remove-quiescent"));
}
#[test]
#[ignore = "three saved/decode hardware epochs across retired-remote pending producer copy removal"]
fn replacement_pending_copy_replay_hardware() {
    record_with_fault(false, true, 3, Some("replace-remove-pending"));
}
#[test]
#[ignore = "ten owned-device epochs with saved and decoded software replay each epoch"]
fn replacement_replay_soak_warp() {
    record(true, true, 10);
}
#[test]
#[ignore = "ten owned-device epochs with saved and decoded hardware replay each epoch"]
fn replacement_replay_soak_hardware() {
    record(false, true, 10);
}

#[test]
#[ignore = "recreation faults discard queued media, retire contexts, and stop terminally"]
fn replacement_failure_is_terminal_warp() {
    for fault in [
        "replace-after-release",
        "replace-before-hello",
        "replace-bad-hello",
    ] {
        let (mut source, config) = mode(true, true);
        source.fault = Some(fault.into());
        let mut capture = CrossCapture::new(source, StreamId(0));
        capture.start(config.clone()).unwrap();
        assert!(matches!(
            capture.next_event().unwrap(),
            VideoCaptureEvent::FormatChanged { .. }
        ));
        let old = capture.gpu_frame_context().unwrap();
        while !matches!(capture.next_event().unwrap(), VideoCaptureEvent::Frame(_)) {}
        let control = capture.control();
        control.replace_device(640, 360).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let error = loop {
            match capture.next_event() {
                Err(error) => break error,
                Ok(VideoCaptureEvent::Frame(_)) => {} // only already-delivered old epoch can precede retirement
                other => panic!("fault published a new format: {other:?}"),
            }
            assert!(Instant::now() < deadline);
        };
        assert!(!error.to_string().is_empty());
        assert!(old.session_resource().is_none());
        assert!(capture.gpu_frame_context().is_none());
        capture.stop().unwrap();
        let stats = control.stats().unwrap();
        assert!(stats.worker_exited.load(Ordering::Acquire));
        assert!(!stats.source_live());
        assert!(control.replace_device(640, 360).is_err());
        // An explicit stopped/new child is a new session, not silent recovery.
        capture.start(config).unwrap();
        while !matches!(capture.next_event().unwrap(), VideoCaptureEvent::Frame(_)) {}
        capture.stop().unwrap();
    }
}

#[test]
#[ignore = "stopping while recreation waits at delivery barrier remains bounded"]
fn replacement_stop_at_barrier_warp() {
    let (source, config) = mode(true, true);
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture.start(config).unwrap();
    while !matches!(capture.next_event().unwrap(), VideoCaptureEvent::Frame(_)) {}
    let control = capture.control();
    let stats = capture.stats();
    control.replace_device(640, 360).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while stats.current_epoch() != 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let start = Instant::now();
    capture.stop().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(stats.worker_exited.load(Ordering::Acquire));
    assert!(!stats.source_live());
}

#[test]
fn replacement_invalid_routes_do_not_launch() {
    let (mut source, config) = mode(true, true);
    source.rearm = false;
    let mut capture = CrossCapture::new(source, StreamId(0));
    assert!(capture.start(config).is_err());
    assert!(!capture.stats().source_live());
    assert!(capture.control().replace_device(640, 360).is_err());
}

fn same_size_mixed(warp: bool) {
    let (source, config) = mode(warp, true);
    let mut capture = CrossCapture::new(source, StreamId(0));
    capture.start(config).unwrap();
    next_format(&mut capture, (318, 178));
    let control = capture.control();
    for (index, recreate) in [true, false, true].into_iter().enumerate() {
        let context = capture.gpu_frame_context().unwrap();
        let before = next_frame(&mut capture);
        if recreate {
            control.replace_device(640, 360).unwrap();
        } else {
            control.resize(640, 360).unwrap();
        }
        next_format(&mut capture, (640, 360));
        assert_eq!(capture.stats().current_epoch(), index as u64 + 2);
        assert!(context.resolve(&before).is_err());
        let frame = next_frame(&mut capture);
        assert!(frame.pts > before.pts);
        let lease = capture
            .gpu_frame_context()
            .unwrap()
            .resolve(&frame)
            .unwrap();
        color(&lease, [0, 255, 0, 255]);
    }
    capture.stop().unwrap();
}
#[test]
#[ignore = "same-dimension healthy recreation interleaved with explicit resize on WARP"]
fn replacement_same_size_and_resize_warp() {
    same_size_mixed(true);
}
#[test]
#[ignore = "same-dimension healthy recreation interleaved with explicit resize on hardware"]
fn replacement_same_size_and_resize_hardware() {
    same_size_mixed(false);
}
