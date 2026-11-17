#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "d3d12-remote-helper-fixture"
))]
use capture_hook_prototype::d3d12_cross_process::{Peer, RemoteConsumer};
use capture_hook_prototype::d3d12_protocol::Wire;
use capture_hook_prototype::local_gpu_bridge::GpuPoolBudget;
use capture_hook_prototype::owned_attachment::attach_owned_d3d12_fixture;
use capture_hook_prototype::transport::Message;
use gpu_windows::{BgraReadback, GpuTextureSlot};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn pixels(slot: &GpuTextureSlot) {
    let bytes = BgraReadback::default().read_scaled(slot, 64, 36).unwrap();
    assert!(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == [0, 0, 255, 255]));
}
#[cfg(feature = "d3d12-observed-loss-fixture")]
#[test]
#[ignore = "independent hardware helper observes actual owner loss and retires before normal completion"]
fn independent_helper_observed_loss_stop_hardware() {
    observed_helper_loss(0);
}
#[cfg(feature = "d3d12-observed-loss-fixture")]
#[test]
#[ignore = "independent helper rejects wrong loss retirement proof without releasing owner graph"]
fn independent_helper_observed_loss_wrong_proof_hardware() {
    observed_helper_loss(1);
}
#[cfg(feature = "d3d12-observed-loss-fixture")]
#[test]
#[ignore = "independent helper stops boundedly while removed owner waits for release"]
fn independent_helper_observed_loss_stop_barrier_hardware() {
    observed_helper_loss(2);
}
#[cfg(feature = "d3d12-observed-loss-fixture")]
fn observed_helper_loss(scenario: u8) {
    use capture_hook_prototype::d3d12_cross_process::RemoteCopyOutcome;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    let peer =
        Peer::launch_observed_loss(Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")))
            .unwrap();
    let Wire::Control(Message::Select(session)) = peer.receive(Duration::from_secs(10)).unwrap()
    else {
        panic!("missing owner identity")
    };
    let mut worker = attach_owned_d3d12_fixture(&peer, session).unwrap();
    assert!(matches!(
        peer.receive(Duration::from_secs(10)).unwrap(),
        Wire::Control(Message::Report { .. })
    ));
    peer.send(Message::Select(session)).unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing helper offer")
    };
    let mut consumer =
        RemoteConsumer::open(&peer, hello, false, Arc::new(GpuPoolBudget::default())).unwrap();
    peer.approve(hello).unwrap();
    let mut retained = None;
    for _ in 0..20 {
        let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
        else {
            panic!("missing helper frame")
        };
        let (publication, ack) = consumer.copy(header).unwrap();
        if retained.is_none() {
            retained = publication;
        }
        peer.send(ack).unwrap();
    }
    let retained = retained.unwrap();
    pixels(retained.surface());
    unsafe {
        PostMessageW(
            Some(HWND(session.target.window as usize as *mut _)),
            capture_hook_prototype::d3d12_present_fixture::OWNED_HELPER_LOSS_MESSAGE,
            WPARAM(0),
            LPARAM(0),
        )
    }
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline);
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::DeviceLost(notice) => {
                assert_eq!(notice.session, session);
                break;
            }
            Wire::Control(Message::Frame(header)) => {
                match consumer.copy_if_preserving_loss(header, true).unwrap() {
                    RemoteCopyOutcome::Copied(_, ack) => peer.send(ack).unwrap(),
                    RemoteCopyOutcome::ProducerLost => {}
                }
            }
            other => panic!("unexpected observed loss packet: {other:?}"),
        }
    }
    consumer.retire_epoch_after_producer_loss().unwrap();
    drop(consumer);
    if scenario == 1 {
        peer.retired_epoch(capture_hook_prototype::SessionIdentity {
            epoch: 3,
            ..session
        })
        .unwrap();
        let error = worker.finish().expect_err("wrong loss proof accepted");
        assert!(error.contains("status=1"), "{error}");
        assert!(peer.is_running());
        pixels(retained.surface());
        return;
    }
    if scenario == 2 {
        let started = Instant::now();
        peer.send(Message::Stop).unwrap();
        assert_eq!(
            peer.receive(Duration::from_secs(5)).unwrap(),
            Wire::Control(Message::Bye)
        );
        worker.finish().unwrap();
        // Without the retirement proof the helper conservatively retains old
        // media after its bounded drain attempt; this is not the healthy fast stop.
        assert!(started.elapsed() < Duration::from_secs(6));
        assert!(peer.is_running());
        pixels(retained.surface());
        return;
    }
    peer.retired_epoch(capture_hook_prototype::SessionIdentity {
        epoch: 2,
        ..session
    })
    .unwrap();
    assert_eq!(
        peer.receive(Duration::from_secs(5)).unwrap(),
        Wire::Control(Message::Bye)
    );
    worker.finish().unwrap();
    assert!(peer.is_running());
    pixels(retained.surface());
    use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
    let mut capture = capture_windows::WgcDisplayCapture::new(media_types::StreamId(0));
    capture
        .start(VideoCaptureConfig {
            source_id: format!(
                "pid:{}:created:{}:hwnd:{}",
                session.target.pid, session.target.creation_time, session.target.window
            ),
            target_fps: 30,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut green = 0;
    while green < 12 {
        assert!(
            Instant::now() < deadline,
            "owner failed to resume fresh green rendering after helper loss-stop"
        );
        if let VideoCaptureEvent::Frame(frame) = capture.next_event().unwrap() {
            let lease = capture
                .gpu_frame_context()
                .unwrap()
                .resolve(&frame)
                .unwrap();
            let bytes = BgraReadback::default()
                .read_scaled(lease.downcast_ref::<GpuTextureSlot>().unwrap(), 64, 36)
                .unwrap();
            if bytes.as_chunks::<4>().0[18 * 64 + 32] == [0, 255, 0, 255] {
                green += 1;
            }
        }
    }
    capture.stop().unwrap();
    pixels(retained.surface());
    assert!(peer.is_running());
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
fn automatic_resize(warp: bool, scenario: u8) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
    let peer = Peer::launch_automatic_resize(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        warp,
    )
    .unwrap();
    let Wire::Control(Message::Select(session)) = peer.receive(Duration::from_secs(10)).unwrap()
    else {
        panic!("missing identity")
    };
    assert!(matches!(
        peer.receive(Duration::from_secs(5)).unwrap(),
        Wire::Control(Message::Report {
            presented: 12..,
            published: 0,
            dropped: 0
        })
    ));
    let mut worker = attach_owned_d3d12_fixture(&peer, session).unwrap();
    assert!(matches!(
        peer.receive(Duration::from_secs(10)).unwrap(),
        Wire::Control(Message::Report {
            presented: 0,
            published: 0,
            dropped: 0
        })
    ));
    peer.send(Message::Select(session)).unwrap();
    let Wire::Hello(mut hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing hello")
    };
    let budget = Arc::new(GpuPoolBudget::default());
    let mut consumer = RemoteConsumer::open(&peer, hello, warp, budget.clone()).unwrap();
    peer.approve(hello).unwrap();
    let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing first frame")
    };
    let (retained, ack) = consumer.copy(header).unwrap();
    let retained = retained.unwrap();
    pixels(retained.surface());
    peer.send(ack).unwrap();
    if scenario == 5 || scenario == 6 {
        unsafe {
            PostMessageW(
                Some(HWND(session.target.window as usize as *mut _)),
                capture_hook_prototype::d3d12_present_fixture::OWNED_RESIZE_FAULT_MESSAGE,
                WPARAM(if scenario == 5 { 1 } else { 2 }),
                LPARAM(0),
            )
            .unwrap();
        }
        let error = worker
            .finish()
            .expect_err("unsupported/failed actual resize admitted capture");
        assert!(error.contains("status=1"), "{error}");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(Instant::now() < deadline);
            match peer.receive(Duration::from_millis(100)) {
                Ok(Wire::Control(Message::Frame(header))) => {
                    assert_eq!(header.session, hello.session)
                }
                Err(_) => break,
                other => panic!("unsupported/failed resize offered fresh media: {other:?}"),
            }
        }
        assert!(peer.is_running());
        pixels(retained.surface());
        return;
    }
    let epochs = if scenario == 7 { 24 } else { 3 };
    for (index, (width, height)) in [(640, 360), (320, 180), (800, 450)]
        .into_iter()
        .cycle()
        .take(epochs)
        .enumerate()
    {
        if scenario == 7 && index % 4 == 0 {
            use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_MINIMIZE, SW_RESTORE};
            let window = HWND(session.target.window as usize as *mut _);
            unsafe {
                let _ = ShowWindow(window, SW_MINIMIZE);
            }
            std::thread::sleep(Duration::from_millis(30));
            unsafe {
                let _ = ShowWindow(window, SW_RESTORE);
            }
        }
        unsafe {
            PostMessageW(
                Some(HWND(session.target.window as usize as *mut _)),
                capture_hook_prototype::d3d12_present_fixture::OWNED_GEOMETRY_MESSAGE,
                WPARAM(width as usize),
                LPARAM(height as isize),
            )
            .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut announced = false;
        let mut retired = false;
        loop {
            assert!(Instant::now() < deadline, "automatic epoch deadline");
            match peer.receive(Duration::from_secs(5)).unwrap() {
                Wire::Control(Message::Frame(header)) => {
                    let (_, ack) = consumer.copy(header).unwrap();
                    peer.send(ack).unwrap();
                }
                Wire::Resize(request) => {
                    assert!(!announced);
                    assert_eq!(request.session, hello.session);
                    assert_eq!((request.width, request.height), (width, height));
                    announced = true;
                }
                Wire::Retire(next) => {
                    assert!(announced && !retired);
                    assert_eq!(next.epoch, hello.session.epoch + 1);
                    if scenario == 4 {
                        // Hold retirement open while the application performs
                        // another real resize; no overlapping epoch may reopen.
                        unsafe {
                            PostMessageW(Some(HWND(session.target.window as usize as *mut _)), capture_hook_prototype::d3d12_present_fixture::OWNED_GEOMETRY_MESSAGE, WPARAM(800), LPARAM(450)).unwrap();
                        }
                        let error = worker
                            .finish()
                            .expect_err("overlapping automatic resize accepted");
                        assert!(error.contains("status=1"), "{error}");
                        assert!(peer.receive(Duration::from_millis(100)).is_err());
                        assert!(peer.is_running());
                        pixels(retained.surface());
                        return;
                    }
                    if scenario == 1 {
                        peer.send(Message::Stop).unwrap();
                        let until = Instant::now() + Duration::from_secs(5);
                        loop {
                            assert!(Instant::now() < until);
                            match peer.receive(Duration::from_secs(5)).unwrap() {
                                Wire::Control(Message::Bye) => break,
                                Wire::Control(Message::Report { .. }) => {}
                                other => panic!("media admitted during retirement stop: {other:?}"),
                            }
                        }
                        worker.finish().unwrap();
                        assert!(peer.is_running());
                        pixels(retained.surface());
                        return;
                    }
                    consumer.retire_epoch().unwrap();
                    peer.retired_epoch(next).unwrap();
                    retired = true;
                }
                Wire::Hello(next) => {
                    assert!(retired);
                    assert_eq!((next.width, next.height), (width, height));
                    assert_eq!(next.session.target, session.target);
                    assert_eq!(next.session.epoch, hello.session.epoch + 1);
                    if scenario == 3 {
                        peer.send(Message::Stop).unwrap();
                        let until = Instant::now() + Duration::from_secs(5);
                        loop {
                            assert!(Instant::now() < until);
                            match peer.receive(Duration::from_secs(5)).unwrap() {
                                Wire::Control(Message::Bye) => break,
                                Wire::Control(Message::Report { .. }) => {}
                                other => panic!("media admitted before approval: {other:?}"),
                            }
                        }
                        worker.finish().unwrap();
                        assert!(peer.is_running());
                        pixels(retained.surface());
                        return;
                    }
                    if scenario == 2 {
                        peer.send(Message::Select(hello.session)).unwrap();
                        let error = worker
                            .finish()
                            .expect_err("stale automatic approval accepted");
                        assert!(error.contains("status=1"), "{error}");
                        assert!(peer.receive(Duration::from_millis(100)).is_err());
                        assert!(peer.is_running());
                        pixels(retained.surface());
                        return;
                    }
                    drop(consumer);
                    consumer = RemoteConsumer::open(&peer, next, warp, budget.clone()).unwrap();
                    peer.approve(next).unwrap();
                    hello = next;
                    break;
                }
                other => panic!("unexpected automatic event: {other:?}"),
            }
        }
        for _ in 0..5 {
            let Wire::Control(Message::Frame(header)) =
                peer.receive(Duration::from_secs(5)).unwrap()
            else {
                panic!("missing resized frame")
            };
            assert_eq!((header.width, header.height), (width, height));
            let (publication, ack) = consumer.copy(header).unwrap();
            let bytes = BgraReadback::default()
                .read_scaled(publication.unwrap().surface(), 64, 36)
                .unwrap();
            assert!(bytes
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [0, 255, 0, 255]));
            peer.send(ack).unwrap();
        }
        pixels(retained.surface());
    }
    consumer.close();
    peer.send(Message::Stop).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline);
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Bye) => break,
            Wire::Control(Message::Frame(_) | Message::Report { .. }) => {}
            other => panic!("unexpected stop: {other:?}"),
        }
    }
    worker.finish().unwrap();
    assert!(peer.is_running());
    pixels(retained.surface());
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "owned observed resize across three real API epochs"]
fn attachment_automatic_resize_warp() {
    automatic_resize(true, 0);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "owned hardware observed resize across three real API epochs"]
