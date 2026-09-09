use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use app_controller::{
    Command, ConsoleNotificationSink, Controller, ControllerEvent, ControllerSettings,
    NotificationSink, SaveMetrics, SAVE_QUEUE_BOUND,
};
use clip_library::LibraryWorker;
use encoder_api::{
    EncoderCandidate, EncoderCapabilities, EncoderPreference, FallbackVideoEncoder, VideoEncoder,
    VideoEncoderFactory,
};
use media_types::{MediaSnapshot, PixelFormat};
use muxer::{ClipMetadata, Muxer, SaveOptions};
use recorder_engine::{
    AudioInput, EngineConfig, EngineEvent, RecorderEngine, RecorderState, ENGINE_EVENT_QUEUE_BOUND,
};
use test_support::{
    AudioStep, MockAudioEncoder, MockMuxer, MockVideoEncoder, ScriptedAudioCapture,
    ScriptedVideoCapture, TempDir, VideoStep,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

fn settings(dir: &TempDir, retention_ms: i64) -> ControllerSettings {
    ControllerSettings {
        output_dir: dir.path().to_path_buf(),
        file_name_pattern: "s8_{index}".to_string(),
        retention_ms,
        minimum_free_space_bytes: 0,
        audio_tracks: Vec::new(),
        ..ControllerSettings::default()
    }
}

fn mock_controller(
    dir: &TempDir,
    steps: Vec<VideoStep>,
    continuous: bool,
    frame_delay_ms: u64,
    write_delay_ms: u64,
) -> Controller {
    let output_dir = dir.path().to_path_buf();
    let sink: Arc<dyn NotificationSink> = Arc::new(ConsoleNotificationSink);
    Controller::with_settings(
        Box::new(move || {
            let mut capture = ScriptedVideoCapture::new(steps.clone());
            if continuous {
                capture = capture.with_continuous_frames();
            }
            capture = capture.with_frame_delay_ms(frame_delay_ms);
            let mut muxer = MockMuxer::default();
            muxer.write_delay_ms = write_delay_ms;
            (
                Box::new(capture) as Box<dyn capture_api::VideoCapture>,
                Box::new(MockVideoEncoder::new()) as Box<dyn encoder_api::VideoEncoder>,
                Box::new(muxer) as Box<dyn muxer::Muxer>,
            )
        }),
        sink,
        ControllerSettings {
            output_dir,
            ..settings(dir, 100)
        },
    )
}

fn wait_controller_ready(controller: &mut Controller) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    while Instant::now() < deadline {
        controller.poll();
        if controller.state() == RecorderState::Ready {
            return;
        }
        std::thread::yield_now();
    }
    panic!(
        "controller did not become ready: state={:?}, packets={}",
        controller.state(),
        controller.buffered_packet_count()
    );
}

fn save_finished(
    controller: &mut Controller,
    mut events: Vec<ControllerEvent>,
) -> Vec<ControllerEvent> {
    let deadline = Instant::now() + TEST_TIMEOUT;
    while Instant::now() < deadline {
        if events.iter().any(|event| {
            matches!(
                event,
                ControllerEvent::ClipSaved { .. } | ControllerEvent::SaveFailed { .. }
            )
        }) {
            return events;
        }
        events.extend(controller.poll());
        std::thread::yield_now();
    }
    panic!("save did not complete: {events:?}");
}

fn count_completed(events: &[ControllerEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, ControllerEvent::ClipSaved { .. }))
        .count()
}

fn regular_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .expect("read output directory")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_file())
                .map(|_| entry.path())
        })
        .collect()
}

fn wait_engine_ready(engine: &mut RecorderEngine) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    while Instant::now() < deadline {
        engine.pump();
        if engine.state() == RecorderState::Ready {
            return;
        }
        std::thread::yield_now();
    }
    panic!("engine did not become ready: {:?}", engine.state());
}

