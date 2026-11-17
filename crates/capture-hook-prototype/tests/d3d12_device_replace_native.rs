#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "d3d12-device-replace-fixture"
))]
//! Exact owned healthy teardown/recreation and device-lifetime preconditions.
use windows::core::{IUnknown, Interface};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, ID3D12CommandQueue, ID3D12Device, D3D12_COMMAND_LIST_TYPE_DIRECT,
    D3D12_COMMAND_QUEUE_DESC,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory4};

fn create(adapter: &IDXGIAdapter) -> ID3D12Device {
    let mut device = None;
    unsafe { D3D12CreateDevice(adapter, D3D_FEATURE_LEVEL_11_0, &mut device) }.unwrap();
    device.expect("D3D12CreateDevice returned no device")
}

fn singleton_lifetime(warp: bool) {
    let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.unwrap();
    let adapter: IDXGIAdapter = if warp {
        unsafe { factory.EnumWarpAdapter() }.unwrap()
    } else {
        unsafe { factory.EnumAdapters(0) }.unwrap()
    };
    let first = create(&adapter);
    let second = create(&adapter);
    let first_identity: IUnknown = first.cast().unwrap();
    let second_identity: IUnknown = second.cast().unwrap();
    assert_eq!(first_identity.as_raw(), second_identity.as_raw());
    unsafe { first.GetDeviceRemovedReason() }.unwrap();
    let luid = unsafe { first.GetAdapterLuid() };
    let queue: ID3D12CommandQueue = unsafe {
        first.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        })
    }
    .unwrap();
    drop(first_identity);
    drop(second_identity);
    drop(first);
    drop(second);

    // Even without explicit device interfaces, a live child queue retains the
    // singleton. Comparing two live canonical devices cannot prove replacement.
    let mut retained: Option<ID3D12Device> = None;
    unsafe { queue.GetDevice(&mut retained) }.unwrap();
    let retained = retained.unwrap();
    let third = create(&adapter);
    let retained_identity: IUnknown = retained.cast().unwrap();
    let third_identity: IUnknown = third.cast().unwrap();
    assert_eq!(retained_identity.as_raw(), third_identity.as_raw());
    drop(retained_identity);
    drop(third_identity);
    drop(retained);
    drop(third);
    drop(queue);

    // After releasing our entire old object graph, creation is healthy again.
    // Address inequality is deliberately NOT asserted: COM addresses can recycle,
    // and this test cannot establish that no runtime-internal reference survived.
    let after_release = create(&adapter);
    unsafe { after_release.GetDeviceRemovedReason() }.unwrap();
    let new_luid = unsafe { after_release.GetAdapterLuid() };
    assert_eq!(
        (luid.HighPart, luid.LowPart),
        (new_luid.HighPart, new_luid.LowPart)
    );
    println!("PASS: {} same-adapter D3D12 singleton and queue-retained lifetime; healthy creation after releasing test references. Not device replacement or removal recovery.", if warp {"WARP"} else {"hardware"});
}

#[test]
#[ignore = "creates real WARP D3D12 device/queue objects"]
fn device_singleton_lifetime_warp() {
    singleton_lifetime(true);
}

#[test]
#[ignore = "creates real hardware D3D12 device/queue objects"]
fn device_singleton_lifetime_hardware() {
    singleton_lifetime(false);
}