fn attachment_automatic_resize_hardware() {
    automatic_resize(false, 0);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "bounded stop while automatic epoch awaits retirement"]
fn attachment_automatic_resize_stop_retirement_warp() {
    automatic_resize(true, 1);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "stale automatic epoch approval must fail without new media"]
fn attachment_automatic_resize_stale_approval_warp() {
    automatic_resize(true, 2);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "bounded stop while fresh automatic GPU Hello awaits approval"]
fn attachment_automatic_resize_stop_approval_warp() {
    automatic_resize(true, 3);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "second real resize during held retirement closes capture without reopening"]
fn attachment_automatic_resize_overlap_warp() {
    automatic_resize(true, 4);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "actual failed ResizeBuffers1 must close capture while renderer survives"]
fn attachment_automatic_resize_api_failure_warp() {
    automatic_resize(true, 5);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "actual successful odd-width resize is unsupported and must not admit media"]
fn attachment_automatic_resize_unsupported_dimensions_warp() {
    automatic_resize(true, 6);
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
#[test]
#[ignore = "24 observed epochs with retained pixels and six owned minimize/restore cycles"]
fn attachment_automatic_resize_repeated_window_cycles_warp() {
    automatic_resize(true, 7);
}
fn lifecycle(warp: bool) {
    lifecycle_factory(warp, false);
}
fn lifecycle_factory(warp: bool, legacy: bool) {
    lifecycle_duration(warp, legacy, Duration::ZERO);
}
fn lifecycle_duration(warp: bool, legacy: bool, minimum_duration: Duration) {
    lifecycle_mode(warp, legacy, minimum_duration, false);
}
fn lifecycle_mode(warp: bool, legacy: bool, minimum_duration: Duration, late: bool) {
    let mut peer = if late {
        #[cfg(feature = "d3d12-late-resize-fixture")]
        {
            Peer::launch_late_resize(
                Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
                warp,
            )
            .unwrap()
        }
        #[cfg(not(feature = "d3d12-late-resize-fixture"))]
        {
            panic!("late fixture feature required")
        }
    } else {
        Peer::launch_passive_factory(
            Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
            warp,
            legacy,
        )
        .unwrap()
    };
    let Wire::Control(Message::Select(session)) = peer.receive(Duration::from_secs(10)).unwrap()
    else {
        panic!("missing passive identity")
    };
    if late {
        let Wire::Control(Message::Report {
            presented,
            published: 0,
            dropped: 0,
        }) = peer.receive(Duration::from_secs(5)).unwrap()
        else {
            panic!("missing pre-load renderer proof")
        };
        assert!(
            presented >= 12,
            "target must already render before the helper loads"
        );
    }
    let mut stale = session;
    stale.target.creation_time += 1;
    assert!(attach_owned_d3d12_fixture(&peer, stale).is_err());
    let mut worker = attach_owned_d3d12_fixture(&peer, session).unwrap();
    assert!(
        attach_owned_d3d12_fixture(&peer, session).is_err(),
        "duplicate remote attempt accepted"
    );
    assert_eq!(
        peer.receive(Duration::from_secs(10)).unwrap(),
        Wire::Control(Message::Report {
            presented: 0,
            published: 0,
            dropped: 0
        })
    );
    // Early mode approves creation; late mode approves an authoritative resize
    // of the already-created chain. Neither sends a renderer COM pointer.
    peer.send(Message::Select(session)).unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing remote helper hello")
    };
    assert_eq!(hello.session, session);
    assert_eq!((hello.width, hello.height), (318, 178));
    let mut consumer =
        RemoteConsumer::open(&peer, hello, warp, Arc::new(GpuPoolBudget::default())).unwrap();
    peer.approve(hello).unwrap();
    let mut retained: Option<
        capture_hook_prototype::publication_pool::Publication<GpuTextureSlot>,
    > = None;
    let mut previous = 0;
    let started = Instant::now();
    let mut frames = 0;
    while frames < 20 || started.elapsed() < minimum_duration {
        let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
        else {
            panic!("missing attached frame")
        };
        assert!(header.qpc_ticks > previous);
        previous = header.qpc_ticks;
        let (publication, ack) = consumer.copy(header).unwrap();
        let publication = publication.unwrap();
        frames += 1;
        if frames <= 20 || frames % 25 == 0 {
            pixels(publication.surface());
            if let Some(retained) = retained.as_ref() {
                pixels(retained.surface());
            }
        }
        if retained.is_none() {
            retained = Some(publication);
        }
        peer.send(ack).unwrap();
    }
    pixels(retained.as_ref().unwrap().surface());
    if late {
        peer.send(Message::Render { count: 1, color: 2 }).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match peer.receive(Duration::from_secs(5)).unwrap() {
                Wire::Control(Message::Report {
                    presented: 0,
                    published: 0,
                    dropped: 0,
                }) => break,
                Wire::Control(Message::Frame(header)) => {
                    let (_, ack) = consumer.copy(header).unwrap();
                    peer.send(ack).unwrap();
                }
                other => panic!("late discovery incorrectly used a new creation: {other:?}"),
            }
            assert!(
                Instant::now() < deadline,
                "missing late discovery provenance report"
            );
        }
    }
    consumer.close();
    peer.send(Message::Stop).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Bye) => break,
            Wire::Control(Message::Frame(_) | Message::Report { .. }) => {}
            other => panic!("unexpected attached stop event: {other:?}"),
        }
        assert!(Instant::now() < deadline);
    }
    worker.finish().unwrap();
    assert!(
        peer.is_running(),
        "stopping capture terminated the owned renderer"
    );
    pixels(retained.as_ref().unwrap().surface());
    peer.kill().unwrap();
    let _ = peer.wait_exit(Duration::from_secs(5)).unwrap();
    pixels(retained.as_ref().unwrap().surface());
    if !minimum_duration.is_zero() {
        assert!(
            frames >= 600,
            "too few remote helper publications: {frames}"
        );
    }
    let discovery = if late {
        "already-rendering chain; authoritative later ResizeBuffers1; zero new factory creations"
    } else {
        "early factory handshake"
    };
    println!("PASS: actual normal DLL load and remote D3D12 worker on {} ({} factory): exact passive child/lifetime/HWND, {discovery}, {frames} GPU publications over {:.1}s, retained pixels and worker stop without terminating renderer; no arbitrary process/game attachment or throughput claim.",if warp {"WARP"} else {"hardware"},if legacy {"legacy"} else {"modern"},started.elapsed().as_secs_f64());
}

