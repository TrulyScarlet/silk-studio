#![cfg(all(windows, target_arch = "x86_64", feature = "d3d12-rearm-fixture"))]
use capture_hook_prototype::d3d12_cross_process::{Peer, RemoteConsumer};
use capture_hook_prototype::d3d12_protocol::{GpuHello, ResizeRequest, Wire};
use capture_hook_prototype::local_gpu_bridge::GpuPoolBudget;
use capture_hook_prototype::transport::Message;
use gpu_windows::{BgraReadback, GpuTextureSlot};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
fn start(warp: bool) -> (Peer, RemoteConsumer, GpuHello, Arc<GpuPoolBudget>) {
    let peer = Peer::launch_rearm(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        warp,
        None,
    )
    .unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing greeting")
    };
    let budget = Arc::new(GpuPoolBudget::default());
    let consumer = RemoteConsumer::open(&peer, hello, warp, budget.clone()).unwrap();
    peer.approve(hello).unwrap();
    (peer, consumer, hello, budget)
}
fn pixels(surface: &GpuTextureSlot, expected: [u8; 4]) {
    let bytes = BgraReadback::default()
        .read_scaled(surface, 64, 36)
        .unwrap();
    assert!(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == expected));
}
fn resize(
    peer: &Peer,
    mut consumer: RemoteConsumer,
    hello: GpuHello,
    dimensions: (u32, u32),
    warp: bool,
    budget: Arc<GpuPoolBudget>,
) -> (RemoteConsumer, GpuHello) {
    peer.resize(ResizeRequest {
        session: hello.session,
        width: dimensions.0,
        height: dimensions.1,
    })
    .unwrap();
    let next = loop {
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Frame(header)) => {
                let (publication, ack) = consumer.copy(header).unwrap();
                drop(publication);
                peer.send(ack).unwrap();
            }
            Wire::Retire(next) => break next,
            other => panic!("unexpected resize packet {other:?}"),
        }
    };
    assert_eq!(next.epoch, hello.session.epoch + 1);
    assert_eq!(next.target, hello.session.target);
    consumer.retire_epoch().unwrap();
    drop(consumer);
    peer.retired_epoch(next).unwrap();
    let Wire::Hello(replacement) = peer.receive(Duration::from_secs(5)).unwrap() else {
        panic!("missing replacement handles")
    };
    assert_eq!(replacement.session, next);
    assert_eq!((replacement.width, replacement.height), dimensions);
    let consumer = RemoteConsumer::open(peer, replacement, warp, budget).unwrap();
    assert!(
        peer.receive(Duration::from_millis(50)).is_err(),
        "new epoch published before exact approval"
    );
    peer.approve(replacement).unwrap();
    (consumer, replacement)
}
fn lifecycle(warp: bool) {
    let (mut peer, mut consumer, hello, budget) = start(warp);
    let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing initial frame")
    };
    let context = consumer.context();
    let (old, ack) = consumer.copy(header).unwrap();
    let old = old.unwrap();
    pixels(old.surface(), [0, 0, 255, 255]);
    peer.send(ack).unwrap();
    let (replacement, newhello) = resize(&peer, consumer, hello, (640, 360), warp, budget.clone());
    consumer = replacement;
    assert!(context.session_resource().is_none());
    pixels(old.surface(), [0, 0, 255, 255]);
    let Wire::Control(Message::Frame(green)) = peer.receive(Duration::from_secs(5)).unwrap() else {
        panic!("missing new frame")
    };
    assert_eq!(green.session, newhello.session);
    assert_eq!((green.width, green.height), (640, 360));
    let (publication, ack) = consumer.copy(green).unwrap();
    pixels(publication.as_ref().unwrap().surface(), [0, 255, 0, 255]);
    drop(publication);
    peer.send(ack).unwrap();
    drop(old);
    drop(context);
    let (replacement, lasthello) = resize(&peer, consumer, newhello, (800, 450), warp, budget);
    consumer = replacement;
    let Wire::Control(Message::Frame(blue)) = peer.receive(Duration::from_secs(5)).unwrap() else {
        panic!("missing second replacement frame")
    };
    assert_eq!(blue.session, lasthello.session);
    let (publication, ack) = consumer.copy(blue).unwrap();
    pixels(publication.as_ref().unwrap().surface(), [255, 0, 0, 255]);
    drop(publication);
    peer.send(ack).unwrap();
    consumer.close();
    peer.send(Message::Stop).unwrap();
    loop {
        if peer.receive(Duration::from_secs(5)).unwrap() == Wire::Control(Message::Bye) {
            break;
        }
    }
    assert!(peer.wait_exit(Duration::from_secs(5)).unwrap());
    println!("PASS: owned same-chain live resize {}: 318x178 -> 640x360 -> 800x450; exact retire/drain/new-handle/approval epochs; retained old red pixels and fresh green/blue; no frames before approval. Not device-loss recovery.",if warp{"WARP"}else{"hardware"});
}
#[test]
#[ignore = "live ResizeBuffers and capture rearm in exact owned WARP child"]
fn resize_lifecycle_warp() {
    lifecycle(true);
}
#[test]
#[ignore = "live ResizeBuffers and capture rearm in exact owned hardware child"]
fn resize_lifecycle_hardware() {
    lifecycle(false);
}
#[test]
#[ignore = "owned WARP resize rejects old-epoch ACK after new epoch approval"]
fn resize_rejects_old_ack_warp() {
    let (mut peer, consumer, hello, budget) = start(true);
    let (_consumer, _newhello) = resize(&peer, consumer, hello, (640, 360), true, budget);
    peer.send(Message::Ack {
        session: hello.session,
        slot: 0,
        sequence: 1,
    })
    .unwrap();
    assert!(!peer.wait_exit(Duration::from_secs(5)).unwrap());
}

#[test]
#[ignore = "owned WARP resize rejects incorrect retirement approval before resizing"]
fn resize_rejects_wrong_retirement_warp() {
    let (mut peer, mut consumer, hello, _budget) = start(true);
    assert!(peer
        .resize(ResizeRequest {
            session: hello.session,
            width: 641,
            height: 360
        })
        .is_err());
    peer.resize(ResizeRequest {
        session: hello.session,
        width: 640,
        height: 360,
    })
    .unwrap();
    loop {
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Frame(header)) => {
                let (publication, ack) = consumer.copy(header).unwrap();
                drop(publication);
                peer.send(ack).unwrap();
            }
            Wire::Retire(next) => {
                consumer.retire_epoch().unwrap();
                let mut wrong = next;
                wrong.epoch += 1;
                peer.retired_epoch(wrong).unwrap();
                break;
            }
            other => panic!("unexpected retirement packet {other:?}"),
        }
    }
    assert!(!peer.wait_exit(Duration::from_secs(5)).unwrap());
}
