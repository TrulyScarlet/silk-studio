#![cfg(all(windows, target_arch = "x86_64", feature = "d3d12-cross-process"))]
use capture_hook_prototype::d3d12_cross_process::{Peer, RemoteConsumer};
use capture_hook_prototype::d3d12_protocol::{GpuHello, Wire};
use capture_hook_prototype::local_gpu_bridge::GpuPoolBudget;
use capture_hook_prototype::transport::Message;
use capture_hook_prototype::{FrameHeader, TargetIdentity};
use gpu_windows::{BgraReadback, GpuTextureSlot};
use media_types::{FramePayload, GpuFrameHandle, PixelFormat, StreamId, TimeBase, VideoFrame};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn launch(warp: bool, fault: Option<&str>) -> Peer {
    Peer::launch(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        warp,
        fault,
    )
    .unwrap()
}
fn greeting(peer: &Peer) -> GpuHello {
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing owned greeting")
    };
    hello
}
fn frame(peer: &Peer) -> FrameHeader {
    let Wire::Control(Message::Frame(frame)) = peer.receive(Duration::from_secs(5)).unwrap() else {
        panic!("missing remote frame")
    };
    frame
}
fn video(header: FrameHeader, handle: u64) -> VideoFrame {
    VideoFrame {
        stream_id: StreamId(0),
        width: header.width,
        height: header.height,
        pixel_format: PixelFormat::Bgra8,
        pts: 0,
        time_base: TimeBase::MILLISECOND,
        payload: FramePayload::Gpu(GpuFrameHandle(handle)),
    }
}
fn red(surface: &GpuTextureSlot) {
    let bytes = BgraReadback::default()
        .read_scaled(surface, 318, 178)
        .unwrap();
    assert!(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| *p == [0, 0, 255, 255]));
}
fn report(peer: &Peer) -> (u64, u64, u64) {
    let Wire::Control(Message::Report {
        presented,
        published,
        dropped,
    }) = peer.receive(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing report")
    };
    (presented, published, dropped)
}

#[test]
fn rejects_non_fixture_before_launch() {
    assert!(Peer::launch(Path::new("C:/Windows/System32/notepad.exe"), true, None).is_err());
}

