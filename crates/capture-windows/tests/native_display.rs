//! Native display-capture verification (spec §27.4 subset, M2 exit
//! criteria). These tests touch REAL hardware and require an interactive
//! desktop session with Windows Graphics Capture support (Win10 1903+),
//! so they are `#[ignore]`-tagged for CI; run locally with:
//!
//! ```text
//! cargo test -p capture-windows --release -- --ignored
//! ```

#![cfg(windows)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
use capture_windows::{enumerate_displays, GpuTextureSlot, WgcDisplayCapture};
use encoder_api::GpuFrameResolveError;
use gpu_windows::{D3D11VideoConverter, GpuDeviceContext};
use media_types::{FramePayload, StreamId};
use windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12;

fn config(source_id: &str, fps: u32) -> VideoCaptureConfig {
    VideoCaptureConfig {
        source_id: source_id.to_string(),
        target_fps: fps,
    }
}

#[test]
#[ignore = "requires an attached display"]
fn enumeration_reports_at_least_one_display() {
    let sources = enumerate_displays().expect("display enumeration");
    assert!(!sources.is_empty(), "no attached displays found");
    assert!(
        sources.iter().any(|s| s.is_primary),
        "no primary display flagged"
    );
    for source in &sources {
        assert!(!source.id.is_empty());
        assert!(!source.name.is_empty());
    }
}

/// Unknown source ids must fail fast without hanging or panicking.
/// Safe on any machine (no session is created on failure paths).
#[test]
fn unknown_source_id_fails_cleanly() {
    let mut capture = WgcDisplayCapture::new(StreamId(0));
    let result = capture.start(config("silk-nonexistent-display", 60));
    assert!(result.is_err(), "unknown source must not start");
}

#[test]
#[ignore = "requires interactive desktop + WGC support"]
fn double_start_rejected_and_stop_is_idempotent() {
    let sources = enumerate_displays().expect("enumeration");
    let mut capture = WgcDisplayCapture::new(StreamId(0));
    capture
        .start(config(&sources[0].id, 60))
        .expect("first start");
    let second = capture.start(config(&sources[0].id, 60));
    assert!(second.is_err(), "second start must be rejected");
    capture.stop().expect("stop");
    capture.stop().expect("stop must be idempotent");
}

#[test]
#[ignore = "requires interactive desktop + WGC support"]
fn captures_monotonic_frames_from_primary_display() {
    let sources = enumerate_displays().expect("enumeration");
    let primary = sources
        .iter()
        .find(|s| s.is_primary)
        .expect("primary display");

    let mut capture = WgcDisplayCapture::new(StreamId(0));
    capture
        .start(config(&primary.id, 60))
        .expect("start capture");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut frames = 0_u32;
    let mut last_pts: Option<i64> = None;
    let mut size: Option<(u32, u32)> = None;

    while Instant::now() < deadline && frames < 90 {
        match capture.next_event() {
            Ok(VideoCaptureEvent::Frame(frame)) => {
                assert!(frame.width > 0 && frame.height > 0, "degenerate frame size");
                if let Some((w, h)) = size {
                    assert_eq!((frame.width, frame.height), (w, h), "size changed mid-run");
                }
                size = Some((frame.width, frame.height));
                if let Some(last) = last_pts {
                    assert!(
                        frame.pts >= last,
                        "timestamps must be non-decreasing: {last} -> {}",
                        frame.pts
                    );
                }
                last_pts = Some(frame.pts);
                frames += 1;
            }
            Ok(VideoCaptureEvent::FormatChanged { .. }) => {}
            Ok(VideoCaptureEvent::SourceLost) => panic!("source lost during healthy run"),
            Err(err) => panic!("capture error after {frames} frames: {err}"),
        }
    }

    capture.stop().expect("stop");

    // A static desktop produces sparse compositor frames; require proof of
    // life rather than a strict fps floor (documented in progress log).
    assert!(
        frames >= 5,
        "expected live frames from compositor, got {frames}"
    );

    let (copied, dropped, _resizes) = capture.stats_snapshot().expect("stats");
    assert!(
        copied >= frames as u64,
        "handler copied {copied}, delivered {frames}"
    );
    // Queue bound respected: drops only under sustained overload.
    assert!(
        dropped <= copied / 4,
        "excessive queue drops: {dropped}/{copied}"
    );
}