#[cfg(feature = "d3d12-late-resize-fixture")]
fn rejected_late_scenario(
    scenario: capture_hook_prototype::d3d12_cross_process::LateResizeScenario,
) {
    let peer = Peer::launch_late_resize_scenario(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        true,
        scenario,
    )
    .unwrap();
    let Wire::Control(Message::Select(session)) = peer.receive(Duration::from_secs(10)).unwrap()
    else {
        panic!("missing late identity")
    };
    let Wire::Control(Message::Report {
        presented,
        published: 0,
        dropped: 0,
    }) = peer.receive(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing existing renderer proof")
    };
    assert!(presented >= 12);
    let mut worker = attach_owned_d3d12_fixture(&peer, session).unwrap();
    assert_eq!(
        peer.receive(Duration::from_secs(10)).unwrap(),
        Wire::Control(Message::Report {
            presented: 0,
            published: 0,
            dropped: 0
        })
    );
    peer.send(Message::Select(session)).unwrap();
    // No media/greeting may be invented while a usable authoritative queue is
    // absent. Give the bounded discovery deadline time to expire before finish.
    assert!(peer.receive(Duration::from_secs(1)).is_err());
    let error = worker
        .finish()
        .expect_err("unsupported late scenario started capture");
    assert!(
        error.contains("status=1"),
        "worker did not return its terminal status: {error}"
    );
    assert!(
        peer.receive(Duration::from_millis(100)).is_err(),
        "late scenario admitted unproven media"
    );
    assert!(
        peer.is_running(),
        "observer failure terminated the owned target"
    );
}
#[cfg(feature = "d3d12-late-resize-fixture")]
#[test]
#[ignore = "already-running chain without a later explicit queue call must not infer a queue"]
fn attachment_late_without_resize_rejected_warp() {
    rejected_late_scenario(
        capture_hook_prototype::d3d12_cross_process::LateResizeScenario::NoResize,
    );
}
#[cfg(feature = "d3d12-late-resize-fixture")]
#[test]
#[ignore = "actual successful ResizeBuffers1 with distinct same-device DIRECT queues is ambiguous"]
fn attachment_late_distinct_queues_rejected_warp() {
    rejected_late_scenario(
        capture_hook_prototype::d3d12_cross_process::LateResizeScenario::DistinctQueues,
    );
}
#[test]
#[ignore = "loads the explicitly built d3d12-helper sibling DLL into exact passive owned WARP child"]
fn attachment_lifecycle_warp() {
    lifecycle(true);
}
#[test]
#[ignore = "loads the explicitly built d3d12-helper sibling DLL into exact passive owned hardware child"]
fn attachment_lifecycle_hardware() {
    lifecycle(false);
}
#[cfg(feature = "d3d12-late-resize-fixture")]
#[test]
#[ignore = "already-rendering WARP chain discovered through actual later ResizeBuffers1 queues"]
fn attachment_late_resize_warp() {
    lifecycle_mode(true, false, Duration::ZERO, true);
}
#[cfg(feature = "d3d12-late-resize-fixture")]
#[test]
#[ignore = "already-rendering hardware chain discovered through actual later ResizeBuffers1 queues"]
fn attachment_late_resize_hardware() {
    lifecycle_mode(false, false, Duration::ZERO, true);
}
#[test]
#[ignore = "remote DLL observes legacy CreateSwapChain in exact passive owned WARP child"]
fn attachment_legacy_lifecycle_warp() {
    lifecycle_factory(true, true);
}
#[test]
#[ignore = "remote DLL observes legacy CreateSwapChain in exact passive owned hardware child"]
fn attachment_legacy_lifecycle_hardware() {
    lifecycle_factory(false, true);
}
#[test]
#[ignore = "sixty seconds of loaded remote D3D12 helper with retained local pixels, WARP modern factory"]
fn attachment_soak_warp() {
    lifecycle_duration(true, false, Duration::from_secs(60));
}
#[test]
#[ignore = "sixty seconds of loaded remote D3D12 helper with retained local pixels, hardware legacy factory"]
fn attachment_soak_hardware() {
    lifecycle_duration(false, true, Duration::from_secs(60));
}