#[cfg(feature = "d3d12-device-replace-fixture")]
fn deferred_producer_loss(mode: u8) {
    use capture_hook_prototype::d3d12_cross_process::RemoteCopyOutcome;
    use capture_hook_prototype::d3d12_protocol::DeviceLossReason;
    let mut peer = launch(false, Some("remove-on-command"));
    let hello = greeting(&peer);
    let mut consumer =
        RemoteConsumer::open(&peer, hello, false, Arc::new(GpuPoolBudget::default())).unwrap();
    let context = consumer.context();
    peer.approve(hello).unwrap();
    assert!(
        consumer.retire_epoch_after_producer_loss().is_err(),
        "healthy producer falsely retired as removed"
    );
    let first = frame(&peer);
    let (retained, ack) = consumer.copy(first).unwrap();
    let retained = retained.unwrap();
    let delivered = video(first, retained.handle());
    red(retained.surface());
    peer.send(ack).unwrap();
    for _ in 0..12 {
        let (_, ack) = consumer.copy(frame(&peer)).unwrap();
        peer.send(ack).unwrap();
    }
    // Hold a real received header across removal, not a fabricated frame.
    let mut queued = vec![frame(&peer)];
    peer.trigger_owned_loss(hello.session).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(std::time::Instant::now() < deadline);
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Frame(header)) => {
                queued.push(header);
                assert!(queued.len() <= 3);
            }
            Wire::DeviceLost(notice) => {
                assert_eq!(notice.session, hello.session);
                assert_eq!(notice.reason, DeviceLossReason::Removed);
                break;
            }
            other => panic!("unexpected producer-loss packet: {other:?}"),
        }
    }
    assert!(consumer.producer_removed());
    assert!(context.resolve(&delivered).is_ok());
    if mode == 2 {
        assert!(consumer.copy(queued[0]).is_err());
        assert!(context.session_resource().is_none());
        assert!(
            consumer.retire_epoch_after_producer_loss().is_err(),
            "default terminal mode silently entered opt-in recovery"
        );
    } else if mode == 1 {
        queued[0].session.epoch += 1;
        assert!(
            consumer.copy_if_preserving_loss(queued[0], true).is_err(),
            "producer loss hid an invalid session header"
        );
        assert!(context.session_resource().is_none());
    } else {
        for header in queued {
            assert!(matches!(
                consumer.copy_if_preserving_loss(header, true).unwrap(),
                RemoteCopyOutcome::ProducerLost
            ));
        }
        let lease = context
            .resolve(&delivered)
            .expect("producer loss invalidated the delivered-frame resolver gap");
        red(lease.downcast_ref::<GpuTextureSlot>().unwrap());
        consumer.retire_epoch_after_producer_loss().unwrap();
        assert!(context.resolve(&delivered).is_err());
        red(lease.downcast_ref::<GpuTextureSlot>().unwrap());
    }
    drop(consumer);
    peer.send(Message::Stop).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(std::time::Instant::now() < deadline);
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Bye) => break,
            Wire::Control(Message::Report { .. }) => {}
            other => panic!("loss stop emitted unexpected media: {other:?}"),
        }
    }
    let status = peer.wait_exit_status(Duration::from_secs(5)).unwrap();
    assert!(
        status.success(),
        "owned loss fixture did not stop normally: {status}"
    );
    red(retained.surface());
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "actual hardware producer removal preserves delivered-frame gap until independent consumer retirement"]
fn producer_loss_preserves_delivered_gap_hardware() {
    deferred_producer_loss(0);
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "actual hardware producer removal cannot hide stale frame session metadata"]
fn producer_loss_does_not_mask_stale_header_hardware() {
    deferred_producer_loss(1);
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "normal consumer remains fail-stop on actual hardware removal unless deferred handling was explicitly chosen"]
fn producer_loss_default_consumer_remains_terminal_hardware() {
    deferred_producer_loss(2);
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "owned loss trigger with wrong epoch exits normally without removing a device"]
fn producer_loss_wrong_trigger_rejected_hardware() {
    let mut peer = launch(false, Some("remove-on-command"));
    let hello = greeting(&peer);
    peer.approve(hello).unwrap();
    peer.trigger_owned_loss(capture_hook_prototype::SessionIdentity {
        epoch: hello.session.epoch + 1,
        ..hello.session
    })
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(std::time::Instant::now() < deadline);
        match peer.receive(Duration::from_secs(5)) {
            Ok(Wire::Control(Message::Frame(_))) => {}
            Err(_) => break,
            other => panic!("invalid trigger admitted a loss notice: {other:?}"),
        }
    }
    assert_eq!(
        peer.wait_exit_status(Duration::from_secs(5))
            .unwrap()
            .code(),
        Some(1)
    );
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "unannounced WARP probe is explicitly unsupported without attempting removal"]
fn producer_loss_unsupported_warp() {
    let mut peer = launch(true, Some("remove-on-command"));
    assert!(peer.receive(Duration::from_secs(5)).is_err());
    assert_eq!(
        peer.wait_exit_status(Duration::from_secs(5))
            .unwrap()
            .code(),
        Some(1)
    );
}
#[cfg(feature = "d3d12-device-replace-fixture")]
fn reconstructed_loss_control(scenario: u8) {
    use capture_hook_prototype::d3d12_cross_process::RemoteCopyOutcome;
    let mut peer = launch(false, Some("remove-and-recreate-on-command"));
    let hello = greeting(&peer);
    let mut consumer =
        RemoteConsumer::open(&peer, hello, false, Arc::new(GpuPoolBudget::default())).unwrap();
    peer.approve(hello).unwrap();
    let first = frame(&peer);
    let (retained, ack) = consumer.copy(first).unwrap();
    let retained = retained.unwrap();
    red(retained.surface());
    peer.send(ack).unwrap();
    peer.trigger_owned_loss(hello.session).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let next = loop {
        assert!(std::time::Instant::now() < deadline);
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Frame(header)) => {
                match consumer.copy_if_preserving_loss(header, true).unwrap() {
                    RemoteCopyOutcome::Copied(_, ack) => peer.send(ack).unwrap(),
                    RemoteCopyOutcome::ProducerLost => {}
                }
            }
            Wire::DeviceLost(notice) => {
                assert_eq!(notice.session, hello.session);
                break capture_hook_prototype::SessionIdentity {
                    epoch: hello.session.epoch + 1,
                    ..hello.session
                };
            }
            other => panic!("unexpected reconstruction pre-retirement packet: {other:?}"),
        }
    };
    consumer.retire_epoch_after_producer_loss().unwrap();
    drop(consumer);
    if scenario == 0 {
        peer.retired_epoch(capture_hook_prototype::SessionIdentity {
            epoch: next.epoch + 1,
            ..next
        })
        .unwrap();
    } else if scenario != 2 && scenario != 4 {
        peer.retired_epoch(next).unwrap();
        let Wire::Hello(replacement) = peer.receive(Duration::from_secs(8)).unwrap() else {
            panic!("missing reconstruction offer")
        };
        assert_eq!(replacement.session, next);
        assert_eq!((replacement.width, replacement.height), (640, 360));
        assert!(
            peer.receive(Duration::from_millis(80)).is_err(),
            "unapproved reconstruction published media"
        );
        if scenario == 1 {
            peer.send(Message::Select(hello.session)).unwrap();
        }
    }
    if scenario == 4 || scenario == 5 {
        peer.send(Message::Stop).unwrap();
        assert_eq!(
            peer.receive(Duration::from_secs(5)).unwrap(),
            Wire::Control(Message::Bye)
        );
        assert!(peer
            .wait_exit_status(Duration::from_secs(5))
            .unwrap()
            .success());
    } else {
        assert!(
            peer.receive(Duration::from_secs(7)).is_err(),
            "invalid reconstruction phase offered unexpected media"
        );
        assert_eq!(
            peer.wait_exit_status(Duration::from_secs(5))
                .unwrap()
                .code(),
            Some(1),
            "reconstruction negative path crashed"
        );
    }
    red(retained.surface());
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "automatic manual-source reconstruction rejects wrong retirement/approval with normal error exits"]
fn producer_loss_reconstruction_wrong_proofs_hardware() {
    for scenario in [0, 1] {
        reconstructed_loss_control(scenario);
    }
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "automatic manual-source reconstruction deadlines fail closed without media"]
fn producer_loss_reconstruction_missing_proofs_hardware() {
    for scenario in [2, 3] {
        reconstructed_loss_control(scenario);
    }
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "automatic manual-source reconstruction stops at retirement/approval and preserves old pixels"]
fn producer_loss_reconstruction_stop_barriers_hardware() {
    for scenario in [4, 5] {
        reconstructed_loss_control(scenario);
    }
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "automatic reconstruction before an initial ACK must reject without a loss notice or native fault"]
fn producer_loss_reconstruction_unprimed_rejected_hardware() {
    let mut peer = launch(false, Some("remove-and-recreate-on-command"));
    let hello = greeting(&peer);
    peer.approve(hello).unwrap();
    peer.trigger_owned_loss(hello.session).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(std::time::Instant::now() < deadline);
        match peer.receive(Duration::from_secs(5)) {
            Ok(Wire::Control(Message::Frame(_))) => {}
            Err(_) => break,
            other => panic!("unprimed source attempted removal/reconstruction: {other:?}"),
        }
    }
    assert_eq!(
        peer.wait_exit_status(Duration::from_secs(5))
            .unwrap()
            .code(),
        Some(1)
    );
}
#[cfg(feature = "d3d12-device-replace-fixture")]
#[test]
#[ignore = "automatic manual-source loss reconstruction is unsupported on WARP before graphics startup"]
fn producer_loss_reconstruction_unsupported_warp() {
    let mut peer = launch(true, Some("remove-and-recreate-on-command"));
    assert!(peer.receive(Duration::from_secs(5)).is_err());
    assert_eq!(
        peer.wait_exit_status(Duration::from_secs(5))
            .unwrap()
            .code(),
        Some(1)
    );
}