fn removed_device_contract(warp: bool) {
    use windows::Win32::Graphics::Direct3D12::{ID3D12Device5, ID3D12Fence, D3D12_FENCE_FLAG_NONE};
    let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.unwrap();
    let adapter: IDXGIAdapter = if warp {
        unsafe { factory.EnumWarpAdapter() }.unwrap()
    } else {
        unsafe { factory.EnumAdapters(0) }.unwrap()
    };
    let device = create(&adapter);
    let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.unwrap();
    assert_eq!(unsafe { fence.GetCompletedValue() }, 0);
    let removed: ID3D12Device5 = device.cast().unwrap();
    unsafe { removed.RemoveDevice() };
    assert!(unsafe { device.GetDeviceRemovedReason() }.is_err());
    assert_eq!(
        unsafe { fence.GetCompletedValue() },
        u64::MAX,
        "removed fence must not be treated as completed work"
    );
    let mut blocked = None::<ID3D12Device>;
    assert!(
        unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut blocked) }.is_err(),
        "retained removed singleton was incorrectly recreated"
    );
    drop(blocked);
    drop(fence);
    drop(removed);
    drop(device);
    let replacement = create(&adapter);
    unsafe { replacement.GetDeviceRemovedReason() }.unwrap();
    println!("PASS: {} actual RemoveDevice reports loss, UINT64_MAX fence sentinel and blocked creation while removed singleton is retained; fresh creation succeeds after release. No in-flight GPU work or driver reset.",if warp {"WARP"} else {"hardware"});
}

