#![cfg(all(windows, target_arch = "x86_64", feature = "d3d12-bootstrap-fixture"))]
//! Production WGC against an exact owned child. Never captures a monitor/game.
use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
use capture_hook_prototype::d3d12_cross_process::Peer;
use capture_hook_prototype::d3d12_protocol::{WindowAction, Wire};
use capture_hook_prototype::transport::Message;
use capture_windows::WgcDisplayCapture;
use gpu_windows::{BgraReadback, GpuTextureSlot};
use media_types::StreamId;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
struct ProcessSample {
    private_bytes: usize,
    working_set: usize,
    os_peak_working_set: usize,
    handles: u32,
}
fn process_sample(process: windows::Win32::Foundation::HANDLE) -> ProcessSample {
    use windows::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size,
        ..Default::default()
    };
    assert!(
        unsafe {
            K32GetProcessMemoryInfo(
                process,
                (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                size,
            )
        }
        .as_bool(),
        "owned process memory query failed"
    );
    let mut handles = 0;
    unsafe { windows::Win32::System::Threading::GetProcessHandleCount(process, &mut handles) }
        .unwrap();
    ProcessSample {
        private_bytes: counters.PrivateUsage,
        working_set: counters.WorkingSetSize,
        os_peak_working_set: counters.PeakWorkingSetSize,
        handles,
    }
}
impl ProcessSample {
    fn include(&mut self, sample: Self) {
        self.private_bytes = self.private_bytes.max(sample.private_bytes);
        self.working_set = self.working_set.max(sample.working_set);
        self.os_peak_working_set = self.os_peak_working_set.max(sample.os_peak_working_set);
        self.handles = self.handles.max(sample.handles);
    }
}

fn frame(capture: &mut WgcDisplayCapture) -> media_types::VideoFrame {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match capture.next_event().unwrap() {
            VideoCaptureEvent::Frame(frame) => return frame,
            VideoCaptureEvent::FormatChanged { .. } | VideoCaptureEvent::Idle => {}
            VideoCaptureEvent::SourceLost => panic!("owned WGC source lost"),
        }
        assert!(Instant::now() < deadline, "owned WGC frame timeout");
    }
}
fn center(lease: &encoder_api::GpuFrameLease) -> [u8; 4] {
    let pixels = BgraReadback::default()
        .read_scaled(lease.downcast_ref::<GpuTextureSlot>().unwrap(), 64, 36)
        .unwrap();
    pixels[(18 * 64 + 32) * 4..(18 * 64 + 32) * 4 + 4]
        .try_into()
        .unwrap()
}
fn start() -> (Peer, WgcDisplayCapture) {
    let peer = Peer::launch_bootstrap(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        true,
        None,
    )
    .unwrap();
    start_capture(peer)
}
fn start_capture(peer: Peer) -> (Peer, WgcDisplayCapture) {
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing owned greeting")
    };
    peer.approve(hello).unwrap();
    let mut capture = WgcDisplayCapture::new(StreamId(0));
    capture.set_cursor_capture(false).unwrap();
    capture
        .start(VideoCaptureConfig {
            source_id: format!(
                "pid:{}:created:{}:hwnd:{}",
                hello.session.target.pid,
                hello.session.target.creation_time,
                hello.session.target.window
            ),
            target_fps: 60,
        })
        .unwrap();
    (peer, capture)
}
fn action(peer: &Peer, action: WindowAction) {
    peer.window_action(action).unwrap();
    loop {
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Frame(_)) => {}
            Wire::Control(Message::Report { presented, .. }) if presented == action as u64 => break,
            other => panic!("unexpected owned action response: {other:?}"),
        }
    }
}
#[test]
#[ignore = "production WGC on owned occluded HWND; retained pixels under changing content"]
fn wgc_owned_retained_frames_and_terminal_exit() {
    let (mut peer, mut capture) = start();
    let context = capture.gpu_frame_context().unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let (initial, retained) = loop {
        let initial = frame(&mut capture);
        let lease = context.resolve(&initial).unwrap();
        if center(&lease) == [0, 0, 255, 255] {
            break (initial, lease);
        }
        assert!(
            Instant::now() < deadline,
            "no red selected-window content observed"
        );
    };
    assert_eq!(
        center(&retained),
        [0, 0, 255, 255],
        "WGC captured unrelated covering HWND instead of selected red content"
    );
    action(&peer, WindowAction::Restore);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut changed = 0;
    while changed < 20 {
        let new = frame(&mut capture);
        let lease = context.resolve(&new).unwrap();
        if center(&lease) == [255, 0, 255, 255] {
            changed += 1;
        }
        assert_eq!(
            center(&retained),
            [0, 0, 255, 255],
            "encoder-retained WGC frame was overwritten"
        );
        assert!(Instant::now() < deadline);
    }
    std::thread::sleep(Duration::from_millis(150)); // deliberately queue old media
    peer.kill().unwrap();
    let _ = peer.wait_exit(Duration::from_secs(5)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match capture.next_event() {
            Ok(VideoCaptureEvent::SourceLost) | Err(_) => break,
            Ok(VideoCaptureEvent::Idle) => {}
            other => panic!("terminal owned window delivered queued media: {other:?}"),
        }
        assert!(Instant::now() < deadline);
    }
    capture.stop().unwrap();
    assert!(context.resolve(&initial).is_err());
    assert_eq!(center(&retained), [0, 0, 255, 255]);
}

