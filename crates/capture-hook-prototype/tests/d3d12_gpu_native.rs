#![cfg(all(windows, target_arch = "x86_64", feature = "d3d12-gpu-fixture"))]
use capture_hook_prototype::d3d12_gpu_bridge::GpuInteropFrame;
use capture_hook_prototype::d3d12_present_fixture::D3d12PresentFixture;
use capture_hook_prototype::local_gpu_bridge::GpuPoolBudget;
use gpu_windows::{BgraReadback, GpuTextureSlot};
use media_types::{FramePayload, GpuFrameHandle, PixelFormat, StreamId, TimeBase, VideoFrame};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn video(frame: &GpuInteropFrame) -> VideoFrame {
    VideoFrame {
        stream_id: StreamId(0),
        width: frame.width,
        height: frame.height,
        pixel_format: PixelFormat::Bgra8,
        pts: frame.sequence as i64 * 33,
        time_base: TimeBase::MILLISECOND,
        payload: FramePayload::Gpu(GpuFrameHandle(frame.publication.handle())),
    }
}
fn check(surface: &GpuTextureSlot) {
    // Explicit test-only readback; not part of capture or the hardware replay route.
    let pixels = BgraReadback::default()
        .read_scaled(surface, 318, 178)
        .unwrap();
    assert!(pixels
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == [0, 0, 255, 255]));
}
fn lifecycle(warp: bool) {
    let budget = Arc::new(GpuPoolBudget::default());
    let mut fixture = D3d12PresentFixture::new_gpu(warp, 318, 178, budget.clone()).unwrap();
    fixture.cover_selected().unwrap();
    assert!(fixture.poll().is_err()); // never a silent CPU fallback
    let context = fixture.gpu_context().unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut held = Vec::with_capacity(3);
    while held.len() < 3 {
        fixture.step().unwrap();
        if let Some(frame) = fixture.poll_gpu().unwrap() {
            check(frame.publication.surface());
            held.push(frame);
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(held
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence
            && pair[0].submitted_at <= pair[1].submitted_at));
    let frame = video(&held[0]);
    let lease = context.resolve(&frame).unwrap();
    drop(held.remove(0));
    assert!(context.resolve(&frame).is_err());
    let before = fixture.stats();
    let deadline = Instant::now() + Duration::from_secs(8);
    while fixture.stats().selected < before.selected + 20
        || fixture.stats().unrelated < before.unrelated + 20
    {
        fixture.step().unwrap();
        assert!(fixture.poll_gpu().unwrap().is_none());
        assert!(
            Instant::now() < deadline,
            "retained GPU leases blocked rendering"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let gpu = fixture.gpu_stats().unwrap();
    assert_eq!(gpu.gpu_copies_to_output, 3);
    assert!(gpu.dropped_local_full > 0);
    assert_eq!(gpu.cpu_readbacks, 0);
    check(lease.downcast_ref::<GpuTextureSlot>().unwrap());
    // Resize drains both APIs, retires publication and leaves rendering running.
    fixture.resize_selected(640, 360).unwrap();
    assert!(fixture.poll_gpu().is_err());
    assert!(fixture.gpu_context().is_none());
    assert!(context.resolve(&video(&held[0])).is_err());
    let before = fixture.stats();
    let deadline = Instant::now() + Duration::from_secs(8);
    while fixture.stats().selected < before.selected + 12
        || fixture.stats().unrelated < before.unrelated + 12
    {
        fixture.step().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    drop(fixture);
    check(lease.downcast_ref::<GpuTextureSlot>().unwrap());
    // Retained contexts use the existing six-local-surface restart budget.
    let second = D3d12PresentFixture::new_gpu(warp, 318, 178, budget.clone()).unwrap();
    let second_context = second.gpu_context().unwrap();
    drop(second);
    match D3d12PresentFixture::new_gpu(warp, 318, 178, budget.clone()) {
        Ok(_) => panic!("retained contexts bypassed GPU budget"),
        Err(e) => assert!(e.contains("six-surface"), "{e}"),
    }
    drop(second_context);
    let mut fresh = D3d12PresentFixture::new_gpu(warp, 318, 178, budget).unwrap();
    fresh.step().unwrap();
    drop(fresh);
    println!("PASS: D3D12 shared textures -> D3D11 GPU output, {}: red source isolation; retained publications/leases; local saturation with both Present paths progressing; resize retirement; stale handles; bounded restart; in-flight teardown. stats={gpu:?}. Pixel readbacks were test-only.", if warp {"WARP"} else {"hardware adapter 0"});
}
#[test]
#[ignore = "opens owned WARP swap chains and requires D3D11/D3D12 shared-fence interop"]
fn gpu_interop_warp() {
    lifecycle(true);
}
#[test]
#[ignore = "opens owned hardware swap chains and requires D3D11/D3D12 shared-fence interop"]
fn gpu_interop_hardware() {
    lifecycle(false);
}
