//! Deterministic engine-side encoding epoch test.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use audio_api::{AudioCapture, AudioCaptureConfig, AudioCaptureError, AudioCaptureEvent};
use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent};
use encoder_api::{
    EncoderCapabilities, EncoderError, EncoderMetrics, VideoEncoder, VideoEncoderConfig,
};
use media_types::{
    AudioFrame, EncodedPacket, FramePayload, MediaSnapshot, MediaType, PacketPayload, PixelFormat,
    SampleFormat, StreamDescriptor, StreamId, StreamPackets, TimeBase, VideoFrame, VideoSourceInfo,
};
use muxer::{ClipMetadata, Mp4Muxer, Muxer, SaveOptions};
use recorder_engine::{AudioInput, EngineConfig, RecorderEngine};
use test_support::{
    AudioStep, MockAudioEncoder, MockMuxer, MockVideoEncoder, ScriptedAudioCapture,
    ScriptedVideoCapture, VideoStep,
};

struct TwoFrameCapture {
    frames: VecDeque<VideoFrame>,
    started: bool,
}

impl TwoFrameCapture {
    fn new() -> Self {
        Self::with_frame_count(2)
    }

    fn with_frame_count(count: u32) -> Self {
        let frame = |pts| VideoFrame {
            stream_id: StreamId(0),
            width: 1_920,
            height: 1_080,
            pixel_format: PixelFormat::Nv12,
            pts,
            time_base: TimeBase::MILLISECOND,
            payload: FramePayload::Cpu(Arc::from(&[0_u8][..])),
        };
        Self {
            frames: (0..count)
                .map(|index| frame(i64::from(index) * 33))
                .collect(),
            started: false,
        }
    }
}

impl VideoCapture for TwoFrameCapture {
    fn enumerate_sources(&self) -> capture_api::Result<Vec<VideoSourceInfo>> {
        Ok(Vec::new())
    }

    fn start(&mut self, _config: VideoCaptureConfig) -> capture_api::Result<()> {
        self.started = true;
        Ok(())
    }

    fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
        if !self.started {
            return Err(VideoCaptureError::Backend {
                details: "capture not started".to_string(),
            });
        }
        self.frames
            .pop_front()
            .map(VideoCaptureEvent::Frame)
            .ok_or(VideoCaptureError::EndOfStream)
    }

    fn stop(&mut self) -> capture_api::Result<()> {
        self.started = false;
        Ok(())
    }
}

struct RestartingAudioCapture {
    starts: u32,
    step: u8,
    started: bool,
}

struct SnapshotRecordingMuxer {
    snapshot: Arc<Mutex<Option<media_types::MediaSnapshot>>>,
}

impl Muxer for SnapshotRecordingMuxer {
    fn write_snapshot(
        &mut self,
        snapshot: &media_types::MediaSnapshot,
        final_path: &Path,
        _options: &SaveOptions,
    ) -> muxer::Result<ClipMetadata> {
        *self.snapshot.lock().expect("snapshot lock") = Some(snapshot.clone());
        Ok(ClipMetadata {
            path: final_path.to_path_buf(),
            duration_ms: snapshot.video_duration_ticks().unwrap_or(0) as u64,
            size_bytes: 0,
            created_at_unix_ms: snapshot.captured_at_unix_ms,
            video_codec: Some("h264".to_string()),
            audio_codecs: vec!["aac".to_string()],
        })
    }
}

impl RestartingAudioCapture {
    fn new() -> Self {
        Self {
            starts: 0,
            step: 0,
            started: false,
        }
    }

    fn frame() -> AudioFrame {
        AudioFrame {
            stream_id: StreamId(0),
            sample_format: SampleFormat::F32,
            sample_rate: 48_000,
            channels: 2,
            sample_count: 960,
            pts: 0,
            time_base: TimeBase::from_hz(48_000),
            data: vec![0_u8; 4 * 2 * 960].into(),
        }
    }
}

impl AudioCapture for RestartingAudioCapture {
    fn enumerate_devices(&self) -> audio_api::Result<Vec<media_types::AudioDeviceInfo>> {
        Ok(Vec::new())
    }

