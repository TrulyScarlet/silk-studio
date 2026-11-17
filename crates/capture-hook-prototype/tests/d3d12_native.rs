#![cfg(all(windows, target_arch = "x86_64", feature = "d3d12-fixture"))]
use capture_hook_prototype::d3d12_fixture::{D3d12Fixture, FixtureFrame};
use std::time::{Duration, Instant};

fn receive(fixture: &mut D3d12Fixture) -> FixtureFrame {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(frame) = fixture.poll().unwrap() {
            return frame;
        }
        assert!(Instant::now() < deadline, "D3D12 completion deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn color(frame: &FixtureFrame, expected: [u8; 4]) {
    assert_eq!(
        frame.bgra.len(),
        frame.width as usize * frame.height as usize * 4
    );
    assert!(frame
        .bgra
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == expected));
}
fn lifecycle(warp: bool) {
    // Width 318 deliberately requires D3D12 row padding; fresh size/device epochs.
    for (width, height) in [(318, 178), (640, 480), (800, 450)] {
        let mut fixture = D3d12Fixture::new(warp, width, height).unwrap();
        assert!(D3d12Fixture::new(warp, width, height).is_err());
        assert!(fixture.submit_color([f32::NAN, 0., 0., 1.]).is_err());
        for rgba in [[1., 0., 0., 1.], [0., 1., 0., 1.], [0., 0., 1., 1.]] {
            assert!(fixture.submit_color(rgba).unwrap());
        }
        assert!(!fixture.submit_color([1.; 4]).unwrap());
        let red = receive(&mut fixture);
        let green = receive(&mut fixture);
        let blue = receive(&mut fixture);
        assert_eq!([red.sequence, green.sequence, blue.sequence], [1, 2, 3]);
        assert!(red.submitted_at <= green.submitted_at && green.submitted_at <= blue.submitted_at);
        assert_eq!((red.width, red.height), (width, height));
        color(&red, [0, 0, 255, 255]);
        color(&green, [0, 255, 0, 255]);
        color(&blue, [255, 0, 0, 255]);
        // Fence completion alone cannot recycle any slot with a retained lease.
        for _ in 0..12 {
            assert!(!fixture.submit_color([1.; 4]).unwrap());
        }
        assert!(fixture.poll().unwrap().is_none());
        drop(green);
        assert!(fixture.submit_color([1.; 4]).unwrap());
        let white = receive(&mut fixture);
        assert_eq!(white.sequence, 4);
        color(&white, [255; 4]);
        color(&red, [0, 0, 255, 255]);
        color(&blue, [255, 0, 0, 255]);
        fixture.cancel();
        assert!(fixture.submit_color([1.; 4]).is_err());
        assert!(fixture.poll().is_err());
        drop(fixture);
        color(&red, [0, 0, 255, 255]);
        color(&white, [255; 4]);
    }
    // Teardown must drain submissions even when they were never polled/mapped.
    let mut fixture = D3d12Fixture::new(warp, 320, 180).unwrap();
    assert!(fixture.submit_color([0., 1., 0., 1.]).unwrap());
    drop(fixture);
    println!("PASS: owned D3D12 {} clear/copy/fence/readback; padded rows, three-slot pressure, retained pixels, cancellation, sequential size/device recreation and in-flight teardown. CPU readback; no hook/fullscreen/game test.", if warp {"WARP"} else {"hardware adapter 0"});
}
#[test]
fn rejects_dimensions_before_device_creation() {
    for (width, height) in [
        (0, 1),
        (1, 0),
        (1921, 1080),
        (1920, 1081),
        (u32::MAX, u32::MAX),
    ] {
        assert!(D3d12Fixture::new(true, width, height).is_err());
    }
}
#[test]
#[ignore = "uses real D3D12 WARP resources"]
fn lifecycle_warp() {
    lifecycle(true);
}
#[test]
#[ignore = "uses real D3D12 hardware adapter 0"]
fn lifecycle_hardware() {
    lifecycle(false);
}