#[test]
#[ignore = "production WGC drop pressure while all eight staging surfaces are retained"]
fn wgc_owned_pool_saturation_does_not_overwrite() {
    let (peer, mut capture) = start();
    let context = capture.gpu_frame_context().unwrap();
    let mut leases = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    while leases.len() < 8 {
        let frame = frame(&mut capture);
        let lease = context.resolve(&frame).unwrap();
        // WGC may initially publish transparent startup content. Saturate using
        // eight observed red publications, not an assumed first-frame color.
        if center(&lease) == [0, 0, 255, 255] {
            leases.push(lease);
        }
        assert!(
            Instant::now() < deadline,
            "could not retain eight red WGC publications"
        );
    }
    action(&peer, WindowAction::Restore);
    let started = Instant::now();
    assert!(matches!(
        capture.next_event().unwrap(),
        VideoCaptureEvent::Idle
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
    for lease in &leases {
        assert_eq!(center(lease), [0, 0, 255, 255]);
    }
    leases.pop();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let new = frame(&mut capture);
        let lease = context.resolve(&new).unwrap();
        if center(&lease) == [255, 0, 255, 255] {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    capture.stop().unwrap();
}

#[test]
#[ignore = "production selection rejects missing or stale process lifetime without monitor fallback"]
fn wgc_owned_stale_identity_never_starts_backend() {
    let peer = Peer::launch_bootstrap(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        true,
        None,
    )
    .unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing greeting")
    };
    let target = hello.session.target;
    let canonical = format!(
        "pid:{}:created:{}:hwnd:{}",
        target.pid, target.creation_time, target.window
    );
    assert!(capture_windows::enumerate_windows()
        .unwrap()
        .iter()
        .any(|window| window.id == canonical));
    for source in [
        format!("hwnd:{}", target.window),
        format!("pid:{}:hwnd:{}", target.pid, target.window),
        format!(
            "pid:{}:created:{}:hwnd:{}",
            target.pid,
            target.creation_time + 1,
            target.window
        ),
    ] {
        let mut capture = capture_windows::DisplayCapture::new(StreamId(0));
        assert!(capture
            .start(VideoCaptureConfig {
                source_id: format!("game:{source}"),
                target_fps: 60
            })
            .is_err());
        assert!(capture.backend_name().is_none());
        assert!(capture.gpu_frame_context().is_none());
        capture.stop().unwrap();
    }
}

#[test]
#[ignore = "owned application sets capture exclusion after WGC has queued frames"]
fn wgc_owned_dynamic_exclusion_discards_queued_frames() {
    let (peer, mut capture) = start();
    let _ = frame(&mut capture);
    let context = capture.gpu_frame_context().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    action(&peer, WindowAction::ExcludeCapture);
    assert!(matches!(
        capture.next_event(),
        Err(capture_api::VideoCaptureError::PermissionDenied)
    ));
    assert!(capture.gpu_frame_context().is_none());
    assert!(context.session_resource().is_none());
    capture.stop().unwrap();
}

#[test]
#[ignore = "production WGC resize preserves delivered resolver gap and enforces retired surface budget"]
fn wgc_owned_resize_and_retained_budget() {
    let (peer, mut capture) = start();
    let context = capture.gpu_frame_context().unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let (old, retained) = loop {
        let old = frame(&mut capture);
        let lease = context.resolve(&old).unwrap();
        if center(&lease) == [0, 0, 255, 255] {
            break (old, lease);
        }
        assert!(Instant::now() < deadline);
    };
    action(&peer, WindowAction::Borderless);
    // A format callback must not replace the resolver while this delivered frame
    // is still in the caller's encode/resolve gap.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(center(&context.resolve(&old).unwrap()), [0, 0, 255, 255]);
    let deadline = Instant::now() + Duration::from_secs(8);
    let dimensions = loop {
        match capture.next_event().unwrap() {
            VideoCaptureEvent::FormatChanged { width, height }
                if width > old.width && height > old.height =>
            {
                break (width, height)
            }
            VideoCaptureEvent::Frame(_) | VideoCaptureEvent::Idle => {}
            other => panic!("unexpected resize event: {other:?}"),
        }
        assert!(Instant::now() < deadline);
    };
    assert!(context.resolve(&old).is_err());
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let new = frame(&mut capture);
        assert_eq!((new.width, new.height), dimensions);
        // The compositor can emit transparent transitional frames after resize,
        // just as during startup. Require observed new content before proceeding.
        if center(&context.resolve(&new).unwrap()) == [0, 255, 255, 255] {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no yellow content observed after WGC resize"
        );
    }
    assert_eq!(center(&retained), [0, 0, 255, 255]);
    // One retained old surface plus the active eight leaves fewer than eight
    // slots for a third pool. Fail rather than exceed sixteen live surfaces.
    action(&peer, WindowAction::Windowed);
    let deadline = Instant::now() + Duration::from_secs(8);
    let error = loop {
        match capture.next_event() {
            Err(error) => break error,
            Ok(VideoCaptureEvent::Frame(_) | VideoCaptureEvent::Idle) => {}
            other => panic!("budget failure published another format: {other:?}"),
        }
        assert!(Instant::now() < deadline);
    };
    assert!(error.to_string().contains("sixteen-surface"), "{error}");
    capture.stop().unwrap();
    assert_eq!(center(&retained), [0, 0, 255, 255]);
}

#[test]
#[ignore = "production WGC minimized source yields control without monitor fallback, then resumes"]
fn wgc_owned_minimize_idle_restore() {
    let (peer, mut capture) = start();
    let _ = frame(&mut capture);
    action(&peer, WindowAction::Minimize);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        match capture.next_event().unwrap() {
            VideoCaptureEvent::Idle => break,
            VideoCaptureEvent::Frame(_) | VideoCaptureEvent::FormatChanged { .. } => {}
            other => panic!("minimized live HWND was treated as lost: {other:?}"),
        }
        assert!(Instant::now() < deadline);
    }
    action(&peer, WindowAction::Restore);
    let context = capture.gpu_frame_context().unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let frame = frame(&mut capture);
        if center(&context.resolve(&frame).unwrap()) == [255, 0, 255, 255] {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    capture.stop().unwrap();
}

#[test]
#[ignore = "sixty seconds of production WGC while an old frame remains retained"]
fn wgc_owned_retained_surface_soak() {
    let seconds = std::env::var("SILK_WGC_SOAK_SECONDS")
        .map(|value| value.parse::<u32>().expect("invalid WGC soak duration"))
        .unwrap_or(60);
    assert!((60..=28_800).contains(&seconds));
    // Endurance may use a preserved build snapshot so unrelated development
    // builds cannot replace or lock the running fixture artifacts. Peer still
    // requires the exact owned fixture basename and launches it itself.
    let fixture = std::env::var_os("SILK_WGC_FIXTURE_EXE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture").into());
    let peer = Peer::launch_endurance(&fixture, true, seconds).unwrap();
    let (peer, mut capture) = start_capture(peer);
    let context = capture.gpu_frame_context().unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let retained = loop {
        let old = frame(&mut capture);
        let lease = context.resolve(&old).unwrap();
        if center(&lease) == [0, 0, 255, 255] {
            break lease;
        }
        assert!(Instant::now() < deadline);
    };
    action(&peer, WindowAction::Restore);
    let started = Instant::now();
    let mut frames = 0;
    let mut new_color = 0;
    let mut previous_pts = 0;
    let own_process = unsafe { windows::Win32::System::Threading::GetCurrentProcess() };
    let baseline = [process_sample(own_process), process_sample(peer.handle())];
    let mut sampled_peaks = baseline;
    println!(
        "WGC resource baseline: parent={:?}; owned fixture={:?}",
        baseline[0], baseline[1]
    );
    let mut next_progress = Duration::from_secs(60);
    while started.elapsed() < Duration::from_secs(u64::from(seconds)) {
        if started.elapsed() >= next_progress {
            let current = [process_sample(own_process), process_sample(peer.handle())];
            sampled_peaks[0].include(current[0]);
            sampled_peaks[1].include(current[1]);
            println!("WGC owned endurance progress: {} seconds, {frames} frames, {new_color} fresh checks; parent={:?}; owned fixture={:?}",started.elapsed().as_secs(), current[0], current[1]);
            next_progress += Duration::from_secs(60);
        }
        match capture.next_event().unwrap() {
            VideoCaptureEvent::Frame(frame) => {
                assert!(frame.pts >= previous_pts);
                previous_pts = frame.pts;
                frames += 1;
                if frames % 25 == 0 {
                    assert_eq!(center(&retained), [0, 0, 255, 255]);
                    let lease = context.resolve(&frame).unwrap();
                    if center(&lease) == [255, 0, 255, 255] {
                        new_color += 1;
                    }
                }
            }
            VideoCaptureEvent::Idle | VideoCaptureEvent::FormatChanged { .. } => {}
            other => panic!("owned WGC soak lost source: {other:?}"),
        }
    }
    assert!(
        frames >= seconds * 10,
        "too few production WGC frames: {frames}"
    );
    assert!(new_color >= 10);
    capture.stop().unwrap();
    assert_eq!(center(&retained), [0, 0, 255, 255]);
    sampled_peaks[0].include(process_sample(own_process));
    sampled_peaks[1].include(process_sample(peer.handle()));
    println!("WGC resource sampled peaks: parent={:?}; owned fixture={:?}; one-minute samples, not a GPU-memory or throughput claim", sampled_peaks[0], sampled_peaks[1]);
    println!("PASS: production WGC retained-surface soak: {frames} frames in {:.1}s, {new_color} fresh magenta checks, original red pixels retained through stop; no monitor capture.",started.elapsed().as_secs_f64());
}