    fn start(&mut self, _config: AudioCaptureConfig) -> audio_api::Result<()> {
        self.starts += 1;
        self.step = 0;
        self.started = true;
        Ok(())
    }

    fn next_event(&mut self) -> audio_api::Result<AudioCaptureEvent> {
        if !self.started {
            return Err(AudioCaptureError::Backend {
                details: "capture not started".to_string(),
            });
        }
        match (self.starts, self.step) {
            (1, 0) => {
                self.step = 1;
                Ok(AudioCaptureEvent::Frames(Self::frame()))
            }
            (1, 1) => {
                self.step = 2;
                Ok(AudioCaptureEvent::DeviceLost)
            }
            (2, 0) => {
                self.step = 1;
                Ok(AudioCaptureEvent::Frames(Self::frame()))
            }
            _ => {
                self.started = false;
                Err(AudioCaptureError::EndOfStream)
            }
        }
    }

    fn stop(&mut self) -> audio_api::Result<()> {
        self.started = false;
        Ok(())
    }
}

struct EpochEncoder {
    configured: bool,
    calls: usize,
    epoch: u64,
}

struct FormatChangeEncoder {
    configure_count: Arc<AtomicUsize>,
    configured_dimensions: Arc<Mutex<Vec<(u32, u32)>>>,
    configured: bool,
}

impl VideoEncoder for FormatChangeEncoder {
    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities {
            backend_name: "format-change-test".to_string(),
            codec: "h264".to_string(),
            hardware_accelerated: false,
            max_width: 3_840,
            max_height: 2_160,
            supported_fps: vec![30, 60],
            supported_pixel_formats: vec![PixelFormat::Nv12],
        }
    }

    fn configure(&mut self, config: VideoEncoderConfig) -> encoder_api::Result<()> {
        config.validate()?;
        self.configure_count.fetch_add(1, Ordering::SeqCst);
        self.configured_dimensions
            .lock()
            .expect("configured dimension lock")
            .push((config.width, config.height));
        self.configured = true;
        Ok(())
    }

    fn encode(&mut self, frame: VideoFrame) -> encoder_api::Result<Vec<EncodedPacket>> {
        if !self.configured {
            return Err(EncoderError::NotConfigured);
        }
        Ok(vec![EncodedPacket {
            stream_id: frame.stream_id,
            media_type: MediaType::Video,
            pts: frame.pts,
            dts: frame.pts,
            duration: 33,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: true,
            sequence: 0,
            payload: PacketPayload::from(&[1_u8, 2, 3][..]),
        }])
    }

    fn drain(&mut self) -> encoder_api::Result<Vec<EncodedPacket>> {
        Ok(Vec::new())
    }
}

impl VideoEncoder for EpochEncoder {
    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities {
            backend_name: "epoch-test".to_string(),
            codec: "h264".to_string(),
            hardware_accelerated: false,
            max_width: 3_840,
            max_height: 2_160,
            supported_fps: vec![30, 60],
            supported_pixel_formats: vec![PixelFormat::Nv12],
        }
    }

    fn configure(&mut self, config: VideoEncoderConfig) -> encoder_api::Result<()> {
        config.validate()?;
        self.configured = true;
        Ok(())
    }

    fn encode(&mut self, frame: VideoFrame) -> encoder_api::Result<Vec<EncodedPacket>> {
        if !self.configured {
            return Err(EncoderError::NotConfigured);
        }
        self.calls += 1;
        if self.calls == 2 {
            self.epoch = 1;
        }
        Ok(vec![EncodedPacket {
            stream_id: frame.stream_id,
            media_type: MediaType::Video,
            pts: frame.pts,
            dts: frame.pts,
            duration: 33,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: true,
            sequence: 0,
            payload: PacketPayload::from(&[1_u8, 2, 3][..]),
        }])
    }

    fn drain(&mut self) -> encoder_api::Result<Vec<EncodedPacket>> {
        Ok(Vec::new())
    }

    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn metrics(&self) -> EncoderMetrics {
        EncoderMetrics {
            frames_submitted: self.calls as u64,
            frames_encoded: self.calls as u64,
            epoch: self.epoch,
            active_backend: Some("epoch-test".to_string()),
            ..EncoderMetrics::default()
        }
    }
}