fn finish_engine(engine: &mut RecorderEngine) -> Vec<EngineEvent> {
    let deadline = Instant::now() + TEST_TIMEOUT;
    let mut events = Vec::new();
    while !engine.worker_finished() && Instant::now() < deadline {
        events.extend(engine.pump());
        std::thread::yield_now();
    }
    events.extend(engine.pump());
    assert!(engine.worker_finished(), "engine worker did not finish");
    events
}

struct GateMuxer {
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    inner: MockMuxer,
}

impl GateMuxer {
    fn new(entered: Arc<AtomicBool>, release: Arc<AtomicBool>) -> Self {
        Self {
            entered,
            release,
            inner: MockMuxer::default(),
        }
    }
}

impl Muxer for GateMuxer {
    fn write_snapshot(
        &mut self,
        snapshot: &MediaSnapshot,
        final_path: &Path,
        options: &SaveOptions,
    ) -> muxer::Result<ClipMetadata> {
        self.entered.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        self.inner.write_snapshot(snapshot, final_path, options)
    }
}

fn gated_controller(
    dir: &TempDir,
    entered: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
) -> Controller {
    let output_dir = dir.path().to_path_buf();
    let sink: Arc<dyn NotificationSink> = Arc::new(ConsoleNotificationSink);
    Controller::with_settings(
        Box::new(move || {
            (
                Box::new(
                    ScriptedVideoCapture::new(Vec::new())
                        .with_continuous_frames()
                        .with_frame_delay_ms(1),
                ) as Box<dyn capture_api::VideoCapture>,
                Box::new(MockVideoEncoder::new()) as Box<dyn encoder_api::VideoEncoder>,
                Box::new(GateMuxer::new(Arc::clone(&entered), Arc::clone(&release)))
                    as Box<dyn muxer::Muxer>,
            )
        }),
        sink,
        ControllerSettings {
            output_dir,
            ..settings(dir, 100)
        },
    )
}

#[test]
fn controller_restarts_with_fresh_video_and_audio_factories() {
    let dir = TempDir::new("s8-restart").expect("temp dir");
    let video_creations = Arc::new(AtomicUsize::new(0));
    let audio_creations = Arc::new(AtomicUsize::new(0));
    let video_counter = Arc::clone(&video_creations);
    let audio_counter = Arc::clone(&audio_creations);
    let sink: Arc<dyn NotificationSink> = Arc::new(ConsoleNotificationSink);
    let output_dir = dir.path().to_path_buf();
    let mut controller = Controller::with_audio_factory(
        Box::new(move || {
            video_counter.fetch_add(1, Ordering::SeqCst);
            (
                Box::new(
                    ScriptedVideoCapture::new(Vec::new())
                        .with_continuous_frames()
                        .with_frame_delay_ms(1),
                ) as Box<dyn capture_api::VideoCapture>,
                Box::new(MockVideoEncoder::new()) as Box<dyn encoder_api::VideoEncoder>,
                Box::new(MockMuxer::default()) as Box<dyn muxer::Muxer>,
            )
        }),
        Box::new(move |_plans| {
            audio_counter.fetch_add(1, Ordering::SeqCst);
            vec![AudioInput::new(
                media_types::StreamId(1),
                "desktop",
                Box::new(ScriptedAudioCapture::new(vec![AudioStep::Chunks(4)])),
                Box::new(MockAudioEncoder::default()),
            )]
        }),
        sink,
        ControllerSettings {
            output_dir,
            ..settings(&dir, 100)
        },
    );

    for cycle in 0..6 {
        controller.execute(&Command::StartCapture);
        wait_controller_ready(&mut controller);

        let mut changed = settings(&dir, 100 + i64::from(cycle) * 20);
        changed.file_name_pattern = "s8-cycle_{index}".to_string();
        controller
            .update_settings(changed)
            .expect("live settings update");

        controller.execute(&Command::StopCapture);
        assert_eq!(controller.state(), RecorderState::Stopped);
    }

    assert_eq!(video_creations.load(Ordering::SeqCst), 6);
    assert_eq!(audio_creations.load(Ordering::SeqCst), 6);
}