#[test]
#[ignore = "integrated/non-passive owned target cannot be passed to remote loader"]
fn attachment_rejects_non_passive_warp() {
    let peer = Peer::launch_bootstrap(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        true,
        None,
    )
    .unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing integrated greeting")
    };
    assert!(attach_owned_d3d12_fixture(&peer, hello.session).is_err());
}

#[test]
#[ignore = "wrong initial media approval stops helper with no admitted frames and leaves renderer alive"]
fn attachment_wrong_approval_is_terminal_warp() {
    let peer = Peer::launch_passive(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        true,
    )
    .unwrap();
    let Wire::Control(Message::Select(session)) = peer.receive(Duration::from_secs(10)).unwrap()
    else {
        panic!("missing passive identity")
    };
    let mut worker = attach_owned_d3d12_fixture(&peer, session).unwrap();
    assert_eq!(
        peer.receive(Duration::from_secs(10)).unwrap(),
        Wire::Control(Message::Report {
            presented: 0,
            published: 0,
            dropped: 0
        })
    );
    peer.send(Message::Select(session)).unwrap();
    let Wire::Hello(_) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing attached hello")
    };
    let mut wrong = session;
    wrong.epoch += 1;
    peer.send(Message::Select(wrong)).unwrap();
    assert!(worker.finish().is_err());
    assert!(peer.is_running());
    assert!(
        peer.receive(Duration::from_millis(100)).is_err(),
        "unapproved media was admitted"
    );
    assert!(
        attach_owned_d3d12_fixture(&peer, session).is_err(),
        "failed worker allowed a retry in the same process"
    );
}