struct DrainOnlyEncoder {
    configured: bool,
    pending: Vec<EncodedPacket>,
}

impl VideoEncoder for DrainOnlyEncoder {
    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities {
            backend_name: "drain-only-test".to_string(),
            codec: "h264".to_string(),
            hardware_accelerated: false,
            max_width: 3_840,
            max_height: 2_160,
            supported_fps: vec![30, 60],
            supported_pixel_formats: vec![PixelFormat::Nv12],
        }
    }

    fn configure(&mut self, config: VideoEncoderConfig) -> encoder_api::Result<()> {
        config.validate()?;
        self.configured = true;
        Ok(())
    }

    fn encode(&mut self, frame: VideoFrame) -> encoder_api::Result<Vec<EncodedPacket>> {
        if !self.configured {
            return Err(EncoderError::NotConfigured);
        }
        self.pending.push(EncodedPacket {
            stream_id: frame.stream_id,
            media_type: MediaType::Video,
            pts: frame.pts,
            dts: frame.pts,
            duration: 33,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: frame.pts == 0,
            sequence: 0,
            payload: PacketPayload::from(&[4_u8, 5, 6][..]),
        });
        Ok(Vec::new())
    }

    fn drain(&mut self) -> encoder_api::Result<Vec<EncodedPacket>> {
        Ok(std::mem::take(&mut self.pending))
    }
}

#[test]
fn encoder_epoch_reset_drops_old_packets_and_re_registers_stream() {
    let mut engine = RecorderEngine::new(
        EngineConfig {
            retention_ms: 1_000,
            ..EngineConfig::default()
        },
        Box::new(TwoFrameCapture::new()),
        Box::new(EpochEncoder {
            configured: false,
            calls: 0,
            epoch: 0,
        }),
        Box::new(MockMuxer::default()) as Box<dyn Muxer>,
    );
    engine.start().expect("start engine");

    let deadline = Instant::now() + Duration::from_secs(2);
    while !engine.worker_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    engine.pump();

    assert_eq!(engine.buffer_epoch(), 1);
    assert_eq!(engine.buffered_packet_count(), 1);
    assert_eq!(engine.encoder_metrics().frames_encoded, 2);
    engine.stop().expect("stop engine");
}