#[test]
fn controller_can_retry_after_rejected_start() {
    let dir = TempDir::new("s8-start-retry").expect("temp dir");
    let sink: Arc<dyn NotificationSink> = Arc::new(ConsoleNotificationSink);
    let mut invalid = settings(&dir, 100);
    invalid.video_encoder_config.fps = 24;
    let mut controller = Controller::with_settings(
        Box::new(|| {
            (
                Box::new(ScriptedVideoCapture::new(vec![VideoStep::Frames(8)]))
                    as Box<dyn capture_api::VideoCapture>,
                Box::new(MockVideoEncoder::new()) as Box<dyn encoder_api::VideoEncoder>,
                Box::new(MockMuxer::default()) as Box<dyn muxer::Muxer>,
            )
        }),
        sink,
        invalid,
    );

    let rejected = controller.execute(&Command::StartCapture);
    assert!(rejected.iter().any(|event| {
        matches!(event, ControllerEvent::Warning { code, .. } if code == "ENCODER_NOT_AVAILABLE")
    }));
    assert_eq!(controller.state(), RecorderState::Stopped);

    controller
        .update_settings(settings(&dir, 100))
        .expect("repair settings");
    controller.execute(&Command::StartCapture);
    wait_controller_ready(&mut controller);
    controller.execute(&Command::StopCapture);
}

#[test]
fn one_hundred_sequential_saves_complete_without_staged_files() {
    let dir = TempDir::new("s8-100-saves").expect("temp dir");
    let mut controller = mock_controller(&dir, Vec::new(), true, 1, 0);
    controller.execute(&Command::StartCapture);
    wait_controller_ready(&mut controller);

    for _ in 0..100 {
        let events = controller.execute(&Command::SaveReplay);
        assert!(events
            .iter()
            .any(|event| matches!(event, ControllerEvent::SaveQueued { .. })));
        let completed = save_finished(&mut controller, events);
        assert_eq!(count_completed(&completed), 1);
        assert!(!completed
            .iter()
            .any(|event| { matches!(event, ControllerEvent::SaveFailed { .. }) }));
    }

    let files = regular_files(dir.path());
    assert_eq!(files.len(), 100);
    assert!(files
        .iter()
        .all(|path| path.extension().is_some_and(|ext| ext == "mp4")));
    assert!(regular_files(dir.path())
        .iter()
        .all(|path| { path.extension().is_none_or(|extension| extension != "part") }));

    let metrics = controller.save_metrics();
    assert_eq!(metrics.saves_queued, 100);
    assert_eq!(metrics.saves_completed, 100);
    assert_eq!(metrics.saves_failed, 0);
    controller.execute(&Command::StopCapture);
}

#[test]
fn saving_and_library_scan_overlap_an_active_capture() {
    let dir = TempDir::new("s8-concurrent-save").expect("temp dir");
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let mut controller = gated_controller(&dir, Arc::clone(&entered), Arc::clone(&release));
    let library = LibraryWorker::new(dir.path().to_path_buf(), dir.path().join("catalog.json"))
        .expect("library worker");

    controller.execute(&Command::StartCapture);
    wait_controller_ready(&mut controller);
    let before = controller.buffered_packet_count();
    let queued = controller.execute(&Command::SaveReplay);
    assert!(queued
        .iter()
        .any(|event| matches!(event, ControllerEvent::SaveQueued { .. })));

    let entered_deadline = Instant::now() + TEST_TIMEOUT;
    while !entered.load(Ordering::Acquire) && Instant::now() < entered_deadline {
        controller.poll();
        std::thread::yield_now();
    }
    assert!(
        entered.load(Ordering::Acquire),
        "save worker did not enter muxer"
    );

    let growth_deadline = Instant::now() + TEST_TIMEOUT;
    let mut grew = false;
    while Instant::now() < growth_deadline {
        controller.poll();
        if controller.buffered_packet_count() > before {
            grew = true;
            break;
        }
        std::thread::yield_now();
    }
    assert!(grew, "capture buffer did not progress during gated save");
    assert_eq!(controller.state(), RecorderState::Ready);
    assert!(library.scan().is_ok(), "library scan failed during save");

    release.store(true, Ordering::Release);
    let completed = save_finished(&mut controller, queued);
    assert_eq!(count_completed(&completed), 1);
    assert!(library
        .scan()
        .expect("post-save scan")
        .iter()
        .any(|entry| !entry.missing));

    controller.execute(&Command::StopCapture);
    assert!(regular_files(dir.path())
        .iter()
        .all(|path| { path.extension().is_none_or(|extension| extension != "part") }));
}

