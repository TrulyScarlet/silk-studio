#![cfg(all(windows, target_arch = "x86_64", feature = "d3d12-bootstrap-fixture"))]
use capture_hook_prototype::d3d12_cross_process::{Peer, RemoteConsumer};
use capture_hook_prototype::d3d12_protocol::{WindowAction, Wire};
use capture_hook_prototype::local_gpu_bridge::GpuPoolBudget;
use capture_hook_prototype::transport::Message;
use capture_hook_prototype::FrameHeader;
use gpu_windows::{BgraReadback, GpuTextureSlot};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn start(warp: bool) -> (Peer, RemoteConsumer) {
    start_mode(warp, false)
}
fn start_mode(warp: bool, legacy: bool) -> (Peer, RemoteConsumer) {
    let executable = Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture"));
    let peer = if legacy {
        Peer::launch_legacy_factory(executable, warp).unwrap()
    } else {
        let peer = Peer::launch_bootstrap(
            Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
            warp,
            None,
        )
        .unwrap();
        peer
    };
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing bootstrap greeting")
    };
    assert_eq!((hello.width, hello.height), (318, 178));
    let consumer =
        RemoteConsumer::open(&peer, hello, warp, Arc::new(GpuPoolBudget::default())).unwrap();
    peer.approve(hello).unwrap();
    (peer, consumer)
}
fn frame(peer: &Peer) -> FrameHeader {
    let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing worker frame")
    };
    header
}
fn pixels(surface: &GpuTextureSlot, expected: [u8; 4]) {
    let bytes = BgraReadback::default()
        .read_scaled(surface, 318, 178)
        .unwrap();
    assert!(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == expected));
}
fn stop(peer: &mut Peer, consumer: &mut RemoteConsumer) {
    consumer.close();
    peer.send(Message::Stop).unwrap();
    let mut progress = None;
    loop {
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Report { presented, .. }) => progress = Some(presented),
            Wire::Control(Message::Bye) => break,
            Wire::Control(Message::Frame(_)) => {}
            other => panic!("unexpected worker stop packet {other:?}"),
        }
    }
    assert!(progress.unwrap() >= 12);
    assert!(peer.wait_exit(Duration::from_secs(5)).unwrap());
}
fn lifecycle(warp: bool) {
    lifecycle_mode(warp, false);
}
fn lifecycle_mode(warp: bool, legacy: bool) {
    let (mut peer, mut consumer) = start_mode(warp, legacy);
    let headers = [frame(&peer), frame(&peer), frame(&peer)];
    peer.send(Message::Render { count: 1, color: 2 }).unwrap();
    // Two raw renderer creations, only the exact selected HWND claimed by worker.
    assert_eq!(
        peer.receive(Duration::from_secs(5)).unwrap(),
        Wire::Control(Message::Report {
            presented: 2,
            published: 2,
            dropped: 1
        })
    );
    peer.send(Message::Render { count: 1, color: 1 }).unwrap();
    let Wire::Control(Message::Report {
        presented: before_capture,
        published: 0,
        dropped: 0,
    }) = peer.receive(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing pre-capture renderer progress")
    };
    assert!(before_capture >= 12);
    peer.send(Message::Render { count: 1, color: 3 }).unwrap();
    let Wire::Control(Message::Report {
        presented: before,
        dropped: unrelated,
        ..
    }) = peer.receive(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing worker counters")
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        std::thread::sleep(Duration::from_millis(25));
        peer.send(Message::Render { count: 1, color: 3 }).unwrap();
        let Wire::Control(Message::Report {
            presented: after,
            published: captured,
            dropped: unrelated_after,
        }) = peer.receive(Duration::from_secs(5)).unwrap()
        else {
            panic!("missing pressure counters")
        };
        assert_eq!(captured, 3);
        if after >= before + 12 && unrelated_after >= unrelated + 12 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "renderer did not advance both chains under capture pressure"
        );
    }
    for header in headers {
        let (publication, ack) = consumer.copy(header).unwrap();
        pixels(publication.as_ref().unwrap().surface(), [0, 0, 255, 255]);
        drop(publication);
        peer.send(ack).unwrap();
    }
    for action in [
        WindowAction::Minimize,
        WindowAction::Restore,
        WindowAction::Borderless,
        WindowAction::Windowed,
    ] {
        peer.window_action(action).unwrap();
        let marker = loop {
            match peer.receive(Duration::from_secs(5)).unwrap() {
                Wire::Control(Message::Frame(header)) => {
                    let (publication, ack) = consumer.copy(header).unwrap();
                    drop(publication);
                    peer.send(ack).unwrap();
                }
                Wire::Control(Message::Report {
                    presented,
                    published,
                    dropped: flags,
                }) => {
                    assert_eq!(presented, action as u64);
                    match action {
                        WindowAction::Minimize => assert_eq!(flags & 1, 1),
                        WindowAction::Restore => assert_eq!(flags & 1, 0),
                        WindowAction::Borderless => assert_eq!(flags & 6, 6),
                        WindowAction::Windowed => assert_eq!(flags & 3, 0),
                        WindowAction::ExcludeCapture => {
                            unreachable!("exclusion is tested by the owned WGC suite")
                        }
                    };
                    break published as i64;
                }
                other => panic!("unexpected action packet {other:?}"),
            }
        };
        let expected = match action {
            WindowAction::Minimize => [0, 255, 0, 255],
            WindowAction::Restore => [255, 0, 255, 255],
            WindowAction::Borderless => [0, 255, 255, 255],
            WindowAction::Windowed => [0, 0, 255, 255],
            WindowAction::ExcludeCapture => {
                unreachable!("exclusion is tested by the owned WGC suite")
            }
        };
        let mut fresh = 0;
        let deadline = Instant::now() + Duration::from_secs(8);
        while fresh < 3 {
            assert!(Instant::now() < deadline);
            let header = frame(&peer);
            let (publication, ack) = consumer.copy(header).unwrap();
            if header.qpc_ticks >= marker {
                if let Some(publication) = &publication {
                    pixels(publication.surface(), expected);
                    fresh += 1;
                }
            }
            drop(publication);
            peer.send(ack).unwrap();
        }
    }
    stop(&mut peer, &mut consumer);
    println!("PASS: independent owned bootstrap {}: worker-created factory; PID/lifetime/HWND-only selection; two observed creations/one worker claim; Present pressure progress; fresh window-action pixels; worker cancellation with 12 further renderer Presents. No remote loading or live rearm.", if warp {"WARP"} else {"hardware"});
}
#[test]
#[ignore = "owned legacy CreateSwapChain observation, WARP transport and lifecycle"]
fn legacy_factory_lifecycle_warp() {
    lifecycle_mode(true, true);
}
#[test]
#[ignore = "owned legacy CreateSwapChain observation, hardware transport and lifecycle"]
fn legacy_factory_lifecycle_hardware() {
    lifecycle_mode(false, true);
}
fn soak(warp: bool) {
    let (mut peer, mut consumer) = start(warp);
    let deadline = Instant::now() + Duration::from_secs(25);
    for index in 0..500 {
        assert!(Instant::now() < deadline);
        let header = frame(&peer);
        let (publication, ack) = consumer.copy(header).unwrap();
        if index % 50 == 0 {
            pixels(publication.as_ref().unwrap().surface(), [0, 0, 255, 255]);
        }
        drop(publication);
        peer.send(ack).unwrap();
        if index == 100 || index == 300 {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    assert_eq!(consumer.gpu_copies, 500);
    stop(&mut peer, &mut consumer);
}
#[test]
#[ignore = "independent owned WARP renderer/bootstrap worker and native window tests"]
fn bootstrap_lifecycle_warp() {
    lifecycle(true);
}
#[test]
#[ignore = "independent owned hardware renderer/bootstrap worker and native window tests"]
fn bootstrap_lifecycle_hardware() {
    lifecycle(false);
}
#[test]
#[ignore = "500-frame independent owned WARP worker soak"]
fn bootstrap_soak_warp() {
    soak(true);
}
#[test]
#[ignore = "500-frame independent owned hardware worker soak"]
fn bootstrap_soak_hardware() {
    soak(false);
}

#[test]
#[ignore = "owned bootstrap startup identity and exact-approval failures"]
fn bootstrap_rejects_stale_identity_and_approval_warp() {
    let path = Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture"));
    let mut peer = Peer::launch_bootstrap(path, true, Some("bootstrap-stale-target")).unwrap();
    assert!(peer.receive(Duration::from_secs(10)).is_err());
    assert!(!peer.wait_exit(Duration::from_secs(5)).unwrap());
    let mut peer = Peer::launch_bootstrap(path, true, None).unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing worker greeting")
    };
    let mut wrong = hello.session;
    wrong.epoch += 1;
    peer.send(Message::Select(wrong)).unwrap();
    assert!(!peer.wait_exit(Duration::from_secs(5)).unwrap());
}