fn presentation(warp: bool) {
    use capture_hook_prototype::d3d12_present_fixture::D3d12PresentFixture;
    assert_eq!(owned_present_windows(), 0);
    let mut fixture = D3d12PresentFixture::new(warp, 318, 178).unwrap();
    assert_eq!(owned_present_windows(), 2);
    fixture.cover_selected().unwrap();
    assert_eq!(fixture.selected_dimensions().unwrap(), (318, 178));
    let mut held = Vec::with_capacity(3);
    let deadline = Instant::now() + Duration::from_secs(8);
    while held.len() < 3 {
        fixture.step().unwrap();
        if let Some(frame) = fixture.poll().unwrap() {
            color(&frame, [0, 0, 255, 255]);
            held.push(frame);
        }
        assert!(
            Instant::now() < deadline,
            "selected swap-chain capture deadline"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        held.iter().map(|frame| frame.sequence).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let before = fixture.stats();
    let deadline = Instant::now() + Duration::from_secs(8);
    while fixture.stats().selected < before.selected + 12
        || fixture.stats().unrelated < before.unrelated + 12
    {
        fixture.step().unwrap();
        assert!(
            Instant::now() < deadline,
            "full capture ring blocked presentation"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(fixture.stats().captured, 3);
    assert!(fixture.stats().dropped_capture >= 12);
    assert!(fixture.poll().unwrap().is_none());
    fixture.cancel_capture();
    assert!(fixture.poll().is_err());
    let before = fixture.stats();
    let deadline = Instant::now() + Duration::from_secs(8);
    while fixture.stats().selected < before.selected + 12
        || fixture.stats().unrelated < before.unrelated + 12
    {
        fixture.step().unwrap();
        assert!(
            Instant::now() < deadline,
            "capture cancellation blocked rendering"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(fixture.stats().selected >= before.selected + 12);
    assert!(fixture.stats().unrelated >= before.unrelated + 12);
    fixture.resize_selected(640, 360).unwrap();
    assert_eq!(fixture.selected_dimensions().unwrap(), (640, 360));
    let before = fixture.stats();
    let deadline = Instant::now() + Duration::from_secs(8);
    while fixture.stats().selected < before.selected + 12
        || fixture.stats().unrelated < before.unrelated + 12
    {
        fixture.step().unwrap();
        assert!(Instant::now() < deadline, "resized renderer stopped");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(fixture.stats().captured, 3); // resize cannot silently rearm capture
    assert!(fixture.poll().is_err());
    let stats = fixture.stats();
    drop(fixture);
    assert_eq!(owned_present_windows(), 0);
    for frame in held {
        color(&frame, [0, 0, 255, 255]);
    }
    // A fresh presentation instance starts a fresh capture contract after teardown.
    let mut fresh = D3d12PresentFixture::new(warp, 640, 360).unwrap();
    fresh.step().unwrap();
    fresh.resize_selected(800, 450).unwrap(); // resize with active/unpolled capture
    assert!(fresh.poll().is_err());
    assert_eq!(fresh.selected_dimensions().unwrap(), (800, 450));
    fresh.step().unwrap(); // destroy with potentially unpolled rendering work
    drop(fresh);
    assert_eq!(owned_present_windows(), 0);
    println!("PASS: owned D3D12 swap-chain {} selected red/unrelated blue; 3 retained captures; both Present paths progress under full ring and cancellation; real ResizeBuffers with terminal capture; retained CPU pixels; fresh instance and in-flight teardown. stats={stats:?}. Windowed CPU readback only.", if warp {"WARP"} else {"hardware adapter 0"});
}

fn owned_present_windows() -> usize {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId,
    };
    unsafe extern "system" fn visit(window: HWND, context: LPARAM) -> BOOL {
        let count = &mut *(context.0 as *mut usize);
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == std::process::id() {
            let mut class = [0u16; 128];
            let length = GetClassNameW(window, &mut class) as usize;
            if String::from_utf16_lossy(&class[..length]) == "SilkOwnedD3d12PresentationFixture" {
                *count += 1;
            }
        }
        true.into()
    }
    let mut count = 0usize;
    unsafe {
        EnumWindows(Some(visit), LPARAM(&mut count as *mut usize as isize)).unwrap();
    }
    count
}
#[test]
#[ignore = "opens two owned windowed D3D12 WARP swap chains"]
fn presentation_warp() {
    presentation(true);
}
#[test]
#[ignore = "opens two owned windowed D3D12 hardware swap chains"]
fn presentation_hardware() {
    presentation(false);
}