fn pending_queue_removal_contract(warp: bool) {
    use std::time::{Duration, Instant};
    use windows::Win32::Graphics::Direct3D12::{ID3D12Device5, ID3D12Fence, D3D12_FENCE_FLAG_NONE};
    let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.unwrap();
    let adapter: IDXGIAdapter = if warp {
        unsafe { factory.EnumWarpAdapter() }.unwrap()
    } else {
        unsafe { factory.EnumAdapters(0) }.unwrap()
    };
    let device = create(&adapter);
    let removable: ID3D12Device5 = device.cast().unwrap();
    let queue: ID3D12CommandQueue = unsafe {
        device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        })
    }
    .unwrap();
    let gate: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.unwrap();
    let completed: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.unwrap();
    // The device-local queue cannot reach this signal until the CPU releases
    // gate. RemoveDevice is invoked immediately, not after a timer/TDR. A guard
    // releases the wait on any unwinding path before removal.
    struct ReleaseWait(ID3D12Fence);
    impl Drop for ReleaseWait {
        fn drop(&mut self) {
            let _ = unsafe { self.0.Signal(1) };
        }
    }
    let release = ReleaseWait(gate.clone());
    unsafe { queue.Wait(&gate, 1) }.unwrap();
    unsafe { queue.Signal(&completed, 1) }.unwrap();
    assert_eq!(
        unsafe { completed.GetCompletedValue() },
        0,
        "queue work must still be pending before removal"
    );
    let started = Instant::now();
    unsafe { removable.RemoveDevice() };
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "owned removal exceeded bound"
    );
    assert!(unsafe { device.GetDeviceRemovedReason() }.is_err());
    assert_eq!(unsafe { completed.GetCompletedValue() }, u64::MAX);
    // UINT64_MAX denotes removal, never successful completion of sequence one.
    let mut slots = capture_hook_prototype::shared_fence_slots::SharedFenceSlots::default();
    slots.reserve(0).unwrap().unwrap();
    assert!(slots
        .ready(unsafe { completed.GetCompletedValue() })
        .is_err());
    assert!(slots.reserve(0).is_err());
    drop(release);
    drop(completed);
    drop(gate);
    drop(queue);
    drop(removable);
    drop(device);
    let replacement = create(&adapter);
    unsafe { replacement.GetDeviceRemovedReason() }.unwrap();
    println!("PASS: {} owned removal with a provably pending queue signal; terminal fence rejection and healthy recreation after full test-graph release. Not in-flight media recovery or TDR acceptance.", if warp { "WARP" } else { "hardware" });
}
#[test]
#[ignore = "explicit removal of owned WARP device with pending queue signal; no TDR"]
fn pending_queue_removal_warp() {
    pending_queue_removal_contract(true);
}
#[test]
#[ignore = "explicit removal of owned hardware device with pending queue signal; no TDR"]
fn pending_queue_removal_hardware() {
    pending_queue_removal_contract(false);
}
fn pending_media_removal(warp: bool) {
    pending_media_removal_mode(warp, false);
}
fn pending_media_removal_mode(warp: bool, expose_consumer: bool) {
    pending_media_removal_run(warp, expose_consumer, false);
}
fn pending_media_removal_run(warp: bool, expose_consumer: bool, repeat: bool) {
    use std::time::{Duration, Instant};
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture"));
    command.args(["--allow-owned-d3d12", "--pending-media-removal"]);
    if expose_consumer {
        command.arg("--expose-local-consumer");
    }
    if repeat {
        command.arg("--repeat-pending-media-removal");
    }
    if warp {
        command.arg("--warp");
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(if repeat { 60 } else { 15 });
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "owned pending-media probe failed: {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned pending-media probe exceeded process deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "owned pending WARP copy removal and same-process unexposed-graph recreation"]
fn pending_media_removal_warp() {
    pending_media_removal(true);
}
#[test]
#[ignore = "owned pending hardware copy removal and same-process unexposed-graph recreation"]
fn pending_media_removal_hardware() {
    pending_media_removal(false);
}
#[test]
#[ignore = "retired local WARP consumer permits removed-graph recreation and retains pixels"]
fn pending_media_removal_exposed_warp() {
    pending_media_removal_mode(true, true);
}
#[test]
#[ignore = "retired local hardware consumer permits removed-graph recreation and retains pixels"]
fn pending_media_removal_exposed_hardware() {
    pending_media_removal_mode(false, true);
}
#[test]
#[ignore = "ten same-process WARP removal/recreation cycles with retained local pixels"]
fn pending_media_removal_repeated_warp() {
    pending_media_removal_run(true, true, true);
}
#[test]
#[ignore = "ten same-process hardware removal/recreation cycles with retained local pixels"]
fn pending_media_removal_repeated_hardware() {
    pending_media_removal_run(false, true, true);
}
#[test]
#[ignore = "actual owned WARP removal fence/singleton contract, no submitted GPU work"]
fn removed_device_fence_and_singleton_warp() {
    removed_device_contract(true);
}
#[test]
#[ignore = "actual owned hardware removal fence/singleton contract, no submitted GPU work"]
fn removed_device_fence_and_singleton_hardware() {
    removed_device_contract(false);
}

use capture_hook_prototype::d3d12_cross_process::{Peer, RemoteConsumer};
use capture_hook_prototype::d3d12_protocol::{GpuHello, ResizeRequest, Wire};
use capture_hook_prototype::local_gpu_bridge::GpuPoolBudget;
use capture_hook_prototype::transport::Message;
use gpu_windows::{BgraReadback, GpuTextureSlot};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
fn start(warp: bool, fault: Option<&str>) -> (Peer, RemoteConsumer, GpuHello, Arc<GpuPoolBudget>) {
    let peer = Peer::launch_device_replace(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        warp,
        fault,
    )
    .unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing initial hello")
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
    assert!(bytes.as_chunks::<4>().0.iter().all(|p| *p == expected));
}
fn prime_pending_route(peer: &Peer, consumer: &mut RemoteConsumer, fault: Option<&str>) {
    if fault != Some("replace-remove-pending") {
        return;
    }
    let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing initial remote publication")
    };
    let (publication, ack) = consumer.copy(header).unwrap();
    pixels(publication.unwrap().surface(), [0, 0, 255, 255]);
    peer.send(ack).unwrap();
}
fn retire(
    peer: &Peer,
    consumer: &mut RemoteConsumer,
    hello: GpuHello,
    dimensions: (u32, u32),
) -> capture_hook_prototype::SessionIdentity {
    peer.replace_device(ResizeRequest {
        session: hello.session,
        width: dimensions.0,
        height: dimensions.1,
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
                assert_eq!(next.epoch, hello.session.epoch + 1);
                assert_eq!(next.target, hello.session.target);
                assert_eq!(next.generation, hello.session.generation);
                consumer.retire_epoch().unwrap();
                return next;
            }
            other => panic!("unexpected retirement packet: {other:?}"),
        }
    }
}
fn replace(
    peer: &Peer,
    mut consumer: RemoteConsumer,
    hello: GpuHello,
    dimensions: (u32, u32),
    warp: bool,
    budget: Arc<GpuPoolBudget>,
) -> (RemoteConsumer, GpuHello) {
    let next = retire(peer, &mut consumer, hello, dimensions);
    drop(consumer);
    peer.retired_epoch(next).unwrap();
    let Wire::Hello(new) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing replacement hello")
    };
    assert_eq!(new.session, next);
    assert_eq!((new.width, new.height), dimensions);
    assert_eq!(new.adapter_luid, hello.adapter_luid);
    assert_eq!(new.qpc_frequency, hello.qpc_frequency);
    let consumer = RemoteConsumer::open(peer, new, warp, budget).unwrap();
    assert!(
        peer.receive(Duration::from_millis(80)).is_err(),
        "replacement published before approval"
    );
    peer.approve(new).unwrap();
    (consumer, new)
}
fn lifecycle(warp: bool, rounds: usize, minimum_duration: Duration) {
    lifecycle_with_fault(warp, rounds, minimum_duration, None);
}
fn lifecycle_with_fault(
    warp: bool,
    rounds: usize,
    minimum_duration: Duration,
    fault: Option<&str>,
) {
    let started = std::time::Instant::now();
    let mut publications = 0u64;
    let (mut peer, mut consumer, mut hello, budget) = start(warp, fault);
    let Wire::Control(Message::Frame(first)) = peer.receive(Duration::from_secs(5)).unwrap() else {
        panic!("missing initial frame")
    };
    let context = consumer.context();
    let (old, ack) = consumer.copy(first).unwrap();
    let old = old.unwrap();
    pixels(old.surface(), [0, 0, 255, 255]);
    peer.send(ack).unwrap();
    let mut prior_qpc = first.qpc_ticks;
    for index in 0..rounds {
        let dimensions = if index % 2 == 0 {
            (640, 360)
        } else {
            (800, 450)
        };
        (consumer, hello) = replace(&peer, consumer, hello, dimensions, warp, budget.clone());
        assert!(context.session_resource().is_none());
        pixels(old.surface(), [0, 0, 255, 255]);
        // Observe fresh pixels and timestamps for multiple publications each epoch.
        let mut epoch_publications = 0;
        while epoch_publications < 12
            || (index + 1 == rounds && started.elapsed() < minimum_duration)
        {
            let Wire::Control(Message::Frame(header)) =
                peer.receive(Duration::from_secs(5)).unwrap()
            else {
                panic!("missing recreated frame")
            };
            assert_eq!(header.session, hello.session);
            assert!(header.qpc_ticks > prior_qpc);
            prior_qpc = header.qpc_ticks;
            let (publication, ack) = consumer.copy(header).unwrap();
            pixels(
                publication.as_ref().unwrap().surface(),
                if index % 2 == 0 {
                    [0, 255, 0, 255]
                } else {
                    [255, 0, 0, 255]
                },
            );
            drop(publication);
            peer.send(ack).unwrap();
            epoch_publications += 1;
            publications += 1;
        }
    }
    consumer.close();
    peer.send(Message::Stop).unwrap();
    loop {
        if peer.receive(Duration::from_secs(5)).unwrap() == Wire::Control(Message::Bye) {
            break;
        }
    }
    assert!(peer.wait_exit(Duration::from_secs(5)).unwrap());
    pixels(old.surface(), [0, 0, 255, 255]);
    let route = match fault {
        Some("replace-remove-pending") => "retired remote epoch + pending producer-copy removal",
        Some("replace-remove-quiescent") => "quiescent device removal",
        _ => "healthy device-graph recreation",
    };
    println!("PASS: {} owned {route}: {rounds} same-HWND epochs, {publications} checked publications in {:.1}s, fresh pixels/QPC, retained old pixels, exact approval and continued Present after stop. Not unexpected-loss/TDR or real-game acceptance.", if warp {"WARP"} else {"hardware"}, started.elapsed().as_secs_f64());
}
#[test]
#[ignore = "owned WARP same-HWND healthy teardown/recreation"]
fn replacement_lifecycle_warp() {
    lifecycle(true, 2, Duration::ZERO);
}
#[test]
#[ignore = "owned hardware same-HWND healthy teardown/recreation"]
fn replacement_lifecycle_hardware() {
    lifecycle(false, 2, Duration::ZERO);
}