#[test]
#[ignore = "passive owner rejects a mismatched creation approval before chain creation"]
fn attachment_wrong_creation_approval_warp() {
    let mut peer = Peer::launch_passive(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        true,
    )
    .unwrap();
    let Wire::Control(Message::Select(mut session)) =
        peer.receive(Duration::from_secs(10)).unwrap()
    else {
        panic!("missing passive identity")
    };
    session.target.creation_time += 1;
    peer.send(Message::Select(session)).unwrap();
    assert!(!peer.wait_exit(Duration::from_secs(5)).unwrap());
}

#[test]
#[ignore = "owned target exit after remote factory readiness but before any chain creation"]
fn attachment_target_exit_before_creation_warp() {
    let mut peer = Peer::launch_passive(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        true,
    )
    .unwrap();
    let Wire::Control(Message::Select(session)) = peer.receive(Duration::from_secs(10)).unwrap()
    else {
        panic!("missing passive identity")
    };
    let mut worker = attach_owned_d3d12_fixture(&peer, session).unwrap();
    assert_eq!(
        peer.receive(Duration::from_secs(10)).unwrap(),
        Wire::Control(Message::Report {
            presented: 0,
            published: 0,
            dropped: 0
        })
    );
    peer.kill().unwrap();
    let _ = peer.wait_exit(Duration::from_secs(5)).unwrap();
    assert!(worker.finish().is_err());
    assert!(!peer.is_running());
}
