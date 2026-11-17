#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "d3d12-present-hook-fixture"
))]
use capture_hook_prototype::d3d12_cross_process::{Peer, RemoteConsumer};
use capture_hook_prototype::d3d12_present_fixture::D3d12PresentFixture;
use capture_hook_prototype::d3d12_protocol::Wire;
use capture_hook_prototype::local_gpu_bridge::GpuPoolBudget;
use capture_hook_prototype::transport::Message;
use gpu_windows::BgraReadback;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn rejects_non_owned_hook_process_before_window_or_device() {
    assert!(D3d12PresentFixture::new_gpu_present_hook(
        true,
        318,
        178,
        Arc::new(GpuPoolBudget::default())
    )
    .is_err());
}
fn run(warp: bool) {
    run_mode(warp, false);
}
fn launch(warp: bool, factory_hook: bool) -> Peer {
    let path = Path::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture"));
    if factory_hook {
        #[cfg(feature = "d3d12-factory-hook-fixture")]
        {
            return Peer::launch_factory_hook(path, warp, None).unwrap();
        }
        #[cfg(not(feature = "d3d12-factory-hook-fixture"))]
        {
            panic!("factory hook feature required");
        }
    }
    Peer::launch_hook(path, warp, None).unwrap()
}
fn run_mode(warp: bool, factory_hook: bool) {
    let mut peer = launch(warp, factory_hook);
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing hook child greeting")
    };
    let mut consumer =
        RemoteConsumer::open(&peer, hello, warp, Arc::new(GpuPoolBudget::default())).unwrap();
    peer.approve(hello).unwrap();
    let mut held = Vec::new();
    let mut headers = Vec::new();
    for _ in 0..3 {
        let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
        else {
            panic!("missing Present-tapped frame")
        };
        headers.push(header);
    }
    if factory_hook {
        peer.send(Message::Render { count: 1, color: 2 }).unwrap();
        assert_eq!(
            peer.receive(Duration::from_secs(5)).unwrap(),
            Wire::Control(Message::Report {
                presented: 2,
                published: 2,
                dropped: 2
            })
        );
    }
    peer.send(Message::Render { count: 1, color: 3 }).unwrap();
    let Wire::Control(Message::Report {
        presented: before,
        published: captured,
        dropped: unrelated,
    }) = peer.receive(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing detour counters")
    };
    assert_eq!(captured, 3);
    assert!(before >= 3 && unrelated >= 3);
    let deadline = Instant::now() + Duration::from_secs(3);
    let after = loop {
        std::thread::sleep(Duration::from_millis(25));
        peer.send(Message::Render { count: 1, color: 3 }).unwrap();
        let Wire::Control(Message::Report {
            presented: after,
            published: still_captured,
            dropped: unrelated_after,
        }) = peer.receive(Duration::from_secs(5)).unwrap()
        else {
            panic!("missing detour pressure counters")
        };
        assert_eq!(still_captured, 3);
        if after >= before + 12 && unrelated_after >= unrelated + 12 {
            break after;
        }
        assert!(
            Instant::now() < deadline,
            "both selected and unrelated Present forwarding must progress under capture pressure"
        );
    };
    for header in headers {
        let (publication, ack) = consumer.copy(header).unwrap();
        let publication = publication.unwrap();
        let bytes = BgraReadback::default()
            .read_scaled(publication.surface(), 318, 178)
            .unwrap();
        assert!(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 0, 255, 255]));
        held.push(publication);
        peer.send(ack).unwrap();
    }
    consumer.close();
    peer.send(Message::Stop).unwrap();
    let mut final_count = 0;
    loop {
        match peer.receive(Duration::from_secs(5)).unwrap() {
            Wire::Control(Message::Frame(_)) => {}
            Wire::Control(Message::Report { presented, .. }) => final_count = presented,
            Wire::Control(Message::Bye) => break,
            wire => panic!("unexpected hook stop packet {wire:?}"),
        }
    }
    assert!(final_count >= after + 12);
    assert!(peer.wait_exit(Duration::from_secs(5)).unwrap());
    let bytes = BgraReadback::default()
        .read_scaled(held[0].surface(), 318, 178)
        .unwrap();
    assert!(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| *p == [0, 0, 255, 255]));
    println!("PASS: actual owned D3D12 Present detour {}; covered selected-chain pixels; unrelated original Presents; full-ring capture drops; cross-process shared fences; cancellation with continued original Present; retained output after child exit; no unhook/unload.",if warp{"WARP"}else{"hardware"});
}
#[test]
#[ignore = "installs Present detour only in an exact launched owned WARP child"]
fn present_detour_warp() {
    run(true);
}
#[test]
#[ignore = "installs Present detour only in an exact launched owned hardware child"]
fn present_detour_hardware() {
    run(false);
}