#[test]
#[ignore = "documented RemoveDevice after complete owned GPU drain, release and WARP recreation"]
fn replacement_quiescent_device_removal_warp() {
    lifecycle_with_fault(true, 2, Duration::ZERO, Some("replace-remove-quiescent"));
}
#[test]
#[ignore = "documented RemoveDevice after complete owned GPU drain, release and hardware recreation"]
fn replacement_quiescent_device_removal_hardware() {
    lifecycle_with_fault(false, 2, Duration::ZERO, Some("replace-remove-quiescent"));
}
#[test]
#[ignore = "remote retirement, actual unpublished pending copy removal and owned hardware reconstruction"]
fn replacement_pending_copy_removal_hardware() {
    lifecycle_with_fault(false, 2, Duration::ZERO, Some("replace-remove-pending"));
}

#[test]
#[ignore = "twenty healthy same-HWND recreation epochs in an owned WARP child"]
fn replacement_soak_warp() {
    lifecycle(true, 20, Duration::from_secs(60));
}
#[test]
#[ignore = "twenty healthy same-HWND recreation epochs in an owned hardware child"]
fn replacement_soak_hardware() {
    lifecycle(false, 20, Duration::from_secs(60));
}

#[test]
#[ignore = "owned WARP recreation rejects stale ACK/request and wrong retirement/approval"]
fn replacement_rejects_stale_controls_warp() {
    rejected_controls(true, None);
}
fn rejected_controls(warp: bool, fault: Option<&str>) {
    for stale_ack in [false, true] {
        let (mut peer, mut consumer, hello, budget) = start(warp, fault);
        prime_pending_route(&peer, &mut consumer, fault);
        let (_consumer, _new) = replace(&peer, consumer, hello, (640, 360), warp, budget);
        if stale_ack {
            peer.send(Message::Ack {
                session: hello.session,
                slot: 0,
                sequence: 1,
            })
            .unwrap();
        } else {
            peer.replace_device(ResizeRequest {
                session: hello.session,
                width: 640,
                height: 360,
            })
            .unwrap();
        }
        assert_eq!(
            peer.wait_exit_status(Duration::from_secs(5))
                .unwrap()
                .code(),
            Some(1)
        );
    }
    for wrong_retirement in [true, false] {
        let (mut peer, mut consumer, hello, _budget) = start(warp, fault);
        prime_pending_route(&peer, &mut consumer, fault);
        assert!(peer
            .replace_device(ResizeRequest {
                session: hello.session,
                width: 641,
                height: 360
            })
            .is_err());
        let next = retire(&peer, &mut consumer, hello, (640, 360));
        drop(consumer);
        if wrong_retirement {
            peer.retired_epoch(capture_hook_prototype::SessionIdentity {
                epoch: next.epoch + 1,
                ..next
            })
            .unwrap();
        } else {
            peer.retired_epoch(next).unwrap();
            let Wire::Hello(new) = peer.receive(Duration::from_secs(10)).unwrap() else {
                panic!("missing hello")
            };
            assert!(peer.receive(Duration::from_millis(80)).is_err());
            peer.send(Message::Select(capture_hook_prototype::SessionIdentity {
                epoch: new.session.epoch + 1,
                ..new.session
            }))
            .unwrap();
        }
        assert_eq!(
            peer.wait_exit_status(Duration::from_secs(5))
                .unwrap()
                .code(),
            Some(1)
        );
    }
}

