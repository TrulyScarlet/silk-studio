#![cfg(windows)]
//! Explicitly opt-in: reads configured endpoints and records a local test clip.
use std::time::{Duration, Instant};

use audio_wasapi::WasapiAudioCapture;
use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
use capture_windows::DisplayCapture;
use encoder_api::{EncoderPreference, GpuFrameContext, VideoEncoderConfig};
use media_types::{MediaType, StreamId, VideoSourceInfo};
use recorder_engine::{AudioInput, EngineConfig, EngineEvent, RecorderEngine, RecorderState};

struct RecoveringCapture {
    capture: DisplayCapture,
    started: Option<Instant>,
    losses: u32,
}

impl VideoCapture for RecoveringCapture {
    fn enumerate_sources(&self) -> capture_api::Result<Vec<VideoSourceInfo>> {
        self.capture.enumerate_sources()
    }
    fn start(&mut self, config: VideoCaptureConfig) -> capture_api::Result<()> {
        self.capture.start(config)?;
        self.started.get_or_insert_with(Instant::now);
        Ok(())
    }
    fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
        if self.losses < 2
            && self.started.unwrap().elapsed()
                >= Duration::from_secs(3 * u64::from(self.losses + 1))
        {
            self.losses += 1;
            return Ok(VideoCaptureEvent::SourceLost);
        }
        self.capture.next_event()
    }
    fn stop(&mut self) -> capture_api::Result<()> {
        self.capture.stop()
    }
    fn gpu_frame_context(&self) -> Option<GpuFrameContext> {
        self.capture.gpu_frame_context()
    }
}

#[test]
#[ignore = "requires live desktop/audio; set SILK_RECOVERY_CONFIG to an explicit config path"]
fn native_display_restarts_preserve_all_configured_audio_tracks() {
    let path = std::env::var("SILK_RECOVERY_CONFIG").expect("explicit test config path");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let tracks = config["settings"]["audio"]["tracks"].as_array().unwrap();
    let inputs: Vec<_> = tracks
        .iter()
        .filter(|t| t["enabled"] == true)
        .enumerate()
        .map(|(i, t)| {
            let capture = if t["sourceKind"] == "input" {
                WasapiAudioCapture::microphone()
            } else {
                WasapiAudioCapture::loopback()
            };
            let mut input = AudioInput::new(
                StreamId(i as u32 + 1),
                t["name"].as_str().unwrap(),
                Box::new(capture),
                encoder_media_foundation::new_aac_encoder(),
            );
            input.capture_config.device_id = t["deviceId"].as_str().map(str::to_owned);
            input
        })
        .collect();
    let track_count = inputs.len();
    assert!(track_count > 0);
    let mut engine = RecorderEngine::new_with_audio(
        EngineConfig {
            retention_ms: 60_000,
            video_encoder: VideoEncoderConfig {
                width: 1280,
                height: 720,
                fps: 60,
                ..VideoEncoderConfig::default()
            },
            ..EngineConfig::default()
        },
        Box::new(RecoveringCapture {
            capture: DisplayCapture::new(StreamId(0)),
            started: None,
            losses: 0,
        }),
        Box::new(encoder_media_foundation::new_fallback_encoder(
            EncoderPreference::Auto,
        )),
        inputs,
        Box::new(muxer::Mp4Muxer),
    );
    engine.start().unwrap();
    let started = Instant::now();
    let mut recovered = 0;
    while started.elapsed() < Duration::from_secs(12) {
        for event in engine.pump() {
            match event {
                EngineEvent::WorkerFailed { code, message } => panic!("{code}: {message}"),
                EngineEvent::StateChanged(RecorderState::Recovering) => recovered += 1,
                EngineEvent::Warning { code, message } => println!("{code}: {message}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(recovered, 2);
    assert_eq!(engine.state(), RecorderState::Buffering);
    assert!(!engine.worker_finished());
    let snapshot = engine.snapshot_replay().unwrap();
    let video = snapshot
        .streams
        .iter()
        .find(|s| s.descriptor.media_type == MediaType::Video)
        .unwrap();
    let video_start = video.packets.first().unwrap().pts;
    let video_end = video.packets.last().unwrap().pts + video.packets.last().unwrap().duration;
    assert!(video_end - video_start > 4_000);
    assert_eq!(snapshot.streams.len(), track_count + 1);
    for stream in &snapshot.streams {
        let first = stream.packets.first().unwrap().pts;
        let last = stream.packets.last().unwrap();
        let end = last.pts + last.duration;
        println!(
            "{:?}: start={first}ms end={end}ms packets={}",
            stream.descriptor.name,
            stream.packets.len()
        );
        assert!(
            (first - video_start).abs() < 250,
            "track must start alongside recovered video"
        );
        assert!(
            (end - video_end).abs() < 250,
            "track must reach end of recovered video"
        );
    }
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/dxgi-evidence");
    std::fs::create_dir_all(&output).unwrap();
    let clip = output.join(format!("recovered-audio-{}.mp4", std::process::id()));
    engine.save_replay(&clip).unwrap();
    engine.stop().unwrap();
    println!("Recovery clip: {}", clip.display());
}