fn window_states(warp: bool) {
    window_states_mode(warp, false);
}
fn window_states_mode(warp: bool, factory_hook: bool) {
    use capture_hook_prototype::d3d12_protocol::WindowAction;
    let mut peer = launch(warp, factory_hook);
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing owned hook greeting")
    };
    let mut consumer =
        RemoteConsumer::open(&peer, hello, warp, Arc::new(GpuPoolBudget::default())).unwrap();
    peer.approve(hello).unwrap();
    for action in [
        WindowAction::Minimize,
        WindowAction::Restore,
        WindowAction::Borderless,
        WindowAction::Minimize,
        WindowAction::Restore,
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
                    }
                    break published as i64;
                }
                wire => panic!("unexpected window action packet {wire:?}"),
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
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while fresh < 3 {
            assert!(
                std::time::Instant::now() < deadline,
                "window transition stopped fresh capture"
            );
            let Wire::Control(Message::Frame(header)) =
                peer.receive(Duration::from_secs(5)).unwrap()
            else {
                panic!("missing post-transition frame")
            };
            let (publication, ack) = consumer.copy(header).unwrap();
            if header.qpc_ticks >= marker {
                if let Some(publication) = &publication {
                    let pixels = BgraReadback::default()
                        .read_scaled(publication.surface(), 318, 178)
                        .unwrap();
                    assert!(
                        pixels.as_chunks::<4>().0.iter().all(|p| *p == expected),
                        "stale/unrelated pixels after {action:?}"
                    );
                    fresh += 1;
                }
            }
            drop(publication);
            peer.send(ack).unwrap();
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
    println!("PASS: owned D3D12 Present hook {}: minimize/restore, observed monitor-sized borderless HWND, minimize/restore while borderless, windowed return; three fresh QPC-after-action frames with changed expected colors per phase. Fixed backbuffer; NOT FSE/Alt-Tab/game evidence.",if warp{"WARP"}else{"hardware"});
}
#[test]
#[ignore = "changes only owned WARP fixture windows between minimized, borderless and windowed states"]
fn window_states_warp() {
    window_states(true);
}
#[test]
#[ignore = "changes only owned hardware fixture windows between minimized, borderless and windowed states"]
fn window_states_hardware() {
    window_states(false);
}