#[test]
#[ignore = "owned WARP recreation retirement and approval deadlines fail closed"]
fn replacement_missing_proofs_warp() {
    missing_proofs(true, None);
}
fn missing_proofs(warp: bool, fault: Option<&str>) {
    for no_retirement in [true, false] {
        let (mut peer, mut consumer, hello, _budget) = start(warp, fault);
        prime_pending_route(&peer, &mut consumer, fault);
        let next = retire(&peer, &mut consumer, hello, (640, 360));
        drop(consumer);
        if !no_retirement {
            peer.retired_epoch(next).unwrap();
            let greeting = peer.receive(Duration::from_secs(10));
            if let Err(error) = &greeting {
                let status = peer.wait_exit(Duration::from_secs(5));
                panic!("replacement failed before approval probe: {error}; exit={status:?}");
            }
            assert!(matches!(greeting.unwrap(), Wire::Hello(_)));
        }
        assert_eq!(
            peer.wait_exit_status(Duration::from_secs(8))
                .unwrap()
                .code(),
            Some(1)
        );
    }
}
#[test]
#[ignore = "pending-copy removal rejects stale controls and mismatched remote retirement/approval"]
fn replacement_pending_copy_rejects_controls_hardware() {
    rejected_controls(false, Some("replace-remove-pending"));
}
#[test]
#[ignore = "pending-copy removal cannot proceed without exact bounded retirement/approval proofs"]
fn replacement_pending_copy_missing_proofs_hardware() {
    missing_proofs(false, Some("replace-remove-pending"));
}
#[test]
#[ignore = "unprimed pending-copy removal is explicitly unsupported and must exit normally, never crash"]
fn replacement_pending_copy_before_first_ack_rejected_hardware() {
    let (mut peer, mut consumer, hello, _) = start(false, Some("replace-remove-pending"));
    peer.replace_device(ResizeRequest {
        session: hello.session,
        width: 640,
        height: 360,
    })
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let next = loop {
        assert!(std::time::Instant::now() < deadline);
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Frame(_)) => {} // Deliberately no copy or ACK.
            Wire::Retire(next) => break next,
            other => panic!("unexpected unprimed retirement packet: {other:?}"),
        }
    };
    consumer.retire_epoch().unwrap();
    drop(consumer);
    peer.retired_epoch(next).unwrap();
    assert!(
        peer.receive(Duration::from_secs(5)).is_err(),
        "unprimed removal published replacement media"
    );
    let status = peer.wait_exit_status(Duration::from_secs(5)).unwrap();
    assert_eq!(
        status.code(),
        Some(1),
        "unsupported unprimed removal crashed: {status}"
    );
}
fn pending_copy_cold_roundtrips(warp: bool) {
    for _ in 0..6 {
        let fault = Some("replace-remove-pending");
        let (mut peer, mut consumer, hello, budget) = start(warp, fault);
        prime_pending_route(&peer, &mut consumer, fault);
        let (mut consumer, _) = replace(&peer, consumer, hello, (640, 360), warp, budget);
        let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
        else {
            panic!("missing fresh cold-start publication")
        };
        let (publication, ack) = consumer.copy(header).unwrap();
        let publication = publication.unwrap();
        pixels(publication.surface(), [0, 255, 0, 255]);
        peer.send(ack).unwrap();
        consumer.close();
        peer.send(Message::Stop).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            assert!(std::time::Instant::now() < deadline);
            match peer.receive(Duration::from_secs(5)).unwrap() {
                Wire::Control(Message::Bye) => break,
                Wire::Control(Message::Frame(_) | Message::Report { .. }) => {}
                other => panic!("unexpected cold-start stop packet: {other:?}"),
            }
        }
        assert!(peer.wait_exit(Duration::from_secs(5)).unwrap());
        pixels(publication.surface(), [0, 255, 0, 255]);
    }
}
#[test]
#[ignore = "unsafe WARP remote pending-copy removal must be rejected before graphics startup"]
fn replacement_pending_copy_unsupported_warp() {
    let mut peer = Peer::launch_device_replace(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        true,
        Some("replace-remove-pending"),
    )
    .unwrap();
    assert!(peer.receive(Duration::from_secs(5)).is_err());
    let status = peer.wait_exit_status(Duration::from_secs(5)).unwrap();
    assert_eq!(
        status.code(),
        Some(1),
        "unsupported WARP probe crashed: {status}"
    );
}
#[test]
#[ignore = "six cold hardware sources recover after one exact initial ACK; normal exit and retained fresh pixels"]
fn replacement_pending_copy_cold_roundtrips_hardware() {
    pending_copy_cold_roundtrips(false);
}
#[test]
#[ignore = "hardware pending-copy transition stops normally at retirement and fresh approval, retaining old local pixels"]
fn replacement_pending_copy_stop_barriers_hardware() {
    for stop_at_approval in [false, true] {
        let (mut peer, mut consumer, hello, _) = start(false, Some("replace-remove-pending"));
        let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
        else {
            panic!("missing retained initial frame")
        };
        let (retained, ack) = consumer.copy(header).unwrap();
        let retained = retained.unwrap();
        pixels(retained.surface(), [0, 0, 255, 255]);
        peer.send(ack).unwrap();
        let next = retire(&peer, &mut consumer, hello, (640, 360));
        drop(consumer);
        if stop_at_approval {
            peer.retired_epoch(next).unwrap();
            let Wire::Hello(replacement) = peer.receive(Duration::from_secs(10)).unwrap() else {
                panic!("missing replacement greeting before cancellation")
            };
            assert_eq!(replacement.session, next);
        }
        peer.send(Message::Stop).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            assert!(std::time::Instant::now() < deadline);
            match peer.receive(Duration::from_secs(5)).unwrap() {
                Wire::Control(Message::Bye) => break,
                Wire::Control(Message::Report { .. }) => {}
                other => panic!("canceled pending-copy transition published media: {other:?}"),
            }
        }
        let status = peer.wait_exit_status(Duration::from_secs(5)).unwrap();
        assert!(
            status.success(),
            "pending-copy cancellation was not a normal exit: {status}"
        );
        pixels(retained.surface(), [0, 0, 255, 255]);
    }
}

