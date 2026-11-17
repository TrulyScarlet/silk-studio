#![cfg(all(windows, target_arch = "x86_64", feature = "experimental-hooks"))]

#[cfg(feature = "d3d12-fixture")]
#[path = "controlled_recorder/d3d12_present_capture.rs"]
mod d3d12_present_capture;

#[cfg(feature = "d3d12-cross-process")]
#[path = "controlled_recorder/d3d12_cross_replay.rs"]
mod d3d12_cross_replay;
#[cfg(feature = "d3d12-device-replace-fixture")]
#[path = "controlled_recorder/d3d12_device_replace_replay.rs"]
mod d3d12_device_replace_replay;
#[cfg(feature = "d3d12-device-replace-fixture")]
#[path = "controlled_recorder/d3d12_loss_capture.rs"]
mod d3d12_loss_capture;
#[cfg(feature = "d3d12-rearm-fixture")]
#[path = "controlled_recorder/d3d12_resize_replay.rs"]
mod d3d12_resize_replay;
#[cfg(feature = "d3d12-bootstrap-fixture")]
#[path = "controlled_recorder/guarded_replay.rs"]
mod guarded_replay;
#[cfg(feature = "d3d12-bootstrap-fixture")]
#[path = "controlled_recorder/wgc_owned_replay.rs"]
mod wgc_owned_replay;

use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
use capture_hook_prototype::controlled_capture::{
    BridgeMode, ControlledCapture, ControlledSource, HelperRearm, ProducerMode, SOURCE_ID,
};
use encoder_api::{EncoderPreference, VideoEncoderConfig};
use encoder_media_foundation::new_fallback_encoder;
use media_types::{FramePayload, StreamId};
use recorder_engine::{EngineConfig, RecorderEngine, RecorderState};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn source(warp: bool, crash_after_frames: Option<u64>) -> ControlledSource {
    ControlledSource {
        executable: PathBuf::from(env!("CARGO_BIN_EXE_silk-d3d11-hook-fixture")),
        warp,
        crash_after_frames,
        bridge_mode: BridgeMode::CpuReadback,
        producer_mode: ProducerMode::Commanded,
    }
}
fn config() -> VideoCaptureConfig {
    VideoCaptureConfig {
        source_id: SOURCE_ID.into(),
        target_fps: 30,
    }
}