fn short_soak(warp: bool) {
    short_soak_mode(warp, false);
}
fn short_soak_mode(warp: bool, factory_hook: bool) {
    let mut peer = launch(warp, factory_hook);
    let Wire::Hello(hello) = peer.receive(Duration::from_secs(10)).unwrap() else {
        panic!("missing hook greeting")
    };
    let mut consumer =
        RemoteConsumer::open(&peer, hello, warp, Arc::new(GpuPoolBudget::default())).unwrap();
    peer.approve(hello).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    for index in 0..500 {
        assert!(
            std::time::Instant::now() < deadline,
            "short hook soak exceeded its bounded fixture budget"
        );
        let Wire::Control(Message::Frame(header)) = peer.receive(Duration::from_secs(5)).unwrap()
        else {
            panic!("missing soak frame")
        };
        let (publication, ack) = consumer.copy(header).unwrap();
        if index % 50 == 0 {
            let publication = publication
                .as_ref()
                .expect("no output on sampled soak frame");
            let pixels = BgraReadback::default()
                .read_scaled(publication.surface(), 318, 178)
                .unwrap();
            assert!(pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [0, 0, 255, 255]));
        }
        drop(publication);
        peer.send(ack).unwrap();
        if index == 100 || index == 300 {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    assert_eq!(consumer.gpu_copies, 500);
    consumer.close();
    peer.send(Message::Stop).unwrap();
    loop {
        if peer.receive(Duration::from_secs(5)).unwrap() == Wire::Control(Message::Bye) {
            break;
        }
    }
    assert!(peer.wait_exit(Duration::from_secs(5)).unwrap());
    println!("PASS: short owned D3D12 hook {} soak: 500 cross-process GPU publications, two consumer stalls, ten explicit pixel checks, then cancellation/child cleanup. Not a performance or long-duration benchmark.",if warp{"WARP"}else{"hardware"});
}
#[test]
#[ignore = "short 500-frame owned WARP Present-hook transport soak"]
fn short_soak_warp() {
    short_soak(true);
}
#[test]
#[ignore = "short 500-frame owned hardware Present-hook transport soak"]
fn short_soak_hardware() {
    short_soak(false);
}

#[cfg(feature = "d3d12-factory-hook-fixture")]
mod factory {
    use super::*;
    #[test]
    fn rejects_factory_hook_outside_exact_child() {
        assert!(D3d12PresentFixture::new_gpu_factory_hook(
            true,
            318,
            178,
            Arc::new(GpuPoolBudget::default())
        )
        .is_err());
    }
    fn probe(warp: bool) {
        probe_mode(warp, false);
    }
    fn probe_mode(warp: bool, legacy: bool) {
        let mut command =
            std::process::Command::new(env!("CARGO_BIN_EXE_silk-d3d12-owned-fixture"));
        command.args([
            "--allow-owned-d3d12",
            "--present-hook",
            "--factory-hook",
            "--factory-hook-probe",
        ]);
        if warp {
            command.arg("--warp");
        }
        if legacy {
            command.arg("--legacy-factory");
        }
        // The probe has no IPC/UI wait and must be bounded by the process runner.
        let mut child = command.spawn().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("owned factory probe timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    #[ignore = "actual factory interception and bounded ownership faults in exact owned WARP child"]
    fn factory_probe_warp() {
        probe(true);
    }
    #[test]
    #[ignore = "actual factory interception and bounded ownership faults in exact owned hardware child"]
    fn factory_probe_hardware() {
        probe(false);
    }
    #[test]
    #[ignore = "legacy factory failed/successful forwarding, bounds and ownership on owned WARP"]
    fn legacy_factory_probe_warp() {
        probe_mode(true, true);
    }
    #[test]
    #[ignore = "legacy factory failed/successful forwarding, bounds and ownership on owned hardware"]
    fn legacy_factory_probe_hardware() {
        probe_mode(false, true);
    }
    #[test]
    #[ignore = "owned factory-observed queue and actual Present detour on WARP"]
    fn factory_capture_warp() {
        run_mode(true, true);
    }
    #[test]
    #[ignore = "owned factory-observed queue and actual Present detour on hardware"]
    fn factory_capture_hardware() {
        run_mode(false, true);
    }
    #[test]
    #[ignore = "owned factory/Present hook minimize and borderless transitions on WARP"]
    fn factory_window_states_warp() {
        window_states_mode(true, true);
    }
    #[test]
    #[ignore = "owned factory/Present hook minimize and borderless transitions on hardware"]
    fn factory_window_states_hardware() {
        window_states_mode(false, true);
    }
    #[test]
    #[ignore = "owned factory/Present hooks 500-publication short WARP soak"]
    fn factory_short_soak_warp() {
        short_soak_mode(true, true);
    }
    #[test]
    #[ignore = "owned factory/Present hooks 500-publication short hardware soak"]
    fn factory_short_soak_hardware() {
        short_soak_mode(false, true);
    }
}