#[test]
#[ignore = "owned WARP recreation failures after release and before fresh handles"]
fn replacement_injected_failures_warp() {
    injected_failures(true);
}
#[test]
#[ignore = "owned hardware recreation failures after release and before fresh handles"]
fn replacement_injected_failures_hardware() {
    injected_failures(false);
}
fn injected_failures(warp: bool) {
    for fault in ["replace-after-release", "replace-before-hello"] {
        let (mut peer, mut consumer, hello, _budget) = start(warp, Some(fault));
        let next = retire(&peer, &mut consumer, hello, (640, 360));
        drop(consumer);
        peer.retired_epoch(next).unwrap();
        assert!(
            peer.receive(Duration::from_secs(10)).is_err(),
            "failure published fresh media"
        );
        assert!(!peer.wait_exit(Duration::from_secs(5)).unwrap());
    }
}

#[test]
#[ignore = "plain live-resize opt-in does not authorize device recreation"]
fn replacement_requires_distinct_opt_in_warp() {
    let mut peer = Peer::launch_rearm(
        Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture")),
        true,
        None,
    )
    .unwrap();
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing hello")
    };
    peer.approve(hello).unwrap();
    peer.replace_device(ResizeRequest {
        session: hello.session,
        width: 640,
        height: 360,
    })
    .unwrap();
    assert!(!peer.wait_exit(Duration::from_secs(5)).unwrap());
}