#[test]
#[ignore = "requires interactive desktop + WGC support"]
fn gpu_context_resolves_a_staged_texture() {
    let sources = enumerate_displays().expect("enumeration");
    let primary = sources
        .iter()
        .find(|s| s.is_primary)
        .expect("primary display");

    let mut capture = WgcDisplayCapture::new(StreamId(0));
    capture
        .start(config(&primary.id, 60))
        .expect("start capture");
    let context = capture.gpu_frame_context().expect("active GPU context");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut resolved = false;
    while Instant::now() < deadline {
        match capture.next_event() {
            Ok(VideoCaptureEvent::Frame(frame)) => {
                let handle = match &frame.payload {
                    FramePayload::Gpu(handle) => *handle,
                    FramePayload::Cpu(_) => panic!("native WGC frame must be GPU-backed"),
                };
                match context.resolve(&frame) {
                    Ok(lease) => {
                        assert_eq!(lease.handle(), handle);
                        assert!(
                            lease.downcast_ref::<GpuTextureSlot>().is_some(),
                            "WGC resolver must return its D3D11 texture resource"
                        );
                        resolved = true;
                        break;
                    }
                    Err(GpuFrameResolveError::StaleHandle { .. }) => {
                        // The bounded registry may evict a queued event before
                        // the consumer resolves it; wait for a newer frame.
                    }
                    Err(err) => panic!("GPU resource resolution failed: {err}"),
                }
            }
            Ok(VideoCaptureEvent::FormatChanged { .. }) => {}
            Ok(VideoCaptureEvent::SourceLost) => panic!("source lost during healthy run"),
            Err(err) => panic!("capture error while resolving GPU frame: {err}"),
        }
    }
    capture.stop().expect("stop");

    assert!(resolved, "no staged GPU texture was resolved");
}

#[test]
#[ignore = "requires interactive desktop + WGC + D3D11 video processor"]
fn gpu_video_processor_converts_wgc_frame_to_nv12() {
    let sources = enumerate_displays().expect("enumeration");
    let primary = sources
        .iter()
        .find(|s| s.is_primary)
        .expect("primary display");

    let mut capture = WgcDisplayCapture::new(StreamId(0));
    capture
        .start(config(&primary.id, 60))
        .expect("start capture");
    let context = capture.gpu_frame_context().expect("active GPU context");
    let device = Arc::downcast::<GpuDeviceContext>(
        context.session_resource().expect("session GPU resource"),
    )
    .expect("D3D11 session resource");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut converted = false;
    while Instant::now() < deadline {
        match capture.next_event() {
            Ok(VideoCaptureEvent::Frame(frame)) => {
                let lease = context.resolve(&frame).expect("resolve captured texture");
                let slot = lease
                    .downcast_ref::<GpuTextureSlot>()
                    .expect("D3D11 texture slot");
                let converter = D3D11VideoConverter::new(
                    Arc::clone(&device),
                    frame.width,
                    frame.height,
                    1920,
                    1080,
                    60,
                )
                .expect("create D3D11 video converter");
                let output = converter.convert(slot).expect("convert BGRA to NV12");
                let mut description = D3D11_TEXTURE2D_DESC::default();
                unsafe {
                    output.texture().GetDesc(&mut description);
                }
                assert_eq!(description.Format, DXGI_FORMAT_NV12);
                assert_eq!((description.Width, description.Height), (1920, 1080));
                converted = true;
                break;
            }
            Ok(VideoCaptureEvent::FormatChanged { .. }) => {}
            Ok(VideoCaptureEvent::SourceLost) => panic!("source lost during conversion"),
            Err(error) => panic!("capture error during conversion: {error}"),
        }
    }
    capture.stop().expect("stop capture");

    assert!(converted, "no WGC frame was converted");
}

#[test]
#[ignore = "requires interactive desktop + WGC support"]
fn fps_shaping_30_delivers_halved_cadence() {
    let sources = enumerate_displays().expect("enumeration");
    let primary = sources
        .iter()
        .find(|s| s.is_primary)
        .expect("primary display");

    let mut capture = WgcDisplayCapture::new(StreamId(0));
    capture.start(config(&primary.id, 30)).expect("start @30");

    let mut intervals_ms: Vec<i64> = Vec::new();
    let mut last_pts: Option<i64> = None;
    let deadline = Instant::now() + Duration::from_secs(8);

    while Instant::now() < deadline && intervals_ms.len() < 40 {
        if let Ok(VideoCaptureEvent::Frame(frame)) = capture.next_event() {
            if let Some(last) = last_pts {
                intervals_ms.push(frame.pts - last);
            }
            last_pts = Some(frame.pts);
        }
    }
    capture.stop().expect("stop");

    assert!(intervals_ms.len() >= 20, "not enough shaped frames");
    // Shaped cadence should hover around ~33 ms; allow jitter but reject
    // an essentially-unshaped stream (majority of gaps < 20 ms).
    let short_gaps = intervals_ms.iter().filter(|d| **d < 20).count();
    assert!(
        short_gaps * 3 <= intervals_ms.len(),
        "{short_gaps}/{} gaps below 20 ms — 30 fps shaping ineffective",
        intervals_ms.len()
    );
}

#[test]
#[ignore = "requires interactive desktop + WGC support"]
fn post_stop_next_event_ends_promptly() {
    let sources = enumerate_displays().expect("enumeration");
    let mut capture = WgcDisplayCapture::new(StreamId(0));
    capture.start(config(&sources[0].id, 60)).expect("start");
    // Give the session a beat to produce something.
    std::thread::sleep(Duration::from_millis(300));
    capture.stop().expect("stop");

    // Contract: after stop, next_event returns *some* error promptly —
    // never blocks indefinitely (EndOfStream or not-started are both fine).
    let started = Instant::now();
    let result = capture.next_event();
    assert!(
        result.is_err(),
        "next_event after stop must fail, got event"
    );
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "next_event must end promptly after stop"
    );
}
