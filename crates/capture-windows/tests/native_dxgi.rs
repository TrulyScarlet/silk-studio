#![cfg(windows)]
//! Opt-in hardware checks. Run with --ignored --test-threads=1 on a live desktop.
use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
use capture_windows::{enumerate_displays, DisplayCapture, DxgiDisplayCapture, GpuTextureSlot};
use media_types::StreamId;
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct3D11::*;

#[test]
#[ignore = "requires interactive desktop and a working Windows capture backend"]
fn auto_reports_actual_dimensions_before_delivering_video() {
    let mut capture = DisplayCapture::new(StreamId(0));
    capture
        .start(VideoCaptureConfig {
            source_id: "auto".into(),
            target_fps: 60,
        })
        .unwrap();
    let (width, height) = match capture.next_event().unwrap() {
        VideoCaptureEvent::FormatChanged { width, height } => (width, height),
        _ => panic!("Auto must announce selected monitor dimensions before frames"),
    };
    assert!(width > 0 && height > 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = false;
    while Instant::now() < deadline {
        match capture.next_event().unwrap() {
            VideoCaptureEvent::Frame(frame) => {
                assert_eq!((frame.width, frame.height), (width, height));
                assert!(capture.gpu_frame_context().unwrap().resolve(&frame).is_ok());
                received = true;
                break;
            }
            VideoCaptureEvent::SourceLost => panic!("foreground display changed during test"),
            _ => {}
        }
    }
    capture.stop().unwrap();
    assert!(received, "Auto did not deliver video");
}

fn readback(slot: &GpuTextureSlot) -> Vec<u8> {
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        slot.texture().GetDesc(&mut desc);
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let gpu = slot.device_context();
        let mut staging = None;
        gpu.device()
            .CreateTexture2D(&desc, None, Some(&mut staging))
            .unwrap();
        let staging = staging.unwrap();
        gpu.context().CopyResource(&staging, slot.texture());
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        gpu.context()
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .unwrap();
        let mut pixels = Vec::with_capacity((desc.Width * desc.Height * 4) as usize);
        for y in 0..desc.Height {
            let row = mapped
                .pData
                .cast::<u8>()
                .add((y * mapped.RowPitch) as usize);
            pixels.extend_from_slice(std::slice::from_raw_parts(row, desc.Width as usize * 4));
        }
        gpu.context().Unmap(&staging, 0);
        pixels
    }
}

fn save_bmp(name: &str, width: u32, height: u32, pixels: &[u8]) {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/dxgi-evidence");
    std::fs::create_dir_all(&directory).unwrap();
    let mut bmp = Vec::new();
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54_u32.to_le_bytes());
    bmp.extend_from_slice(&40_u32.to_le_bytes());
    bmp.extend_from_slice(&width.to_le_bytes());
    bmp.extend_from_slice(&(-(height as i32)).to_le_bytes());
    bmp.extend_from_slice(&1_u16.to_le_bytes());
    bmp.extend_from_slice(&32_u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    bmp.extend_from_slice(pixels);
    std::fs::write(directory.join(name), bmp).unwrap();
}

#[test]
#[ignore = "requires interactive desktop and hardware Desktop Duplication"]
fn each_monitor_has_timed_frames_and_immutable_gpu_leases() {
    let sources = enumerate_displays().unwrap();
    let mut capture = DxgiDisplayCapture::new(StreamId(0));
    for (index, source) in sources.iter().enumerate() {
        for fps in [30, 60, 120] {
            capture
                .start(VideoCaptureConfig {
                    source_id: source.id.clone(),
                    target_fps: fps,
                })
                .unwrap();
            let context = capture.gpu_frame_context().unwrap();
            let first = match capture.next_event().unwrap() {
                VideoCaptureEvent::Frame(f) => f,
                other => panic!("{other:?}"),
            };
            assert_eq!((first.width, first.height), (source.width, source.height));
            let lease = context.resolve(&first).unwrap();
            let pixels = readback(lease.downcast_ref::<GpuTextureSlot>().unwrap());
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[..3] != [0, 0, 0]),
                "capture must contain visible desktop pixels"
            );
            let started = Instant::now();
            let mut last_pts = first.pts;
            let mut count = 0;
            while started.elapsed() < Duration::from_secs(2) {
                let VideoCaptureEvent::Frame(frame) = capture.next_event().unwrap() else {
                    panic!("source lost")
                };
                assert!(frame.pts > last_pts, "timestamps must strictly advance");
                last_pts = frame.pts;
                context
                    .resolve(&frame)
                    .expect("queued frame remains resolvable");
                count += 1;
            }
            assert!(
                count >= fps,
                "expected at least half target cadence, got {count} in 2s at {fps}"
            );
            assert_eq!(
                pixels,
                readback(lease.downcast_ref::<GpuTextureSlot>().unwrap()),
                "leased GPU texture was overwritten"
            );
            save_bmp(
                &format!("display-{index}-{fps}.bmp"),
                first.width,
                first.height,
                &pixels,
            );
            capture.stop().unwrap();
            assert!(context.resolve(&first).is_err());
            assert_eq!(
                pixels,
                readback(lease.downcast_ref::<GpuTextureSlot>().unwrap()),
                "stop invalidated an existing lease"
            );
            println!(
                "display {index}: fps={fps}, frames={count}, pts={last_pts}, immutable lease PASS"
            );
        }
    }
}

#[test]
#[ignore = "requires interactive desktop and hardware Desktop Duplication"]
fn auto_selects_dxgi_and_restarts_cleanly() {
    let mut capture = DisplayCapture::new(StreamId(0));
    for _ in 0..3 {
        capture
            .start(VideoCaptureConfig {
                source_id: "default".into(),
                target_fps: 60,
            })
            .unwrap();
        assert_eq!(capture.backend_name(), Some("dxgi"));
        assert!(matches!(
            capture.next_event().unwrap(),
            VideoCaptureEvent::Frame(_)
        ));
        let stop = Instant::now();
        capture.stop().unwrap();
        assert!(stop.elapsed() < Duration::from_secs(1));
    }
}