fn lifecycle(warp: bool) {
    let mut peer = launch(warp, None);
    let hello = greeting(&peer);
    assert_eq!(hello.session.target.pid, peer.pid());
    let budget = Arc::new(GpuPoolBudget::default());
    let mut wrong = hello;
    wrong.session.target = TargetIdentity {
        creation_time: hello.session.target.creation_time + 1,
        ..hello.session.target
    };
    assert!(RemoteConsumer::open(&peer, wrong, warp, budget.clone()).is_err());
    let mut wrong = hello;
    wrong.handles[4] = wrong.handles[0];
    assert!(RemoteConsumer::open(&peer, wrong, warp, budget.clone()).is_err());
    let mut wrong = hello;
    wrong.qpc_frequency += 1;
    assert!(RemoteConsumer::open(&peer, wrong, warp, budget.clone()).is_err());
    let mut consumer = RemoteConsumer::open(&peer, hello, warp, budget).unwrap();
    let context = consumer.context();
    peer.approve(hello).unwrap();
    let headers = [frame(&peer), frame(&peer), frame(&peer)];
    assert_eq!(headers.map(|h| h.sequence), [1, 2, 3]);
    peer.send(Message::Render { count: 1, color: 0 }).unwrap();
    let before = report(&peer);
    assert_eq!(before.1, 3);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let after = loop {
        std::thread::sleep(Duration::from_millis(25));
        peer.send(Message::Render { count: 1, color: 0 }).unwrap();
        let after = report(&peer);
        assert_eq!(
            after.1, 3,
            "saturated producer admitted another publication"
        );
        if after.0 >= before.0 + 12 && after.2 > 0 {
            break after;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "renderer did not advance twelve Presents under pressure"
        );
    };
    let mut held = Vec::new();
    let mut frames = Vec::new();
    for header in headers {
        let (publication, ack) = consumer.copy(header).unwrap();
        let publication = publication.unwrap();
        red(publication.surface());
        frames.push(video(header, publication.handle()));
        held.push(publication);
        peer.send(ack).unwrap();
    }
    let lease = context.resolve(&frames[0]).unwrap();
    drop(held.remove(0));
    assert!(context.resolve(&frames[0]).is_err());
    // Parent output stays saturated; copy() drops safely but still signals/ACKs.
    let mut copies = 0;
    while copies < 12 {
        let header = frame(&peer);
        let (publication, ack) = consumer.copy(header).unwrap();
        assert!(publication.is_none());
        peer.send(ack).unwrap();
        copies += 1;
    }
    assert_eq!(consumer.gpu_copies, 3);
    assert!(consumer.dropped >= 12);
    red(lease.downcast_ref::<GpuTextureSlot>().unwrap());
    consumer.close();
    assert!(context.resolve(&frames[1]).is_err());
    peer.send(Message::Stop).unwrap();
    let mut stopped_report = None;
    loop {
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Bye) => break,
            Wire::Control(Message::Report { presented, .. }) => stopped_report = Some(presented),
            Wire::Control(Message::Frame(_)) => {}
            other => panic!("unexpected stop packet {other:?}"),
        }
    }
    assert!(stopped_report.unwrap() >= after.0 + 12);
    assert!(peer.wait_exit(Duration::from_secs(5)).unwrap());
    red(lease.downcast_ref::<GpuTextureSlot>().unwrap());
    println!("PASS: owned cross-process D3D12 {}: exact identity/handle validation, 3-frame no-ACK pressure and continued rendering, GPU fence/copy/ACK cycle, retained output leases and local drops, graceful stop with continued Present, retained completed pixels after child exit. No capture readback; native pixel reads were test-only.",if warp{"WARP"}else{"hardware adapter 0"});
}
#[test]
#[ignore = "launches only owned D3D12 WARP fixture process"]
fn transport_warp() {
    lifecycle(true);
}
#[test]
#[ignore = "launches only owned D3D12 hardware fixture process"]
fn transport_hardware() {
    lifecycle(false);
}