struct Output(PathBuf);
impl Output {
    fn new() -> Self {
        let root = std::env::temp_dir().join("opencode");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(format!(
            "silk-hook-replay-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn clip(&self) -> PathBuf {
        self.0.join("controlled-hook.mp4")
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn adapter_rejects_monitor_ids_and_invalid_cadence_before_launch() {
    let mut capture = ControlledCapture::new(source(true, None), StreamId(0));
    for (source_id, fps) in [
        ("auto", 30),
        ("window:hwnd:1", 30),
        ("game:auto", 60),
        (SOURCE_ID, 144),
    ] {
        assert!(capture
            .start(VideoCaptureConfig {
                source_id: source_id.into(),
                target_fps: fps
            })
            .is_err());
    }
    assert_eq!(capture.stats().copied.load(Ordering::Acquire), 0);
    capture.stop().unwrap();
}

#[test]
fn failed_start_cleans_up_and_can_be_retried_without_a_window_or_worker_leak() {
    let mut missing = source(true, None);
    missing.executable.set_file_name("silk-missing-fixture.exe");
    let mut capture = ControlledCapture::new(missing, StreamId(0));
    for _ in 0..2 {
        assert!(capture.start(config()).is_err());
        assert!(capture.stats().worker_exited.load(Ordering::Acquire));
        assert_eq!(capture.stats().copied.load(Ordering::Acquire), 0);
        capture.stop().unwrap();
    }
}

#[test]
#[ignore = "opens only controlled D3D11 fixture windows and uses WARP"]
fn lifecycle_backpressure_and_restart_warp() {
    let mut capture = ControlledCapture::new(source(true, None), StreamId(0));
    for _ in 0..2 {
        capture.start(config()).unwrap();
        assert!(capture.start(config()).is_err());
        assert!(matches!(
            capture.next_event().unwrap(),
            VideoCaptureEvent::FormatChanged {
                width: 640,
                height: 480
            }
        ));
        let stats = capture.stats();
        // Deliberately do not consume frames: at most three events remain queued.
        let deadline = Instant::now() + Duration::from_secs(3);
        while stats.copied.load(Ordering::Acquire) < 12 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(stats.dropped_queue_full.load(Ordering::Acquire) > 0);
        let VideoCaptureEvent::Frame(first) = capture.next_event().unwrap() else {
            panic!("expected frame")
        };
        assert_eq!((first.width, first.height), (640, 480));
        let FramePayload::Cpu(first_bytes) = &first.payload else {
            panic!("expected immutable CPU bridge frame")
        };
        let original = first_bytes.to_vec();
        let VideoCaptureEvent::Frame(second) = capture.next_event().unwrap() else {
            panic!("expected frame")
        };
        assert!(second.pts >= first.pts);
        assert_eq!(first_bytes.as_ref(), original.as_slice());
        capture.stop().unwrap();
        capture.stop().unwrap();
        assert!(stats.worker_exited.load(Ordering::Acquire));
        assert!(capture.next_event().is_err());
    }
}

#[test]
#[ignore = "opens only controlled D3D11 fixture windows and tests owned-child failure"]
fn producer_failure_is_terminal_without_desktop_fallback_warp() {
    let mut capture = ControlledCapture::new(source(true, Some(3)), StreamId(0));
    capture.start(config()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut failed = false;
    while Instant::now() < deadline {
        match capture.next_event() {
            Ok(_) => {}
            Err(error) => {
                assert!(error.to_string().contains("producer exited"));
                failed = true;
                break;
            }
        }
    }
    assert!(failed);
    let stats = capture.stats();
    capture.stop().unwrap();
    assert!(stats.worker_exited.load(Ordering::Acquire));
    assert_eq!(stats.copied.load(Ordering::Acquire), 3);
}

fn record(warp: bool, preference: EncoderPreference) {
    record_mode(warp, preference, BridgeMode::CpuReadback);
}

fn record_mode(warp: bool, preference: EncoderPreference, mode: BridgeMode) {
    record_source(warp, preference, mode, ProducerMode::Commanded);
}

fn record_source(
    warp: bool,
    preference: EncoderPreference,
    mode: BridgeMode,
    producer: ProducerMode,
) {
    let output = Output::new();
    let encoder = new_fallback_encoder(preference);
    let capture = ControlledCapture::new(
        ControlledSource {
            bridge_mode: mode,
            producer_mode: producer,
            ..source(warp, None)
        },
        StreamId(0),
    );
    let control = capture.helper_control();
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: SOURCE_ID.into(),
            follow_source_dimensions: true,
            retention_ms: 400,
            video_encoder: VideoEncoderConfig {
                width: 0,
                height: 0,
                fps: 30,
                codec: "h264".into(),
                keyframe_interval_seconds: 1.0,
                ..Default::default()
            },
            ..Default::default()
        },
        Box::new(capture),
        Box::new(encoder),
        Box::new(muxer::Mp4Muxer),
    );
    engine
        .start()
        .expect("start actual hook/transport/encoder pipeline");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        events.extend(engine.pump());
        assert_ne!(engine.state(), RecorderState::Error, "{events:?}");
        if engine.state() == RecorderState::Ready
            && engine.buffer_duration_ms() >= 800
            && engine.buffered_packet_count() >= 10
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(engine.state(), RecorderState::Ready, "{events:?}");
    assert!(engine.buffer_duration_ms() >= 800);
    let clip = engine
        .save_replay(&output.clip())
        .expect("save real hook replay");
    let metrics = engine.encoder_metrics();
    if let Some(control) = control {
        let stats = control.stats().unwrap();
        assert_eq!(stats.helper_epochs.load(Ordering::Acquire), 1);
        if mode == BridgeMode::Gpu {
            assert_eq!(stats.cpu_readbacks.load(Ordering::Acquire), 0);
            assert!(stats.gpu_copies.load(Ordering::Acquire) >= 10);
        }
    }
    if mode == BridgeMode::Gpu {
        if preference == EncoderPreference::HardwareOnly {
            assert!(
                metrics.gpu_surface_inputs >= 10,
                "hardware path did not consume native GPU surfaces: {metrics:?}"
            );
            assert_eq!(
                metrics.cpu_readback_inputs, 0,
                "hardware path silently read back GPU frames"
            );
        } else {
            assert!(
                metrics.cpu_readback_inputs >= 10,
                "software compatibility path should explicitly report readback: {metrics:?}"
            );
        }
    }
    let active_backend = &metrics.active_backend;
    engine.stop().unwrap();
    validate_mp4(&clip.path, clip.size_bytes);
    if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
        decode_mp4(&PathBuf::from(ffmpeg), &clip.path);
    }
    println!("PASS: {producer:?} hook -> NT shared GPU transport -> {mode:?} capture bridge -> Media Foundation -> replay buffer -> MP4; capture={} encoder={preference:?} backend={active_backend:?} gpu_surface_inputs={} cpu_readback_inputs={} size={} duration_ms={}",if warp {"WARP"} else {"hardware"},metrics.gpu_surface_inputs,metrics.cpu_readback_inputs,clip.size_bytes,clip.duration_ms);
}

fn decode_mp4(ffmpeg: &Path, clip: &Path) {
    decode_mp4_color(ffmpeg, clip, None);
}

#[cfg(feature = "d3d12-fixture")]
mod d3d12_replay {
    use super::*;
    use capture_api::VideoCaptureError;
    use capture_hook_prototype::d3d12_fixture::D3d12Fixture;
    use std::sync::{atomic::AtomicU64, Arc};
    const SOURCE: &str = "owned-d3d12-cpu-readback";

    // Test-only pull renderer on the recorder's video worker. No UI media work,
    // external target, swap-chain assumption or arbitrary source ID is accepted.
    struct Capture {
        warp: bool,
        fixture: Option<D3d12Fixture>,
        format: bool,
        origin: Instant,
        next: Instant,
        readbacks: Arc<AtomicU64>,
    }
    impl Capture {
        fn new(warp: bool) -> Self {
            Self {
                warp,
                fixture: None,
                format: false,
                origin: Instant::now(),
                next: Instant::now(),
                readbacks: Arc::new(AtomicU64::new(0)),
            }
        }
    }
    impl VideoCapture for Capture {
        fn enumerate_sources(&self) -> capture_api::Result<Vec<media_types::VideoSourceInfo>> {
            Ok(vec![media_types::VideoSourceInfo {
                id: SOURCE.into(),
                name: "Owned offscreen D3D12 CPU-readback fixture".into(),
                is_primary: true,
                width: 318,
                height: 178,
            }])
        }
        fn start(&mut self, config: VideoCaptureConfig) -> capture_api::Result<()> {
            if self.fixture.is_some() || config.source_id != SOURCE || config.target_fps != 30 {
                return Err(VideoCaptureError::InvalidConfiguration {
                    reason: "owned D3D12 test source only, 30 FPS, no double start".into(),
                });
            }
            self.fixture = Some(
                D3d12Fixture::new(self.warp, 318, 178)
                    .map_err(|details| VideoCaptureError::Backend { details })?,
            );
            self.format = true;
            self.origin = Instant::now();
            self.next = self.origin;
            Ok(())
        }
        fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
            let fixture = self
                .fixture
                .as_mut()
                .ok_or(VideoCaptureError::EndOfStream)?;
            if self.format {
                self.format = false;
                return Ok(VideoCaptureEvent::FormatChanged {
                    width: 318,
                    height: 178,
                });
            }
            if let Some(delay) = self.next.checked_duration_since(Instant::now()) {
                std::thread::sleep(delay);
            }
            let pts = self.origin.elapsed().as_millis() as i64;
            self.next = Instant::now() + Duration::from_nanos(1_000_000_000 / 30);
            if !fixture
                .submit_color([1., 0., 0., 1.])
                .map_err(|details| VideoCaptureError::Backend { details })?
            {
                return Err(VideoCaptureError::Backend {
                    details: "owned replay exhausted bounded consumer leases".into(),
                });
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(frame) = fixture
                    .poll()
                    .map_err(|details| VideoCaptureError::Backend { details })?
                {
                    assert!(frame
                        .bgra
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|pixel| *pixel == [0, 0, 255, 255]));
                    self.readbacks.fetch_add(1, Ordering::Relaxed);
                    return Ok(VideoCaptureEvent::Frame(media_types::VideoFrame {
                        stream_id: StreamId(0),
                        width: frame.width,
                        height: frame.height,
                        pixel_format: media_types::PixelFormat::Bgra8,
                        pts,
                        time_base: media_types::TimeBase::MILLISECOND,
                        payload: FramePayload::Cpu(frame.bgra),
                    }));
                }
                if Instant::now() >= deadline {
                    return Err(VideoCaptureError::Backend {
                        details: "D3D12 replay completion deadline".into(),
                    });
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        fn stop(&mut self) -> capture_api::Result<()> {
            self.fixture.take();
            Ok(())
        }
    }
    fn record(warp: bool) {
        record_mode(warp, false);
    }
    fn record_mode(warp: bool, presentation: bool) {
        record_route(warp, presentation, false);
    }
    fn record_route(warp: bool, presentation: bool, gpu: bool) {
        let output = Output::new();
        type Route = (
            Box<dyn VideoCapture>,
            Arc<AtomicU64>,
            &'static str,
            Option<Arc<AtomicU64>>,
        );
        let (capture, readbacks, source, gpu_copies): Route = if gpu {
            #[cfg(feature = "d3d12-gpu-fixture")]
            {
                let capture = super::d3d12_present_capture::Capture::new_gpu(warp);
                let readbacks = capture.readbacks.clone();
                let copies = capture.gpu_copies.clone();
                (
                    Box::new(capture),
                    readbacks,
                    super::d3d12_present_capture::GPU_SOURCE,
                    Some(copies),
                )
            }
            #[cfg(not(feature = "d3d12-gpu-fixture"))]
            {
                panic!("GPU route requires d3d12-gpu-fixture");
            }
        } else if presentation {
            let capture = super::d3d12_present_capture::Capture::new(warp);
            let readbacks = capture.readbacks.clone();
            (
                Box::new(capture),
                readbacks,
                super::d3d12_present_capture::SOURCE,
                None,
            )
        } else {
            let capture = Capture::new(warp);
            let readbacks = capture.readbacks.clone();
            (Box::new(capture), readbacks, SOURCE, None)
        };
        let mut engine = RecorderEngine::new(
            EngineConfig {
                video_source_id: source.into(),
                follow_source_dimensions: true,
                retention_ms: 400,
                video_encoder: VideoEncoderConfig {
                    width: 0,
                    height: 0,
                    fps: 30,
                    codec: "h264".into(),
                    keyframe_interval_seconds: 1.,
                    ..Default::default()
                },
                ..Default::default()
            },
            capture,
            Box::new(new_fallback_encoder(if gpu && !warp {
                EncoderPreference::HardwareOnly
            } else {
                EncoderPreference::SoftwareOnly
            })),
            Box::new(muxer::Mp4Muxer),
        );
        engine.start().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let events = engine.pump();
            assert_ne!(engine.state(), RecorderState::Error, "{events:?}");
            if engine.state() == RecorderState::Ready
                && engine.buffer_duration_ms() >= 800
                && engine.buffered_packet_count() >= 10
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(engine.state(), RecorderState::Ready);
        assert!(engine.buffer_duration_ms() >= 800);
        let clip = engine.save_replay(&output.clip()).unwrap();
        assert!(
            (700..=2000).contains(&clip.duration_ms),
            "unexpected replay duration: {} ms",
            clip.duration_ms
        );
        let metrics = engine.encoder_metrics();
        if gpu {
            if warp {
                assert!(
                    metrics.cpu_readback_inputs >= 10,
                    "software compatibility path must report encoder readbacks: {metrics:?}"
                );
            } else {
                assert!(
                    metrics.gpu_surface_inputs >= 10,
                    "hardware encoder did not consume GPU frames: {metrics:?}"
                );
                assert_eq!(metrics.cpu_readback_inputs, 0);
            }
        } else {
            assert_eq!(metrics.gpu_surface_inputs, 0);
        }
        engine.stop().unwrap();
        let readbacks = readbacks.load(Ordering::Acquire);
        if gpu {
            assert_eq!(readbacks, 0);
            assert!(gpu_copies.unwrap().load(Ordering::Acquire) >= 10);
        } else {
            assert!(readbacks >= 10);
        }
        validate_mp4_dimensions(&clip.path, clip.size_bytes, (318, 178));
        if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
            decode_mp4_color(&PathBuf::from(ffmpeg), &clip.path, Some([255, 0, 0]));
        }
        println!("PASS: owned D3D12 {} {} -> {} -> replay MP4, 318x178, {} ms, capture_readbacks={readbacks}, encoder_gpu_inputs={}, encoder_readbacks={}, backend={:?}; no hook/game claim", if presentation {"swap-chain"} else {"offscreen"}, if warp {"WARP"} else {"hardware adapter 0"}, if gpu {"shared-fence GPU interop"} else {"CPU readback/software H.264"}, clip.duration_ms, metrics.gpu_surface_inputs, metrics.cpu_readback_inputs, metrics.active_backend);
    }
    #[test]
    fn invalid_source_and_fps_do_not_create_d3d12_device() {
        let mut capture = Capture::new(true);
        for (source_id, target_fps) in [("auto", 30), (SOURCE, 144)] {
            assert!(capture
                .start(VideoCaptureConfig {
                    source_id: source_id.into(),
                    target_fps
                })
                .is_err());
        }
        assert!(capture.fixture.is_none());
        capture.stop().unwrap();
    }
    #[test]
    #[ignore = "D3D12 WARP readback and software Media Foundation encoder"]
    fn replay_warp() {
        record(true);
    }
    #[test]
    #[ignore = "D3D12 hardware adapter 0 readback and software Media Foundation encoder"]
    fn replay_hardware() {
        record(false);
    }
    #[test]
    #[ignore = "owned windowed D3D12 WARP swap-chain replay"]
    fn presentation_replay_warp() {
        record_mode(true, true);
    }
    #[test]
    #[ignore = "owned windowed D3D12 hardware swap-chain replay"]
    fn presentation_replay_hardware() {
        record_mode(false, true);
    }
    #[cfg(feature = "d3d12-gpu-fixture")]
    #[test]
    #[ignore = "owned D3D12 WARP GPU interop and explicitly software encoding"]
    fn gpu_interop_replay_warp() {
        record_route(true, true, true);
    }
    #[cfg(feature = "d3d12-gpu-fixture")]
    #[test]
    #[ignore = "owned D3D12 GPU interop and hardware-only H.264 without CPU readback"]
    fn gpu_interop_replay_hardware() {
        record_route(false, true, true);
    }
}
fn decode_mp4_color(ffmpeg: &Path, clip: &Path, expected: Option<[u8; 3]>) {
    // Reuse only the exact already-approved external tool, not an arbitrary
    // executable from PATH. Hash streaming is bounded to the approved envelope.
    let mut file = std::fs::File::open(ffmpeg).unwrap();
    assert!(file.metadata().unwrap().len() <= 230_000_000);
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    assert_eq!(
        format!("{:X}", hash.finalize()),
        "57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A",
        "unapproved FFmpeg executable"
    );
    let pixel_count = if expected.is_some() {
        let file = std::fs::File::open(clip).unwrap();
        let size = file.metadata().unwrap().len();
        let reader = mp4::Mp4Reader::read_header(file, size).unwrap();
        let count = reader.tracks().get(&1).unwrap().sample_count() as usize;
        assert!(
            (1..=128).contains(&count),
            "bounded epoch decoder sample budget exceeded"
        );
        count
    } else {
        1
    };
    for sample_pixel in [false, true] {
        let mut command = Command::new(ffmpeg);
        command
            .args(["-nostdin", "-v", "error", "-threads", "1", "-i"])
            .arg(clip)
            .args(["-map", "0:v:0"]);
        if sample_pixel {
            command
                .args(["-vf", "scale=1:1", "-frames:v"])
                .arg(pixel_count.to_string())
                .args([
                    "-fps_mode",
                    "passthrough",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ]);
        } else {
            command.args(["-f", "null", "-"]);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(if sample_pixel {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let handle = windows::Win32::Foundation::HANDLE(child.as_raw_handle());
        if unsafe { windows::Win32::System::Threading::WaitForSingleObject(handle, 10_000) }
            != windows::Win32::Foundation::WAIT_OBJECT_0
        {
            let _ = child.kill();
            let _ = child.wait();
            panic!("approved FFmpeg decoder exceeded deadline");
        }
        assert!(
            child.wait().unwrap().success(),
            "saved hook clip failed actual H.264 decode"
        );
        if sample_pixel {
            let mut rgb = Vec::new();
            child
                .stdout
                .take()
                .unwrap()
                .take(385)
                .read_to_end(&mut rgb)
                .unwrap();
            assert_eq!(rgb.len(), pixel_count * 3);
            for rgb in rgb.as_chunks::<3>().0 {
                if let Some(expected) = expected {
                    assert!(
                        rgb.iter()
                            .zip(expected)
                            .all(|(actual, expected)| actual.abs_diff(expected) < 70),
                        "wrong capture epoch's decoded color: {rgb:?}, expected {expected:?}"
                    );
                }
                assert_eq!(
                    rgb.iter().filter(|value| **value > 180).count(),
                    1,
                    "decoded image is black, blank, or not the fixture's primary color: {rgb:?}"
                );
                assert_eq!(
                    rgb.iter().filter(|value| **value < 70).count(),
                    2,
                    "decoded fixture color changed: {rgb:?}"
                );
            }
        }
    }
    println!(
        "PASS: approved FFmpeg decoded every saved video frame; checked {pixel_count} primary-color samples"
    );
}

fn validate_mp4(path: &Path, size: u64) {
    validate_mp4_dimensions(path, size, (640, 480));
}
fn validate_mp4_dimensions(path: &Path, size: u64, dimensions: (u16, u16)) {
    assert!(size > 1000);
    let file = std::fs::File::open(path).unwrap();
    let mut reader = mp4::Mp4Reader::read_header(file, size).expect("parse saved MP4");
    assert_eq!(reader.tracks().len(), 1);
    let track = reader.tracks().get(&1).expect("video track");
    assert_eq!((track.width(), track.height()), dimensions);
    assert!(track.duration() >= Duration::from_millis(750));
    assert!(track.sample_count() >= 10);
    let count = track.sample_count();
    let mut last = 0;
    for index in 1..=count {
        let sample = reader
            .read_sample(1, index)
            .unwrap()
            .expect("real video sample");
        if index == 1 {
            assert!(sample.is_sync, "saved replay must start with a keyframe");
        }
        assert!(sample.start_time >= last);
        last = sample.start_time;
        assert!(!sample.bytes.is_empty());
    }
}

#[test]
#[ignore = "opens original WARP renderer and uses native Windows software H.264 encoder"]
fn replay_save_software_warp() {
    record(true, EncoderPreference::SoftwareOnly);
}

#[test]
#[ignore = "opens original hardware renderer and requires a native hardware H.264 encoder"]
fn replay_save_hardware() {
    record(false, EncoderPreference::HardwareOnly);
}

#[test]
#[ignore = "opens controlled WARP renderer and uses explicit encoder CPU fallback"]
fn replay_save_gpu_bridge_software_warp() {
    record_mode(true, EncoderPreference::SoftwareOnly, BridgeMode::Gpu);
}

#[test]
#[ignore = "opens controlled hardware renderer and requires a D3D11-aware hardware encoder"]
fn replay_save_gpu_only_hardware() {
    record_mode(false, EncoderPreference::HardwareOnly, BridgeMode::Gpu);
}

#[test]
#[ignore = "opens controlled WARP renderer and verifies GPU ownership, stale handles, and restart bounds"]
fn gpu_lease_retention_and_bounded_restart_warp() {
    use gpu_windows::{BgraReadback, GpuTextureSlot};
    let gpu_source = ControlledSource {
        bridge_mode: BridgeMode::Gpu,
        ..source(true, None)
    };
    let mut capture = ControlledCapture::new(gpu_source, StreamId(0));
    capture.start(config()).unwrap();
    let context = capture
        .gpu_frame_context()
        .expect("GPU context is published before encoder configuration");
    assert!(context.session_resource().is_some());
    assert!(matches!(
        capture.next_event().unwrap(),
        VideoCaptureEvent::FormatChanged { .. }
    ));
    let stats = capture.stats();
    let mut frames = Vec::new();
    let mut leases = Vec::new();
    for _ in 0..3 {
        let VideoCaptureEvent::Frame(frame) = capture.next_event().unwrap() else {
            panic!("expected GPU frame")
        };
        assert!(matches!(frame.payload, FramePayload::Gpu(_)));
        leases.push(context.resolve(&frame).unwrap());
        frames.push(frame);
    }
    assert!(
        context.resolve(&frames[0]).is_err(),
        "previous publication must no longer resolve, even though its lease stays alive"
    );
    let mut readback = BgraReadback::default();
    let first = leases[0].downcast_ref::<GpuTextureSlot>().unwrap();
    let original = readback.read_scaled(first, 640, 480).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while stats.dropped_pool_full.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(stats.dropped_pool_full.load(Ordering::Acquire) > 0);
    assert_eq!(stats.cpu_readbacks.load(Ordering::Acquire), 0);
    assert!(stats.gpu_copies.load(Ordering::Acquire) >= 3);
    assert_eq!(
        readback.read_scaled(first, 640, 480).unwrap(),
        original,
        "retained encoder frame was overwritten"
    );
    capture.stop().unwrap();
    assert!(context.resolve(&frames[2]).is_err());
    assert!(context.session_resource().is_none());
    assert_eq!(
        readback.read_scaled(first, 640, 480).unwrap(),
        original,
        "retained GPU lease did not survive stop"
    );
    capture.start(config()).unwrap();
    let second_context = capture.gpu_frame_context().unwrap();
    capture.stop().unwrap();
    let error = capture
        .start(config())
        .expect_err("old contexts must not permit unbounded texture allocations");
    assert!(error.to_string().contains("six-surface"), "{error}");
    drop(second_context);
    capture
        .start(config())
        .expect("release of old context restores the bounded budget");
    capture.stop().unwrap();
    drop(leases);
    drop(context);
    capture.start(config()).unwrap();
    capture.stop().unwrap();
}

fn dll_source(warp: bool) -> ControlledSource {
    ControlledSource {
        producer_mode: ProducerMode::CooperativeDll,
        bridge_mode: BridgeMode::Gpu,
        ..source(warp, None)
    }
}

#[test]
#[ignore = "loads the owned cooperative helper DLL and software encoder"]
fn replay_save_dll_gpu_bridge_software_warp() {
    record_source(
        true,
        EncoderPreference::SoftwareOnly,
        BridgeMode::Gpu,
        ProducerMode::CooperativeDll,
    );
}

#[test]
#[ignore = "loads the owned cooperative helper DLL and hardware encoder"]
fn replay_save_dll_gpu_only_hardware() {
    record_source(
        false,
        EncoderPreference::HardwareOnly,
        BridgeMode::Gpu,
        ProducerMode::CooperativeDll,
    );
}

#[cfg(feature = "resident-hooks")]
#[test]
#[ignore = "normally loads helper into an already-rendering owned process, software encoder"]
fn replay_save_resident_gpu_bridge_software_warp() {
    record_source(
        true,
        EncoderPreference::SoftwareOnly,
        BridgeMode::Gpu,
        ProducerMode::ResidentDll,
    );
}

#[cfg(feature = "resident-hooks")]
#[test]
#[ignore = "normally loads helper into an already-rendering owned process, hardware encoder"]
fn replay_save_resident_gpu_only_hardware() {
    record_source(
        false,
        EncoderPreference::HardwareOnly,
        BridgeMode::Gpu,
        ProducerMode::ResidentDll,
    );
}

fn dll_rearm_replays(warp: bool, preference: EncoderPreference) {
    let output = Output::new();
    let capture = ControlledCapture::new(dll_source(warp), StreamId(0));
    let control = capture.helper_control().unwrap();
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: SOURCE_ID.into(),
            follow_source_dimensions: true,
            retention_ms: 400,
            video_encoder: VideoEncoderConfig {
                width: 0,
                height: 0,
                fps: 30,
                codec: "h264".into(),
                keyframe_interval_seconds: 1.0,
                ..Default::default()
            },
            ..Default::default()
        },
        Box::new(capture),
        Box::new(new_fallback_encoder(preference)),
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    let stats = control.stats().unwrap();
    let mut previous_buffer_epoch = None;
    for (epoch, width, height, recreate, color) in [
        (1, 640, 480, false, [255, 0, 0]),
        (2, 800, 600, false, [0, 255, 0]),
        (3, 960, 540, true, [0, 0, 255]),
    ] {
        if epoch > 1 {
            control
                .rearm(HelperRearm {
                    width,
                    height,
                    recreate,
                })
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut seen_format = false;
        let mut completed = false;
        let mut events = Vec::new();
        while Instant::now() < deadline {
            for event in engine.pump() {
                if let recorder_engine::EngineEvent::Warning { code, message } = &event {
                    if code == "CAPTURE_FORMAT_CHANGED"
                        && message.contains(&format!("{width}x{height}"))
                    {
                        seen_format = true;
                    }
                }
                if matches!(
                    &event,
                    recorder_engine::EngineEvent::StateChanged(RecorderState::Buffering)
                ) && seen_format
                {
                    completed = true;
                }
                events.push(event);
            }
            assert_ne!(
                engine.state(),
                RecorderState::Error,
                "epoch {epoch}: {events:?}"
            );
            if completed
                && stats.helper_epochs.load(Ordering::Acquire) == epoch
                && engine.state() == RecorderState::Ready
                && engine.buffer_duration_ms() >= 800
                && engine.buffered_packet_count() >= 10
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(completed, "no clean format epoch {epoch}: {events:?}");
        assert_eq!(stats.helper_epochs.load(Ordering::Acquire), epoch);
        assert_eq!(engine.state(), RecorderState::Ready, "{events:?}");
        assert!(engine.buffer_duration_ms() >= 800);
        if let Some(previous) = previous_buffer_epoch {
            assert!(
                engine.buffer_epoch() > previous,
                "rearm did not reset replay epoch"
            );
        }
        previous_buffer_epoch = Some(engine.buffer_epoch());
        let path = output.0.join(format!("helper-epoch-{epoch}.mp4"));
        let clip = engine.save_replay(&path).expect("save fresh DLL epoch");
        validate_mp4_dimensions(&clip.path, clip.size_bytes, (width as u16, height as u16));
        if let Some(ffmpeg) = std::env::var_os("SILK_HOOK_TEST_FFMPEG") {
            decode_mp4_color(&PathBuf::from(ffmpeg), &clip.path, Some(color));
        }
        let metrics = engine.encoder_metrics();
        if preference == EncoderPreference::HardwareOnly {
            assert!(metrics.gpu_surface_inputs >= 10, "{metrics:?}");
            assert_eq!(metrics.cpu_readback_inputs, 0, "{metrics:?}");
        } else {
            assert!(metrics.cpu_readback_inputs >= 10, "{metrics:?}");
        }
        assert_eq!(stats.cpu_readbacks.load(Ordering::Acquire), 0);
        println!("PASS: DLL recorder epoch={epoch} source={width}x{height} recreate={recreate} GPU inputs={} encoder readbacks={} backend={:?}",metrics.gpu_surface_inputs,metrics.cpu_readback_inputs,metrics.active_backend);
    }
    engine.stop().unwrap();
    assert!(stats.worker_exited.load(Ordering::Acquire));
}

#[test]
#[ignore = "loads DLL, resizes/recreates owned WARP renderer and decodes three replay epochs"]
fn dll_resize_device_rearm_replays_warp() {
    dll_rearm_replays(true, EncoderPreference::SoftwareOnly);
}

#[test]
#[ignore = "loads DLL, resizes/recreates owned hardware renderer and decodes three replay epochs"]
fn dll_resize_device_rearm_replays_hardware() {
    dll_rearm_replays(false, EncoderPreference::HardwareOnly);
}

#[test]
#[ignore = "loads owned DLL and intentionally exits target with queued GPU frames"]
fn dll_dead_target_discards_pending_frames_warp() {
    let mut capture = ControlledCapture::new(
        ControlledSource {
            crash_after_frames: Some(3),
            ..dll_source(true)
        },
        StreamId(0),
    );
    capture.start(config()).unwrap();
    let stats = capture.stats();
    let context = capture.gpu_frame_context().unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    while !stats.worker_exited.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(stats.worker_exited.load(Ordering::Acquire));
    assert_eq!(stats.copied.load(Ordering::Acquire), 3);
    for _ in 0..3 {
        assert!(capture
            .next_event()
            .unwrap_err()
            .to_string()
            .contains("producer exited"));
    }
    assert!(
        stats.retired_events.load(Ordering::Acquire) > 0,
        "queued media escaped terminal invalidation"
    );
    assert!(context.session_resource().is_none());
    assert_eq!(stats.cpu_readbacks.load(Ordering::Acquire), 0);
    capture.stop().unwrap();
}

#[test]
#[ignore = "loads owned DLL and tests terminal target exit through RecorderEngine"]
fn dll_target_exit_recorder_is_terminal_warp() {
    let capture = ControlledCapture::new(
        ControlledSource {
            crash_after_frames: Some(60),
            ..dll_source(true)
        },
        StreamId(0),
    );
    let control = capture.helper_control().unwrap();
    let mut engine = RecorderEngine::new(
        EngineConfig {
            video_source_id: SOURCE_ID.into(),
            follow_source_dimensions: true,
            video_encoder: VideoEncoderConfig {
                width: 0,
                height: 0,
                fps: 30,
                ..Default::default()
            },
            ..Default::default()
        },
        Box::new(capture),
        Box::new(new_fallback_encoder(EncoderPreference::SoftwareOnly)),
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    let stats = control.stats().unwrap();
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while engine.state() != RecorderState::Error && Instant::now() < deadline {
        events.extend(engine.pump());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(engine.state(), RecorderState::Error, "{events:?}");
    assert_eq!(
        stats.helper_epochs.load(Ordering::Acquire),
        1,
        "terminal loss must not reattach"
    );
    assert_eq!(stats.delivery_epoch(), 0);
    assert!(stats.worker_exited.load(Ordering::Acquire));
    assert_eq!(stats.cpu_readbacks.load(Ordering::Acquire), 0);
    assert!(events.iter().any(|event|matches!(event,recorder_engine::EngineEvent::Warning {message,..} if message.contains("producer exited"))),"{events:?}");
    engine.stop().unwrap();
}

#[test]
#[ignore = "loads owned DLL and verifies queued epoch rejection plus bounded restart"]
fn dll_queued_epoch_rejection_and_restart_warp() {
    let mut capture = ControlledCapture::new(dll_source(true), StreamId(0));
    let control = capture.helper_control().unwrap();
    for _ in 0..2 {
        capture.start(config()).unwrap();
        let stats = capture.stats();
        let deadline = Instant::now() + Duration::from_secs(4);
        while stats.dropped_queue_full.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(stats.dropped_queue_full.load(Ordering::Acquire) > 0);
        control
            .rearm(HelperRearm {
                width: 800,
                height: 600,
                recreate: false,
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while stats.delivery_epoch() == 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(matches!(
            capture.next_event().unwrap(),
            VideoCaptureEvent::FormatChanged {
                width: 800,
                height: 600
            }
        ));
        let VideoCaptureEvent::Frame(frame) = capture.next_event().unwrap() else {
            panic!("new epoch frame required")
        };
        assert_eq!((frame.width, frame.height), (800, 600));
        assert!(stats.retired_events.load(Ordering::Acquire) > 0);
        capture.stop().unwrap();
        assert!(stats.worker_exited.load(Ordering::Acquire));
    }
}