#[test]
fn display_loss_recovers_and_starts_a_new_epoch() {
    let video_encoder = MockVideoEncoder::new();
    let configure_count = Arc::clone(&video_encoder.configure_count);
    let mut engine = RecorderEngine::new(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(ScriptedVideoCapture::new(vec![
            VideoStep::Frames(8),
            VideoStep::SourceLost,
            VideoStep::Frames(8),
        ])),
        Box::new(video_encoder),
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start engine");
    let events = finish_engine(&mut engine);

    assert_eq!(engine.state(), RecorderState::Ready);
    assert_eq!(engine.buffer_epoch(), 1);
    assert_eq!(configure_count.load(Ordering::SeqCst), 2);
    assert!(events.iter().any(|event| {
        matches!(event, EngineEvent::Warning { code, .. } if code == "CAPTURE_DEVICE_LOST")
    }));
    assert!(events
        .iter()
        .any(|event| { matches!(event, EngineEvent::StateChanged(RecorderState::Recovering)) }));
    engine.stop().expect("stop recovered engine");
}

#[test]
fn display_recovery_retries_are_bounded_and_stoppable() {
    let mut engine = RecorderEngine::new(
        EngineConfig::default(),
        Box::new(
            ScriptedVideoCapture::new(vec![VideoStep::Frames(1), VideoStep::SourceLost])
                .with_recovery_failures(recorder_engine::VIDEO_RESTART_LIMIT as usize),
        ),
        Box::new(MockVideoEncoder::new()),
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start engine");
    let events = finish_engine(&mut engine);

    assert_eq!(engine.state(), RecorderState::Error);
    assert!(events.iter().any(|event| {
        matches!(event, EngineEvent::Warning { code, .. } if code == "CAPTURE_RECOVERY_EXHAUSTED")
    }));
    engine.stop().expect("stop after bounded recovery failure");
    assert_eq!(engine.state(), RecorderState::Stopped);
}

#[test]
fn audio_device_loss_recovers_without_stopping_video() {
    let mut engine = RecorderEngine::new_with_audio(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(ScriptedVideoCapture::new(vec![VideoStep::Frames(24)])),
        Box::new(MockVideoEncoder::new()),
        vec![AudioInput::new(
            media_types::StreamId(1),
            "desktop",
            Box::new(ScriptedAudioCapture::new(vec![
                AudioStep::Chunks(2),
                AudioStep::DeviceLost,
                AudioStep::Chunks(2),
            ])),
            Box::new(MockAudioEncoder::default()),
        )],
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start engine");

    let deadline = Instant::now() + TEST_TIMEOUT;
    let mut events = Vec::new();
    while Instant::now() < deadline {
        events.extend(engine.pump());
        if engine
            .audio_sync_metrics()
            .get(&media_types::StreamId(1))
            .is_some_and(|metrics| metrics.output_chunks == 4)
        {
            break;
        }
        std::thread::yield_now();
    }
    events.extend(engine.pump());

    assert_eq!(
        engine
            .audio_sync_metrics()
            .get(&media_types::StreamId(1))
            .map(|metrics| metrics.output_chunks),
        Some(4)
    );
    assert!(events.iter().any(|event| {
        matches!(event, EngineEvent::Warning { code, .. } if code == "AUDIO_DEVICE_LOST")
    }));
    assert_ne!(engine.state(), RecorderState::Error);
    engine.stop().expect("stop audio recovery engine");
}

#[test]
fn resume_discards_old_packets_and_restarts_buffering_in_a_new_epoch() {
    let mut video_encoder = MockVideoEncoder::new();
    video_encoder.keyframe_every = 1;
    let mut engine = RecorderEngine::new(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(
            ScriptedVideoCapture::new(Vec::new())
                .with_continuous_frames()
                .with_frame_delay_ms(1),
        ),
        Box::new(video_encoder),
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start engine");
    wait_engine_ready(&mut engine);
    let before = engine.snapshot_replay().expect("pre-resume snapshot");
    let before_max_sequence = before.streams[0]
        .packets
        .iter()
        .map(|packet| packet.sequence)
        .max()
        .expect("pre-resume packet");
    let previous_epoch = engine.buffer_epoch();

    engine.notify_resume().expect("resume boundary");
    assert_eq!(engine.state(), RecorderState::Buffering);
    assert_eq!(engine.buffer_epoch(), previous_epoch + 1);
    wait_engine_ready(&mut engine);

    let after = engine.snapshot_replay().expect("post-resume snapshot");
    let after_min_sequence = after.streams[0]
        .packets
        .iter()
        .map(|packet| packet.sequence)
        .min()
        .expect("post-resume packet");
    assert!(after_min_sequence > before_max_sequence);
    engine.stop().expect("stop resumed engine");
}

struct InjectedFallbackFactory;

fn fallback_candidate(
    id: &str,
    backend_name: &str,
    hardware_accelerated: bool,
) -> EncoderCandidate {
    EncoderCandidate {
        id: id.to_string(),
        capabilities: EncoderCapabilities {
            backend_name: backend_name.to_string(),
            codec: "h264".to_string(),
            hardware_accelerated,
            max_width: 3_840,
            max_height: 2_160,
            supported_fps: vec![30, 60],
            supported_pixel_formats: vec![PixelFormat::Nv12],
        },
    }
}

impl VideoEncoderFactory for InjectedFallbackFactory {
    fn discover(&self) -> encoder_api::Result<Vec<EncoderCandidate>> {
        Ok(vec![
            fallback_candidate("preferred", "injected-preferred", true),
            fallback_candidate("software", "injected-software", false),
        ])
    }

    fn create(&self, candidate: &EncoderCandidate) -> encoder_api::Result<Box<dyn VideoEncoder>> {
        let mut encoder = MockVideoEncoder::new();
        encoder.backend_name = Some(candidate.capabilities.backend_name.clone());
        encoder.hardware_accelerated = candidate.capabilities.hardware_accelerated;
        if candidate.id == "preferred" {
            encoder.fail_on_encode = Some(0);
        }
        Ok(Box::new(encoder))
    }
}

#[test]
fn injected_encoder_failure_falls_back_and_separates_buffer_epoch() {
    let encoder =
        FallbackVideoEncoder::new(Box::new(InjectedFallbackFactory), EncoderPreference::Auto);
    let mut engine = RecorderEngine::new(
        EngineConfig {
            retention_ms: 100,
            ..EngineConfig::default()
        },
        Box::new(ScriptedVideoCapture::new(vec![VideoStep::Frames(24)])),
        Box::new(encoder),
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start fallback engine");
    let events = finish_engine(&mut engine);

    let metrics = engine.encoder_metrics();
    assert_eq!(metrics.active_backend.as_deref(), Some("injected-software"));
    assert_eq!(metrics.fallback_count, 2);
    assert_eq!(metrics.epoch, 3);
    assert_eq!(engine.buffer_epoch(), 1);
    assert!(events.iter().any(|event| {
        matches!(event, EngineEvent::Warning { code, .. } if code == "ENCODER_FALLBACK")
    }));
    engine.stop().expect("stop fallback engine");
}

#[test]
fn startup_cleans_stale_parts_and_preserves_completed_files() {
    let dir = TempDir::new("s8-stale-parts").expect("temp dir");
    let stale = dir.path().join("crashed.mp4.part");
    let complete = dir.path().join("complete.mp4");
    std::fs::write(&stale, b"partial output").expect("stale part");
    std::fs::write(&complete, b"complete output").expect("complete output");

    let mut controller = mock_controller(&dir, vec![VideoStep::Frames(8)], false, 0, 0);
    controller.execute(&Command::StartCapture);
    assert!(!stale.exists());
    assert!(complete.exists());
    controller.execute(&Command::StopCapture);
    assert!(!stale.exists());
}

#[test]
fn bounded_event_queue_drops_diagnostics_without_blocking_worker() {
    let mut steps = Vec::new();
    for index in 0..(ENGINE_EVENT_QUEUE_BOUND + 16) {
        steps.push(VideoStep::FormatChange {
            width: 1_920 + (index as u32 % 4) * 2,
            height: 1_080,
        });
    }
    steps.push(VideoStep::Frames(1));

    let mut engine = RecorderEngine::new(
        EngineConfig::default(),
        Box::new(ScriptedVideoCapture::new(steps)),
        Box::new(MockVideoEncoder::new()),
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start event queue test");
    let deadline = Instant::now() + TEST_TIMEOUT;
    while !engine.worker_finished() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(engine.worker_finished(), "worker blocked on event delivery");
    assert!(engine.dropped_event_count() > 0);
    engine.stop().expect("stop event queue test");
}

#[test]
fn save_queue_and_replay_store_remain_bounded() {
    let dir = TempDir::new("s8-bounds").expect("temp dir");
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let mut controller = gated_controller(&dir, Arc::clone(&entered), Arc::clone(&release));
    controller.execute(&Command::StartCapture);
    wait_controller_ready(&mut controller);

    let first = controller.execute(&Command::SaveReplay);
    let entered_deadline = Instant::now() + TEST_TIMEOUT;
    while !entered.load(Ordering::Acquire) && Instant::now() < entered_deadline {
        controller.poll();
        std::thread::yield_now();
    }
    assert!(entered.load(Ordering::Acquire));

    let mut submissions = first;
    for _ in 0..SAVE_QUEUE_BOUND {
        let events = controller.execute(&Command::SaveReplay);
        assert!(events
            .iter()
            .any(|event| matches!(event, ControllerEvent::SaveQueued { .. })));
        submissions.extend(events);
    }
    let overflow = controller.execute(&Command::SaveReplay);
    assert!(overflow.iter().any(|event| {
        matches!(event, ControllerEvent::SaveFailed { code, .. } if code == "SAVE_QUEUE_FULL")
    }));

    release.store(true, Ordering::Release);
    let deadline = Instant::now() + TEST_TIMEOUT;
    while count_completed(&submissions) < SAVE_QUEUE_BOUND + 1 && Instant::now() < deadline {
        submissions.extend(controller.poll());
        std::thread::yield_now();
    }
    assert_eq!(count_completed(&submissions), SAVE_QUEUE_BOUND + 1);
    let metrics: SaveMetrics = controller.save_metrics();
    assert_eq!(metrics.saves_queued, SAVE_QUEUE_BOUND as u64 + 1);
    assert_eq!(metrics.queue_full, 1);
    controller.execute(&Command::StopCapture);

    let mut engine = RecorderEngine::new(
        EngineConfig {
            retention_ms: 60_000,
            max_packets_per_stream: 4,
            ..EngineConfig::default()
        },
        Box::new(ScriptedVideoCapture::new(vec![VideoStep::Frames(64)])),
        Box::new(MockVideoEncoder::new()),
        Box::new(MockMuxer::default()),
    );
    engine.start().expect("start replay bound test");
    finish_engine(&mut engine);
    assert!(engine.buffered_packet_count() <= 4);
    engine.stop().expect("stop replay bound test");
}