#[test]
#[ignore = "owned WARP child malformed/truncated input and frame identity faults"]
fn malformed_and_stale_warp() {
    for fault in ["truncate", "malformed"] {
        let mut peer = launch(true, Some(fault));
        assert!(peer.receive(Duration::from_secs(5)).is_err());
        let _ = peer.wait_exit(Duration::from_secs(5)).unwrap();
    }
    let mut peer = launch(true, Some("bad-frame"));
    let hello = greeting(&peer);
    let mut consumer =
        RemoteConsumer::open(&peer, hello, true, Arc::new(GpuPoolBudget::default())).unwrap();
    peer.approve(hello).unwrap();
    let header = frame(&peer);
    assert!(consumer.copy(header).is_err());
    assert!(consumer.copy(header).is_err());
    peer.kill().unwrap();
    let _ = peer.wait_exit(Duration::from_secs(5)).unwrap();
    let mut peer = launch(true, None);
    let hello = greeting(&peer);
    peer.approve(hello).unwrap();
    let header = frame(&peer);
    peer.send(Message::Ack {
        session: hello.session,
        slot: header.slot,
        sequence: header.sequence + 100,
    })
    .unwrap();
    assert!(!peer.wait_exit(Duration::from_secs(5)).unwrap());
    let mut peer = launch(true, None);
    let hello = greeting(&peer);
    peer.approve(hello).unwrap();
    let _ = frame(&peer);
    peer.close_input();
    assert!(!peer.wait_exit(Duration::from_secs(5)).unwrap());
}

#[test]
#[ignore = "kills exact owned WARP child with pending remote GPU headers"]
fn producer_exit_rejects_queued_frames_warp() {
    let mut peer = launch(true, None);
    let hello = greeting(&peer);
    let mut consumer =
        RemoteConsumer::open(&peer, hello, true, Arc::new(GpuPoolBudget::default())).unwrap();
    let context = consumer.context();
    peer.approve(hello).unwrap();
    let header = frame(&peer);
    let (publication, ack) = consumer.copy(header).unwrap();
    let publication = publication.unwrap();
    let frame0 = video(header, publication.handle());
    let lease = context.resolve(&frame0).unwrap();
    peer.send(ack).unwrap();
    red(publication.surface());
    let pending = frame(&peer);
    peer.kill().unwrap();
    let _ = peer.wait_exit(Duration::from_secs(5)).unwrap();
    assert!(consumer.copy(pending).is_err());
    assert!(context.resolve(&frame0).is_err());
    red(lease.downcast_ref::<GpuTextureSlot>().unwrap());
}