#[test]
#[ignore = "production WGC retirement budget survives stop/start and can recover after leases are released"]
fn wgc_owned_restart_preserves_retired_budget() {
    let (peer, mut capture) = start();
    let source = capture_windows::enumerate_windows()
        .unwrap()
        .into_iter()
        .find(|window| {
            window.id.starts_with(&format!("pid:{}:", peer.pid()))
                && window.title == "Silk owned D3D12 selected fixture"
        })
        .expect("missing exact owned selected HWND")
        .id;
    let config = VideoCaptureConfig {
        source_id: source,
        target_fps: 60,
    };
    let mut retained = Vec::new();
    for epoch in 0..2 {
        if epoch > 0 {
            capture.start(config.clone()).unwrap();
        }
        let context = capture.gpu_frame_context().unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while retained.len() < (epoch + 1) * 8 {
            let frame = frame(&mut capture);
            let lease = context.resolve(&frame).unwrap();
            if center(&lease) == [0, 0, 255, 255] {
                retained.push(lease);
            }
            assert!(Instant::now() < deadline);
        }
        capture.stop().unwrap();
        assert!(context.session_resource().is_none());
    }
    capture.start(config.clone()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let error = loop {
        match capture.next_event() {
            Err(error) => break error,
            Ok(VideoCaptureEvent::Idle) => {}
            other => panic!("third session exceeded retained sixteen-surface budget: {other:?}"),
        }
        assert!(Instant::now() < deadline);
    };
    assert!(error.to_string().contains("sixteen-surface"), "{error}");
    capture.stop().unwrap();
    for lease in &retained {
        assert_eq!(center(lease), [0, 0, 255, 255]);
    }
    drop(retained);
    capture.start(config).unwrap();
    let _ = frame(&mut capture);
    capture.stop().unwrap();
}

#[test]
#[ignore = "production WGC full queued epochs preserve the resize format boundary"]
fn wgc_owned_queued_resize_reserves_format_capacity() {
    let (peer, mut capture) = start();
    // Do not consume even the initial format: the old epoch fills its slots.
    std::thread::sleep(Duration::from_millis(200));
    action(&peer, WindowAction::Borderless);
    std::thread::sleep(Duration::from_millis(200));
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut formats = 0;
    let mut resized = false;
    loop {
        match capture.next_event().unwrap() {
            VideoCaptureEvent::FormatChanged { width, height } => {
                formats += 1;
                resized = width > 500 && height > 500;
            }
            VideoCaptureEvent::Frame(frame) if resized => {
                let lease = capture
                    .gpu_frame_context()
                    .unwrap()
                    .resolve(&frame)
                    .unwrap();
                if center(&lease) == [0, 255, 255, 255] {
                    break;
                }
            }
            VideoCaptureEvent::Frame(_) | VideoCaptureEvent::Idle => {}
            other => panic!("full queued resize failed: {other:?}"),
        }
        assert!(Instant::now() < deadline);
    }
    assert!(formats >= 2);
    capture.stop().unwrap();
}
