//! Capture probe / soak driver: captures the primary display for N
//! seconds, printing live stats. Used by `scripts/soak-display.ps1` for
//! the M2 endurance criterion and handy for manual verification.
//!
//! ```text
//! cargo run -p capture-windows --example capture_probe -- --seconds 3600
//! ```

use std::time::{Duration, Instant};

use capture_api::{VideoCapture, VideoCaptureConfig};
use capture_windows::{
    enumerate_displays, DisplayCapture, GuardedWindowCapture, WgcDisplayCapture,
};
use media_types::StreamId;

fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

enum ProbeCapture {
    Wgc(WgcDisplayCapture),
    Display(DisplayCapture),
    Guarded(GuardedWindowCapture),
}

impl ProbeCapture {
    fn stats_snapshot(&self) -> Option<(u64, u64, u64)> {
        match self {
            Self::Wgc(wgc) => wgc.stats_snapshot(),
            _ => None,
        }
    }
}

impl VideoCapture for ProbeCapture {
    fn enumerate_sources(&self) -> capture_api::Result<Vec<media_types::VideoSourceInfo>> {
        match self {
            Self::Wgc(c) => c.enumerate_sources(),
            Self::Display(c) => c.enumerate_sources(),
            Self::Guarded(c) => c.enumerate_sources(),
        }
    }
    fn start(&mut self, config: VideoCaptureConfig) -> capture_api::Result<()> {
        match self {
            Self::Wgc(c) => c.start(config),
            Self::Display(c) => c.start(config),
            Self::Guarded(c) => c.start(config),
        }
    }
    fn next_event(&mut self) -> capture_api::Result<capture_api::VideoCaptureEvent> {
        match self {
            Self::Wgc(c) => c.next_event(),
            Self::Display(c) => c.next_event(),
            Self::Guarded(c) => c.next_event(),
        }
    }
    fn stop(&mut self) -> capture_api::Result<()> {
        match self {
            Self::Wgc(c) => c.stop(),
            Self::Display(c) => c.stop(),
            Self::Guarded(c) => c.stop(),
        }
    }
    fn gpu_frame_context(&self) -> Option<encoder_api::GpuFrameContext> {
        match self {
            Self::Wgc(c) => c.gpu_frame_context(),
            Self::Display(c) => c.gpu_frame_context(),
            Self::Guarded(c) => c.gpu_frame_context(),
        }
    }
}

fn main() {
    let seconds: u64 = arg_value("--seconds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let fps: u32 = arg_value("--fps")
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);

    let source_id = if let Some(s) = arg_value("--source") {
        s
    } else {
        let sources = match enumerate_displays() {
            Ok(sources) => sources,
            Err(err) => {
                eprintln!("enumeration failed: {err}");
                std::process::exit(1);
            }
        };
        let source = sources.iter().find(|s| s.is_primary).unwrap_or(&sources[0]);
        source.id.clone()
    };
    println!("capturing '{}' at {} fps for {seconds}s...", source_id, fps);

    let mut capture = if source_id.starts_with("game-guarded:") || source_id.starts_with("guarded:")
    {
        ProbeCapture::Guarded(GuardedWindowCapture::new(StreamId(0)))
    } else if source_id.starts_with("pid:")
        || source_id.starts_with("hwnd:")
        || source_id.starts_with("window:")
        || source_id.starts_with("game:")
    {
        ProbeCapture::Display(DisplayCapture::new(StreamId(0)))
    } else {
        ProbeCapture::Wgc(WgcDisplayCapture::new(StreamId(0)))
    };
    if let Err(err) = capture.start(VideoCaptureConfig {
        source_id,
        target_fps: fps,
    }) {
        eprintln!("start failed: {err}");
        std::process::exit(1);
    }

    let started = Instant::now();
    let mut last_report = started;
    let mut frames = 0_u64;
    let mut last_pts: Option<i64> = None;
    let mut monotonic_violations = 0_u64;
    let mut format_changes = 0_u32;
    let mut lost = false;

    while started.elapsed() < Duration::from_secs(seconds) {
        match capture.next_event() {
            Ok(capture_api::VideoCaptureEvent::Frame(frame)) => {
                frames += 1;
                if let Some(last) = last_pts {
                    if frame.pts < last {
                        monotonic_violations += 1;
                    }
                }
                last_pts = Some(frame.pts);
                if frames.is_multiple_of(1000) {
                    println!("frame #{frames} ({}x{})", frame.width, frame.height);
                }
            }
            Ok(capture_api::VideoCaptureEvent::FormatChanged { width, height }) => {
                format_changes += 1;
                println!("format changed -> {width}x{height}");
            }
            Ok(capture_api::VideoCaptureEvent::Idle) => {}
            Ok(capture_api::VideoCaptureEvent::SourceLost) => {
                println!("SOURCE LOST - stopping probe");
                lost = true;
                break;
            }
            Err(capture_api::VideoCaptureError::EndOfStream) => break,
            Err(err) => {
                eprintln!("capture error: {err}");
                break;
            }
        }

        if last_report.elapsed() >= Duration::from_secs(5) {
            last_report = Instant::now();
            if let Some((copied, dropped, resizes)) = capture.stats_snapshot() {
                println!(
                    "[{:>4}s] delivered={frames} copied={copied} qdropped={dropped} resizes={resizes}",
                    started.elapsed().as_secs()
                );
            }
        }
    }

    let _ = capture.stop();
    let stats = capture.stats_snapshot();
    println!("=== probe summary ===");
    println!("duration_s={} frames={frames} monotonic_violations={monotonic_violations} format_changes={format_changes} source_lost={lost}", started.elapsed().as_secs());
    if let Some((copied, dropped, resizes)) = stats {
        println!("copied={copied} queue_dropped={dropped} resizes={resizes}");
    }
    if monotonic_violations > 0 || lost {
        std::process::exit(2);
    }
}