#[test]
fn format_change_reconfigures_encoder_and_starts_a_new_epoch() {
    let configure_count = Arc::new(AtomicUsize::new(0));
    let configured_dimensions = Arc::new(Mutex::new(Vec::new()));
    let mut engine = RecorderEngine::new(
        EngineConfig {
            retention_ms: 1,
            ..EngineConfig::default()
        },
        Box::new(ScriptedVideoCapture::new(vec![
            VideoStep::Frames(2),
            VideoStep::FormatChange {
                width: 1_280,
                height: 720,
            },
            VideoStep::Frames(2),
        ])),
        Box::new(FormatChangeEncoder {
            configure_count: Arc::clone(&configure_count),
            configured_dimensions: Arc::clone(&configured_dimensions),
            configured: false,
        }),
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start engine");

    let deadline = Instant::now() + Duration::from_secs(2);
    while !engine.worker_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    let events = engine.pump();

    assert_eq!(configure_count.load(Ordering::SeqCst), 2);
    assert_eq!(
        *configured_dimensions.lock().expect("configured dimensions"),
        vec![(1_920, 1_080), (1_920, 1_080)],
        "fixed output dimensions must survive a capture format change"
    );
    assert_eq!(engine.buffer_epoch(), 1);
    assert_eq!(engine.buffered_packet_count(), 2);
    assert_eq!(engine.state(), recorder_engine::RecorderState::Ready);
    assert!(events.iter().any(|event| matches!(
        event,
        recorder_engine::EngineEvent::Warning { code, message }
            if code == "CAPTURE_FORMAT_CHANGED" && message.contains("1280x720")
    )));
    assert!(!events.iter().any(|event| matches!(
        event,
        recorder_engine::EngineEvent::Warning { code, .. }
            if code == "VIDEO_FORMAT_CHANGE_FAILED"
    )));

    let snapshot = engine
        .snapshot_replay()
        .expect("snapshot after format change");
    let video = snapshot
        .streams
        .iter()
        .find(|stream| stream.descriptor.media_type == MediaType::Video)
        .expect("video stream");
    assert_eq!(video.descriptor.width, Some(1_920));
    assert_eq!(video.descriptor.height, Some(1_080));
    engine.stop().expect("stop engine");
}

#[test]
fn native_output_uses_source_dimensions_and_follows_format_changes() {
    let configure_count = Arc::new(AtomicUsize::new(0));
    let configured_dimensions = Arc::new(Mutex::new(Vec::new()));
    let mut engine = RecorderEngine::new(
        EngineConfig {
            retention_ms: 1,
            follow_source_dimensions: true,
            video_encoder: VideoEncoderConfig {
                width: 0,
                height: 0,
                ..VideoEncoderConfig::default()
            },
            ..EngineConfig::default()
        },
        Box::new(
            ScriptedVideoCapture::new(vec![
                VideoStep::Frames(2),
                VideoStep::FormatChange {
                    width: 1_280,
                    height: 720,
                },
                VideoStep::Frames(2),
            ])
            .with_dimensions(2_560, 1_440),
        ),
        Box::new(FormatChangeEncoder {
            configure_count: Arc::clone(&configure_count),
            configured_dimensions: Arc::clone(&configured_dimensions),
            configured: false,
        }),
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start native-dimension engine");

    let deadline = Instant::now() + Duration::from_secs(2);
    while !engine.worker_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    engine.pump();

    assert_eq!(configure_count.load(Ordering::SeqCst), 2);
    assert_eq!(
        *configured_dimensions.lock().expect("configured dimensions"),
        vec![(2_560, 1_440), (1_280, 720)]
    );
    let snapshot = engine
        .snapshot_replay()
        .expect("snapshot after native format change");
    let video = snapshot
        .streams
        .iter()
        .find(|stream| stream.descriptor.media_type == MediaType::Video)
        .expect("video stream");
    assert_eq!(video.descriptor.width, Some(1_280));
    assert_eq!(video.descriptor.height, Some(720));
    engine.stop().expect("stop engine");
}

#[test]
fn delayed_encoder_output_primes_buffer_after_final_drain() {
    let mut engine = RecorderEngine::new(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(TwoFrameCapture::with_frame_count(20)),
        Box::new(DrainOnlyEncoder {
            configured: false,
            pending: Vec::new(),
        }),
        Box::new(MockMuxer::default()) as Box<dyn Muxer>,
    );
    engine.start().expect("start engine");

    let deadline = Instant::now() + Duration::from_secs(2);
    while !engine.worker_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    engine.pump();

    assert_eq!(engine.state(), recorder_engine::RecorderState::Ready);
    assert_eq!(engine.buffered_packet_count(), 20);
    engine.stop().expect("stop engine");
}

#[test]
fn unrecoverable_video_encoder_failure_transitions_engine_to_error() {
    let mut video_encoder = MockVideoEncoder::new();
    video_encoder.fail_on_encode = Some(0);
    let mut engine = RecorderEngine::new(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(TwoFrameCapture::with_frame_count(2)),
        Box::new(video_encoder),
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start engine");

    let deadline = Instant::now() + Duration::from_secs(2);
    while !engine.worker_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }

    let events = engine.pump();
    assert_eq!(engine.state(), recorder_engine::RecorderState::Error);
    assert!(events.iter().any(|event| matches!(
        event,
        recorder_engine::EngineEvent::Warning { code, message }
            if code == "ENCODER_ENCODE_FAILED" && message.contains("injected failure")
    )));
    engine.stop().expect("stop failed engine");
}

#[test]
fn audio_worker_synchronizes_and_inserts_a_separate_stream() {
    let mut engine = RecorderEngine::new_with_audio(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(TwoFrameCapture::with_frame_count(20)),
        Box::new(MockVideoEncoder::new()),
        vec![AudioInput::new(
            StreamId(1),
            "desktop",
            Box::new(ScriptedAudioCapture::new(vec![AudioStep::Chunks(5)])),
            Box::new(MockAudioEncoder::default()),
        )],
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start engine");

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if engine
            .audio_sync_metrics()
            .get(&StreamId(1))
            .is_some_and(|metrics| metrics.output_chunks == 5)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    let metrics = engine
        .audio_sync_metrics()
        .remove(&StreamId(1))
        .expect("desktop audio metrics");
    assert_eq!(metrics.input_chunks, 5);
    assert_eq!(metrics.output_chunks, 5);
    assert!(engine.buffered_packet_count() >= 25);
    engine.stop().expect("stop engine");
}

#[test]
fn audio_codec_extradata_reaches_snapshot_descriptor() {
    let recorded = Arc::new(Mutex::new(None));
    let mut engine = RecorderEngine::new_with_audio(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(TwoFrameCapture::with_frame_count(20)),
        Box::new(MockVideoEncoder::new()),
        vec![AudioInput::new(
            StreamId(1),
            "desktop",
            Box::new(ScriptedAudioCapture::new(vec![AudioStep::Chunks(5)])),
            Box::new(MockAudioEncoder::default().with_extradata(vec![0x11_u8, 0x90])),
        )],
        Box::new(SnapshotRecordingMuxer {
            snapshot: Arc::clone(&recorded),
        }),
    );
    engine.start().expect("start engine");

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        engine.pump();
        if engine.state() == recorder_engine::RecorderState::Ready
            && engine
                .audio_sync_metrics()
                .get(&StreamId(1))
                .is_some_and(|metrics| metrics.output_chunks == 5)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(engine.state(), recorder_engine::RecorderState::Ready);

    engine
        .save_replay(&std::env::temp_dir().join("silk-audio-metadata-test.mp4"))
        .expect("save snapshot");
    let snapshot = recorded
        .lock()
        .expect("snapshot lock")
        .clone()
        .expect("recorded snapshot");
    let video = snapshot
        .streams
        .iter()
        .find(|stream| stream.descriptor.media_type == MediaType::Video)
        .expect("video stream");
    assert!(video.descriptor.extradata.is_some());
    let audio = snapshot
        .streams
        .iter()
        .find(|stream| stream.descriptor.stream_id == StreamId(1))
        .expect("audio stream");
    assert_eq!(
        audio.descriptor.extradata,
        Some(PacketPayload::from(vec![0x11_u8, 0x90]))
    );
    engine.stop().expect("stop engine");
}

#[test]
fn microphone_loss_does_not_stop_desktop_audio_worker() {
    let mut engine = RecorderEngine::new_with_audio(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(TwoFrameCapture::with_frame_count(20)),
        Box::new(MockVideoEncoder::new()),
        vec![
            AudioInput::new(
                StreamId(1),
                "desktop",
                Box::new(ScriptedAudioCapture::new(vec![AudioStep::Chunks(5)])),
                Box::new(MockAudioEncoder::default()),
            ),
            AudioInput::new(
                StreamId(2),
                "microphone",
                Box::new(ScriptedAudioCapture::new(vec![
                    AudioStep::Chunks(1),
                    AudioStep::DeviceLost,
                ])),
                Box::new(MockAudioEncoder::default()),
            ),
        ],
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start engine");

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let metrics = engine.audio_sync_metrics();
        if metrics
            .get(&StreamId(1))
            .is_some_and(|metrics| metrics.output_chunks == 5)
            && metrics
                .get(&StreamId(2))
                .is_some_and(|metrics| metrics.output_chunks == 1)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    let events = engine.pump();
    assert!(events.iter().any(|event| matches!(
        event,
        recorder_engine::EngineEvent::Warning { code, message }
            if code == "AUDIO_DEVICE_LOST" && message.contains("microphone")
    )));
    assert_ne!(engine.state(), recorder_engine::RecorderState::Error);
    assert_eq!(
        engine
            .audio_sync_metrics()
            .get(&StreamId(1))
            .map(|metrics| metrics.output_chunks),
        Some(5)
    );
    engine.stop().expect("stop engine");
}

#[test]
fn audio_device_loss_restarts_with_contiguous_timestamps() {
    let mut engine = RecorderEngine::new_with_audio(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(TwoFrameCapture::with_frame_count(20)),
        Box::new(MockVideoEncoder::new()),
        vec![AudioInput::new(
            StreamId(1),
            "desktop",
            Box::new(RestartingAudioCapture::new()),
            Box::new(MockAudioEncoder::default()),
        )],
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start engine");

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if engine
            .audio_sync_metrics()
            .get(&StreamId(1))
            .is_some_and(|metrics| metrics.output_chunks == 2)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    let events = engine.pump();
    assert!(events.iter().any(|event| matches!(
        event,
        recorder_engine::EngineEvent::Warning { code, message }
            if code == "AUDIO_DEVICE_LOST" && message.contains("recovery")
    )));
    let metrics = engine
        .audio_sync_metrics()
        .remove(&StreamId(1))
        .expect("desktop audio metrics");
    assert_eq!(metrics.output_chunks, 2);
    assert_eq!(metrics.last_drift_ms, 0);
    assert!(engine.buffered_packet_count() >= 22);
    engine.stop().expect("stop engine");
}

#[cfg(windows)]
fn run_native_hardware_encoding_test(codec: &str, expected_tag: &str) {
    use encoder_api::{AudioEncoderConfig, EncoderPreference};
    use encoder_media_foundation::{new_aac_encoder, new_fallback_encoder};
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    let (width, height) = if codec == "av1" {
        (1_280, 720)
    } else {
        (256, 256)
    };

    let video_config = VideoEncoderConfig {
        codec: codec.to_string(),
        width,
        height,
        fps: 30,
        bitrate_kbps: Some(2_500),
        ..VideoEncoderConfig::default()
    };
    let mut video = new_fallback_encoder(EncoderPreference::HardwareOnly);
    video
        .configure(video_config.clone())
        .unwrap_or_else(|error| {
            panic!("configure native hardware {codec} encoder failed: {error}")
        });

    let frame_len = (width as usize) * (height as usize) * 4;
    let mut video_packets = Vec::new();
    for index in 0..16_i64 {
        let mut frame = vec![0_u8; frame_len];
        for pixel in frame.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[32, 96, (index * 16) as u8, 255]);
        }
        video_packets.extend(
            video
                .encode(VideoFrame {
                    stream_id: StreamId(0),
                    width,
                    height,
                    pixel_format: PixelFormat::Bgra8,
                    pts: index * 33,
                    time_base: TimeBase::MILLISECOND,
                    payload: FramePayload::Cpu(frame.into()),
                })
                .unwrap_or_else(|error| {
                    panic!("encode native hardware {codec} frame {index} failed: {error}")
                }),
        );
    }
    video_packets.extend(
        video
            .drain()
            .unwrap_or_else(|error| panic!("drain native hardware {codec} failed: {error}")),
    );

    assert!(
        !video_packets.is_empty(),
        "hardware {codec} encoder must produce at least one packet"
    );
    assert!(
        video_packets.iter().any(|packet| packet.is_keyframe),
        "hardware {codec} encoder must produce at least one keyframe"
    );
    assert!(
        video_packets
            .iter()
            .all(|packet| !packet.payload.is_empty()),
        "hardware {codec} encoder must not produce empty packet payloads"
    );

    let video_extradata = video
        .codec_extradata()
        .unwrap_or_else(|| panic!("hardware {codec} encoder must expose codec extradata"));
    assert!(
        !video_extradata.is_empty(),
        "hardware {codec} extradata must not be empty"
    );

    let mut audio = new_aac_encoder();
    let audio_config = AudioEncoderConfig::default();
    audio
        .configure(audio_config.clone())
        .expect("configure native AAC");
    let mut audio_packets = Vec::new();
    for index in 0..16_i64 {
        let mut data = Vec::with_capacity(1_024 * 2 * 4);
        for _ in 0..1_024 * 2 {
            data.extend_from_slice(&0.0_f32.to_le_bytes());
        }
        audio_packets.extend(
            audio
                .encode(AudioFrame {
                    stream_id: StreamId(1),
                    sample_format: SampleFormat::F32,
                    sample_rate: audio_config.sample_rate,
                    channels: audio_config.channels,
                    sample_count: 1_024,
                    pts: index * 1_024,
                    time_base: TimeBase::from_hz(audio_config.sample_rate),
                    data: data.into(),
                })
                .expect("encode native AAC"),
        );
    }
    audio_packets.extend(audio.drain().expect("drain native AAC"));
    let audio_extradata = audio.codec_extradata().expect("AAC AudioSpecificConfig");

    let snapshot = MediaSnapshot {
        origin_pts: 0,
        time_base: TimeBase::MILLISECOND,
        streams: vec![
            StreamPackets {
                descriptor: StreamDescriptor {
                    stream_id: StreamId(0),
                    media_type: MediaType::Video,
                    time_base: TimeBase::MILLISECOND,
                    name: Some("Display".to_string()),
                    codec: codec.to_string(),
                    extradata: Some(video_extradata),
                    width: Some(video_config.width),
                    height: Some(video_config.height),
                    sample_rate: None,
                    channels: None,
                    pixel_format: None,
                },
                packets: video_packets.into_iter().map(Arc::new).collect(),
            },
            StreamPackets {
                descriptor: StreamDescriptor {
                    stream_id: StreamId(1),
                    media_type: MediaType::Audio,
                    time_base: TimeBase::MILLISECOND,
                    name: Some("Microphone".to_string()),
                    codec: "aac".to_string(),
                    extradata: Some(audio_extradata),
                    width: None,
                    height: None,
                    sample_rate: Some(audio_config.sample_rate),
                    channels: Some(audio_config.channels),
                    pixel_format: None,
                },
                packets: audio_packets.into_iter().map(Arc::new).collect(),
            },
        ],
        captured_at_unix_ms: 0,
    };

    let dir = test_support::TempDir::new("native-mp4").expect("temp");
    let path = dir.path().join("native.mp4");
    let mut muxer = Mp4Muxer;
    let metadata = muxer
        .write_snapshot(&snapshot, &path, &SaveOptions::default())
        .unwrap_or_else(|error| panic!("write native MP4 with {codec} failed: {error}"));
    assert!(metadata.size_bytes > 0);
    assert_eq!(metadata.audio_codecs, vec!["aac"]);
    let bytes = fs::read(&path).expect("read native MP4");
    assert_eq!(&bytes[4..8], b"ftyp");

    let script_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests dir")
        .parent()
        .expect("repo root")
        .join("scripts")
        .join("validate-media.ps1");
    assert!(
        script_path.exists(),
        "validate-media.ps1 must exist at {}",
        script_path.display()
    );

    let validation_output = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-File",
            &script_path.to_string_lossy(),
            "-ClipPath",
            &path.to_string_lossy(),
            "-ExpectedVideoCodec",
            codec,
            "-ExpectedVideoTag",
            expected_tag,
        ])
        .output()
        .expect("failed to execute validate-media.ps1 with pwsh");

    let stdout = String::from_utf8_lossy(&validation_output.stdout)
        .trim()
        .to_string();
    let stderr = String::from_utf8_lossy(&validation_output.stderr)
        .trim()
        .to_string();
    assert!(
        validation_output.status.success(),
        "validate-media.ps1 failed for {codec} (exit code {:?}):\nstdout: {}\nstderr: {}",
        validation_output.status.code(),
        stdout,
        stderr
    );

    println!("validator output for {codec}: {stdout}");
    assert!(
        stdout.contains("\"decoded\":true") || stdout.contains("\"decoded\": true"),
        "expected validator output to report decoded=true for {codec}, got: {stdout}"
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires hardware H.264 encoder, Media Foundation transforms, and ffmpeg/ffprobe on PATH"]
fn native_hardware_h264_clip_muxes_and_validates() {
    run_native_hardware_encoding_test("h264", "avc1");
}

#[cfg(windows)]
#[test]
#[ignore = "requires hardware HEVC encoder, Media Foundation transforms, and ffmpeg/ffprobe on PATH"]
fn native_hardware_hevc_clip_muxes_and_validates() {
    run_native_hardware_encoding_test("hevc", "hvc1");
}

#[cfg(windows)]
#[test]
#[ignore = "requires hardware AV1 encoder, Media Foundation transforms, and ffmpeg/ffprobe on PATH"]
fn native_hardware_av1_clip_muxes_and_validates() {
    run_native_hardware_encoding_test("av1", "av01");
}
