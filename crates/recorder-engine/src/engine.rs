use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use audio_api::{AudioCapture, AudioCaptureConfig, AudioCaptureError, AudioCaptureEvent};
use audio_sync::{AudioSyncConfig, AudioSyncMetrics, AudioSynchronizer};
use capture_api::{VideoCapture, VideoCaptureEvent};
use encoder_api::{AudioEncoder, AudioEncoderConfig, EncoderEvent, EncoderMetrics, VideoEncoder};
use media_types::{MediaSnapshot, MediaType, StreamDescriptor, StreamId, TimeBase};
use muxer::{ClipMetadata, Muxer, SaveOptions};
use replay_buffer::{normalize_timestamps, BufferConfig, ReplayBuffer};

use crate::error::EngineError;
use crate::state::{validate_transition, RecorderState};

pub const AUDIO_RESTART_LIMIT: u32 = 3;
pub const VIDEO_RESTART_LIMIT: u32 = 5;

/// Maximum number of lifecycle and diagnostic events retained when the
/// controller is between polls. Workers use non-blocking sends and count
/// overflow rather than waiting on UI work.
pub const ENGINE_EVENT_QUEUE_BOUND: usize = 64;

pub const CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE: &str = "CLARITY_GPU_CONTEXT_UNAVAILABLE";
pub const CLARITY_GPU_CONTEXT_UNAVAILABLE_MESSAGE: &str =
    "Capture GPU context is not available for clarity supersampling; falling back to native resolution.";

pub const CLARITY_DIMENSIONS_OVERFLOW_CODE: &str = "CLARITY_DIMENSIONS_OVERFLOW";
pub const CLARITY_DIMENSIONS_OVERFLOW_MESSAGE: &str =
    "2x supersampled dimensions exceed valid video bounds; falling back to native resolution.";

pub const CLARITY_ENCODER_NEGOTIATION_FAILED_CODE: &str = "CLARITY_ENCODER_NEGOTIATION_FAILED";
pub const CLARITY_ENCODER_NEGOTIATION_FAILED_MESSAGE: &str =
    "Encoder rejected preferred 2x supersampled format; falling back to native resolution.";

pub const CLARITY_STARTUP_PROBE_FAILED_CODE: &str = "CLARITY_STARTUP_PROBE_FAILED";
pub const CLARITY_STARTUP_PROBE_FAILED_MESSAGE: &str =
    "First 2x frame probe failed; falling back to native resolution.";

/// Scaling intent for video capture output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EngineScalingPlan {
    #[default]
    Native1x,
    Prefer2xWithFallback,
}

/// Fallback reason when preferred supersampling cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalingFallbackReason {
    GpuContextUnavailable,
    DimensionsOverflow,
    EncoderNegotiationFailed { details: Option<String> },
    StartupProbeFailed { details: Option<String> },
}

impl ScalingFallbackReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::GpuContextUnavailable => CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE,
            Self::DimensionsOverflow => CLARITY_DIMENSIONS_OVERFLOW_CODE,
            Self::EncoderNegotiationFailed { .. } => CLARITY_ENCODER_NEGOTIATION_FAILED_CODE,
            Self::StartupProbeFailed { .. } => CLARITY_STARTUP_PROBE_FAILED_CODE,
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::GpuContextUnavailable => CLARITY_GPU_CONTEXT_UNAVAILABLE_MESSAGE,
            Self::DimensionsOverflow => CLARITY_DIMENSIONS_OVERFLOW_MESSAGE,
            Self::EncoderNegotiationFailed { .. } => CLARITY_ENCODER_NEGOTIATION_FAILED_MESSAGE,
            Self::StartupProbeFailed { .. } => CLARITY_STARTUP_PROBE_FAILED_MESSAGE,
        }
    }
}

/// Worker-written scaling state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoScalingState {
    Pending,
    ActiveNative1x,
    ActiveSupersampled2x,
    Failed,
}

/// Scaling status telemetry snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoScalingStatus {
    pub state: VideoScalingState,
    pub active_dimensions: Option<(u32, u32)>,
    pub fallback_reason: Option<ScalingFallbackReason>,
}

impl Default for VideoScalingStatus {
    fn default() -> Self {
        Self {
            state: VideoScalingState::Pending,
            active_dimensions: None,
            fallback_reason: None,
        }
    }
}

/// Engine tuning knobs.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub video_stream_id: StreamId,
    /// Source identifier passed to the capture backend at start.
    pub video_source_id: String,
    /// Replay retention window in ms (BUF-002/003). The desktop UI plumbs
    /// the user setting here; the small default keeps synthetic pipelines
    /// priming instantly (real defaults arrive with settings wiring).
    pub retention_ms: i64,
    /// Absolute per-stream packet cap — safety valve only (ADR 0004).
    pub max_packets_per_stream: usize,
    /// Parameters applied to the video encoder at start (ENC-005/008).
    /// The configured FPS doubles as the capture target rate (VID-004).
    pub video_encoder: encoder_api::VideoEncoderConfig,
    /// Resolve output dimensions from the selected source at start and update
    /// them when capture reports a format change.
    pub follow_source_dimensions: bool,
    /// Video scaling plan (Native 1x or Prefer 2x with fallback).
    pub scaling_plan: EngineScalingPlan,
    /// Target format applied to every enabled audio input.
    pub audio_encoder: AudioEncoderConfig,
    /// Shared-timeline and conversion policy applied independently per audio
    /// input.
    pub audio_sync: AudioSyncConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            video_stream_id: StreamId(0),
            video_source_id: "display-1".to_string(),
            retention_ms: 400,
            max_packets_per_stream: 20_000,
            video_encoder: encoder_api::VideoEncoderConfig::default(),
            follow_source_dimensions: false,
            scaling_plan: EngineScalingPlan::default(),
            audio_encoder: AudioEncoderConfig::default(),
            audio_sync: AudioSyncConfig::default(),
        }
    }
}

/// One independently captured and encoded audio source. The source is moved
/// to its worker at start; the capture and encoder are never shared between
/// workers.
pub struct AudioInput {
    pub stream_id: StreamId,
    pub label: String,
    pub capture_config: AudioCaptureConfig,
    /// Linear gain applied by the worker's synchronizer for this source.
    pub gain: f32,
    pub capture: Box<dyn AudioCapture>,
    pub encoder: Box<dyn AudioEncoder>,
}

impl AudioInput {
    pub fn new(
        stream_id: StreamId,
        label: impl Into<String>,
        capture: Box<dyn AudioCapture>,
        encoder: Box<dyn AudioEncoder>,
    ) -> Self {
        Self {
            stream_id,
            label: label.into(),
            capture_config: AudioCaptureConfig::default(),
            gain: 1.0,
            capture,
            encoder,
        }
    }

    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }
}

/// Events the engine emits while running; the controller maps these onto
/// UI-facing events. `BufferPrimed` drives Buffering → Ready (spec §9.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    StateChanged(RecorderState),
    BufferPrimed,
    RecoveryStarted { component: String },
    RecoveryCompleted { component: String },
    Warning { code: String, message: String },
    WorkerFailed { code: String, message: String },
}

#[derive(Clone)]
struct EngineEventPublisher {
    sender: SyncSender<EngineEvent>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
}

impl EngineEventPublisher {
    fn publish(&self, event: EngineEvent) {
        if let Err(error) = self.sender.try_send(event) {
            if matches!(error, TrySendError::Full(_)) {
                let count = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                if count == 1 || count.is_power_of_two() {
                    diagnostics::warn(
                        "engine",
                        &format!("engine event queue full; dropped {count} event(s)"),
                    );
                }
            }
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

type SharedBuffer = Arc<Mutex<ReplayBuffer>>;

#[derive(Clone)]
struct VideoWorkerShared {
    buffer: SharedBuffer,
    shutdown: Arc<AtomicBool>,
    encoder_metrics: Arc<Mutex<EncoderMetrics>>,
    scaling_status: Arc<Mutex<VideoScalingStatus>>,
    events: EngineEventPublisher,
}

struct AudioWorkerShared {
    buffer: SharedBuffer,
    shutdown: Arc<AtomicBool>,
    sync_metrics: Arc<Mutex<BTreeMap<StreamId, AudioSyncMetrics>>>,
    events: EngineEventPublisher,
}

struct AudioWorkerConfig {
    capture_config: AudioCaptureConfig,
    stream_id: StreamId,
    label: String,
    encoder_config: AudioEncoderConfig,
    sync_config: AudioSyncConfig,
}

/// Coordinates capture/encode workers and the muxer behind explicit state
/// transitions. Construct on the controller thread; `start` spawns the
/// video worker which owns the capture and encoder objects until stopped.
///
/// Buffer ownership (ADR 0002/0004): one `ReplayBuffer` behind a short
/// mutex shared by worker (insert) and controller (snapshot); no lock is
/// held across encode or file output.
pub struct RecorderEngine {
    config: EngineConfig,
    state: RecorderState,
    events_tx: EngineEventPublisher,
    events_rx: mpsc::Receiver<EngineEvent>,
    event_drops: Arc<std::sync::atomic::AtomicU64>,
    buffer: SharedBuffer,
    shutdown: Arc<AtomicBool>,
    encoder_metrics: Arc<Mutex<EncoderMetrics>>,
    scaling_status: Arc<Mutex<VideoScalingStatus>>,
    audio_sync_metrics: Arc<Mutex<BTreeMap<StreamId, AudioSyncMetrics>>>,
    worker: Option<JoinHandle<()>>,
    audio_workers: Vec<JoinHandle<()>>,
    pending_parts: Option<(Box<dyn VideoCapture>, Box<dyn VideoEncoder>)>,
    pending_audio: Vec<AudioInput>,
    muxer: Option<Box<dyn Muxer>>,
}

impl RecorderEngine {
    pub fn new(
        config: EngineConfig,
        video: Box<dyn VideoCapture>,
        video_encoder: Box<dyn VideoEncoder>,
        muxer: Box<dyn Muxer>,
    ) -> Self {
        Self::new_with_audio(config, video, video_encoder, Vec::new(), muxer)
    }

    pub fn new_with_audio(
        config: EngineConfig,
        video: Box<dyn VideoCapture>,
        video_encoder: Box<dyn VideoEncoder>,
        audio_inputs: Vec<AudioInput>,
        muxer: Box<dyn Muxer>,
    ) -> Self {
        let (event_sender, events_rx) = mpsc::sync_channel(ENGINE_EVENT_QUEUE_BOUND);
        let event_drops = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let events_tx = EngineEventPublisher {
            sender: event_sender,
            dropped: Arc::clone(&event_drops),
        };
        let buffer = Arc::new(Mutex::new(ReplayBuffer::new(BufferConfig {
            retention_ms: config.retention_ms,
            max_packets_per_stream: config.max_packets_per_stream,
        })));
        Self {
            config,
            state: RecorderState::Stopped,
            events_tx,
            events_rx,
            event_drops,
            buffer,
            shutdown: Arc::new(AtomicBool::new(false)),
            encoder_metrics: Arc::new(Mutex::new(EncoderMetrics::default())),
            scaling_status: Arc::new(Mutex::new(VideoScalingStatus::default())),
            audio_sync_metrics: Arc::new(Mutex::new(BTreeMap::new())),
            worker: None,
            audio_workers: Vec::new(),
            pending_parts: Some((video, video_encoder)),
            pending_audio: audio_inputs,
            muxer: Some(muxer),
        }
    }

    pub fn state(&self) -> RecorderState {
        self.state
    }

    /// Packets currently retained for replay purposes.
    pub fn buffered_packet_count(&self) -> usize {
        self.buffer
            .lock()
            .map(|b| b.metrics().packet_count)
            .unwrap_or(0)
    }

    /// Buffered video span in milliseconds.
    pub fn buffer_duration_ms(&self) -> i64 {
        self.buffer.lock().map(|b| b.duration_ticks()).unwrap_or(0)
    }

    /// Current replay-buffer epoch. Codec reinitialization/fallback starts a
    /// fresh epoch so incompatible packet configurations cannot mix.
    pub fn buffer_epoch(&self) -> u64 {
        self.buffer.lock().map(|buffer| buffer.epoch()).unwrap_or(0)
    }

    /// Latest bounded encoder telemetry snapshot published by the video
    /// worker. Reading it never waits on the encoder or media buffer.
    pub fn encoder_metrics(&self) -> EncoderMetrics {
        self.encoder_metrics
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or_default()
    }

    pub fn audio_sync_metrics(&self) -> BTreeMap<StreamId, AudioSyncMetrics> {
        self.audio_sync_metrics
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or_default()
    }

    /// Latest worker-written scaling resolution and fallback status.
    pub fn scaling_status(&self) -> VideoScalingStatus {
        self.scaling_status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }

    /// Number of engine events discarded because the bounded event bridge was
    /// full. Event delivery is best effort; state is also held directly and
    /// priming is checked from the buffer during `pump`.
    pub fn dropped_event_count(&self) -> u64 {
        self.event_drops.load(Ordering::Relaxed)
    }

    /// Apply a replay-window change without restarting capture.
    pub fn set_replay_retention_ms(&mut self, retention_ms: i64) -> Result<(), EngineError> {
        let mut buffer = self
            .buffer
            .lock()
            .map_err(|_| EngineError::MissingComponent)?;
        buffer.set_retention(retention_ms)?;
        self.config.retention_ms = retention_ms;
        Ok(())
    }

    /// Mark a deterministic resume boundary. Platform capture/audio resource
    /// recreation is backend-owned; this method provides the shared engine
    /// behavior that can be tested without hardware: old replay data is
    /// discarded, stream registrations are retained, and priming starts again.
    pub fn notify_resume(&mut self) -> Result<(), EngineError> {
        if !self.state.is_active() {
            return Err(EngineError::WrongState {
                current: self.state,
                expected: "an active state",
            });
        }
        self.transition_to(RecorderState::Recovering)?;
        let reset = self
            .buffer
            .lock()
            .map_err(|_| EngineError::MissingComponent)
            .and_then(|mut buffer| {
                buffer
                    .start_new_epoch_preserving_streams()
                    .map(|_| ())
                    .map_err(EngineError::from)
            });
        match reset {
            Ok(()) => {
                self.transition_to(RecorderState::Buffering)?;
                Ok(())
            }
            Err(error) => {
                let _ = self.transition_to(RecorderState::Error);
                Err(error)
            }
        }
    }

    fn pump_internal(&mut self) -> Vec<EngineEvent> {
        let mut external = Vec::new();
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                EngineEvent::BufferPrimed => {
                    // A queued priming event may belong to the previous
                    // buffer epoch, so validate the current buffer first.
                    self.promote_if_primed();
                }
                EngineEvent::WorkerFailed { code, message } => {
                    if self.state.is_active() && self.state != RecorderState::Error {
                        let _ = self.transition_to(RecorderState::Error);
                    }
                    external.push(EngineEvent::Warning { code, message });
                }
                EngineEvent::RecoveryStarted { .. } => {
                    if self.state.is_active() && self.state != RecorderState::Recovering {
                        let _ = self.transition_to(RecorderState::Recovering);
                    }
                }
                EngineEvent::RecoveryCompleted { .. } => {
                    if self.state == RecorderState::Recovering {
                        let _ = self.transition_to(RecorderState::Buffering);
                    }
                }
                other => external.push(other),
            }
        }
        external
    }

    fn promote_if_primed(&mut self) {
        if self.state != RecorderState::Buffering {
            return;
        }
        let primed = self
            .buffer
            .lock()
            .map(|buffer| buffer.is_primed())
            .unwrap_or(false);
        if primed {
            let _ = self.transition_to(RecorderState::Ready);
        }
    }

    /// Drain lifecycle transitions and warnings emitted since last call,
    /// processing internal ones (priming) into state changes first.
    pub fn pump(&mut self) -> Vec<EngineEvent> {
        let mut external = self.pump_internal();
        self.promote_if_primed();
        // Priming may have queued a StateChanged; surface it this call.
        external.extend(self.pump_internal());
        external
    }

    /// True when the worker exited on its own (source loss/end of stream).
    pub fn worker_finished(&self) -> bool {
        self.worker
            .as_ref()
            .map(|h| h.is_finished())
            .unwrap_or(true)
    }

    fn transition_to(&mut self, target: RecorderState) -> Result<(), EngineError> {
        validate_transition(self.state, target)?;
        self.state = target;
        self.events_tx.publish(EngineEvent::StateChanged(target));
        Ok(())
    }

    /// Start capture and encoding: Stopped → Starting → Buffering, then
    /// Buffering → Ready once the replay window is primed (spec §9.2).
    pub fn start(&mut self) -> Result<(), EngineError> {
        diagnostics::info("engine", "start requested");
        if self.config.follow_source_dimensions {
            let capture = self
                .pending_parts
                .as_ref()
                .map(|(capture, _)| &**capture)
                .ok_or(EngineError::AlreadyStarted)?;
            let sources = capture.enumerate_sources()?;
            let source = sources
                .iter()
                .find(|source| source.id == self.config.video_source_id)
                .or_else(|| {
                    matches!(
                        self.config.video_source_id.as_str(),
                        "" | "default" | "display-1" | "auto"
                    )
                    .then(|| {
                        sources
                            .iter()
                            .find(|source| source.is_primary)
                            .or_else(|| sources.first())
                    })
                    .flatten()
                })
                .ok_or(capture_api::VideoCaptureError::SourceUnavailable)?;
            self.config.video_encoder.width = source.width;
            self.config.video_encoder.height = source.height;
        }
        // Validate common settings before taking ownership of the injected
        // components so a rejected start remains retryable.
        self.config.video_encoder.validate()?;
        self.config.audio_encoder.validate()?;
        self.config.audio_sync.validate()?;
        let Some((mut capture, mut encoder)) = self.pending_parts.take() else {
            return Err(EngineError::AlreadyStarted);
        };

        // Backend-specific configuration runs on the encoding worker so
        // COM/MF objects never change threads.
        if let Err(err) = capture.start(capture_api::VideoCaptureConfig {
            source_id: self.config.video_source_id.clone(),
            target_fps: self.config.video_encoder.fps,
        }) {
            self.pending_parts = Some((capture, encoder));
            return Err(err.into());
        }

        self.transition_to(RecorderState::Starting)?;
        let audio_inputs = std::mem::take(&mut self.pending_audio);

        let worker_descriptor_result = {
            let codec = self.config.video_encoder.codec.clone();
            let descriptor = StreamDescriptor {
                stream_id: self.config.video_stream_id,
                media_type: MediaType::Video,
                time_base: TimeBase::MILLISECOND,
                name: Some(self.config.video_source_id.clone()),
                codec,
                extradata: None,
                width: Some(self.config.video_encoder.width),
                height: Some(self.config.video_encoder.height),
                sample_rate: None,
                channels: None,
                pixel_format: None,
            };
            let worker_descriptor = descriptor.clone();
            let mut guard = self
                .buffer
                .lock()
                .map_err(|_| EngineError::MissingComponent)?;
            guard.register_stream(descriptor)?;
            for input in &audio_inputs {
                if guard.has_stream(input.stream_id) {
                    return Err(EngineError::InvalidConfiguration {
                        details: format!(
                            "audio stream {} duplicates an already registered stream",
                            input.stream_id
                        ),
                    });
                }
                guard.register_stream(audio_stream_descriptor(
                    input.stream_id,
                    &input.label,
                    &self.config.audio_encoder,
                ))?;
            }
            Ok(worker_descriptor)
        };
        let worker_descriptor = match worker_descriptor_result {
            Ok(descriptor) => descriptor,
            Err(error) => {
                self.pending_parts = Some((capture, encoder));
                self.pending_audio = audio_inputs;
                return Err(error);
            }
        };

        self.shutdown.store(false, Ordering::SeqCst);

        let scaling_plan = self.config.scaling_plan;
        let worker_config = VideoWorkerConfig {
            encoder_config: self.config.video_encoder.clone(),
            scaling_plan,
            follow_source_dimensions: self.config.follow_source_dimensions,
            capture_config: capture_api::VideoCaptureConfig {
                source_id: self.config.video_source_id.clone(),
                target_fps: self.config.video_encoder.fps,
            },
            stream_descriptor: worker_descriptor,
        };
        let worker_shared = VideoWorkerShared {
            buffer: Arc::clone(&self.buffer),
            shutdown: Arc::clone(&self.shutdown),
            encoder_metrics: Arc::clone(&self.encoder_metrics),
            scaling_status: Arc::clone(&self.scaling_status),
            events: self.events_tx.clone(),
        };

        let builder = std::thread::Builder::new();
        self.worker = Some(
            builder
                .name("silk-video-worker".to_string())
                .spawn(move || {
                    run_video_worker(capture, &mut *encoder, worker_config, worker_shared);
                })
                .map_err(|_| EngineError::MissingComponent)?,
        );

        let audio_sync_config = self.config.audio_sync.clone();
        let audio_encoder_config = self.config.audio_encoder.clone();
        let mut audio_workers = Vec::with_capacity(audio_inputs.len());
        for input in audio_inputs {
            let AudioInput {
                stream_id,
                label,
                capture_config,
                gain,
                capture,
                encoder,
            } = input;
            let shared = AudioWorkerShared {
                buffer: Arc::clone(&self.buffer),
                shutdown: Arc::clone(&self.shutdown),
                sync_metrics: Arc::clone(&self.audio_sync_metrics),
                events: self.events_tx.clone(),
            };
            let mut sync_config = audio_sync_config.clone();
            sync_config.gain = gain;
            let encoder_config = audio_encoder_config.clone();
            let worker_name = format!("silk-audio-worker-{}", stream_id.0);
            let worker_config = AudioWorkerConfig {
                capture_config,
                stream_id,
                label,
                encoder_config,
                sync_config,
            };
            match std::thread::Builder::new()
                .name(worker_name)
                .spawn(move || {
                    run_audio_worker(capture, encoder, worker_config, shared);
                }) {
                Ok(worker) => audio_workers.push(worker),
                Err(error) => {
                    self.events_tx.publish(EngineEvent::Warning {
                        code: "AUDIO_WORKER_START_FAILED".to_string(),
                        message: format!(
                            "could not start audio worker for stream {stream_id}: {error}"
                        ),
                    });
                }
            }
        }
        self.audio_workers = audio_workers;

        self.state = RecorderState::Buffering;
        self.events_tx
            .publish(EngineEvent::StateChanged(RecorderState::Buffering));
        Ok(())
    }

    /// Stop capture cleanly. Legal from every active state.
    pub fn stop(&mut self) -> Result<(), EngineError> {
        diagnostics::info("engine", "stop requested");
        if !self.state.is_active() {
            return Err(EngineError::WrongState {
                current: self.state,
                expected: "an active state",
            });
        }
        self.transition_to(RecorderState::Stopping)?;
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        for worker in self.audio_workers.drain(..) {
            let _ = worker.join();
        }
        self.pending_parts = None;
        self.pending_audio.clear();
        self.muxer = None;
        self.transition_to(RecorderState::Stopped)?;
        Ok(())
    }

    /// Snapshot the live buffer for a save worker. Ingestion continues:
    /// snapshots take shared references only (BUF-007/BUF-009).
    ///
    /// Clip start follows BUF-006: newest keyframe at or before
    /// `newest − retention`; pre-roll variance is reported via diagnostics
    /// (BUF-013) and returned metadata carries the actual duration.
    pub fn snapshot_replay(&mut self) -> Result<MediaSnapshot, EngineError> {
        self.snapshot_replay_with_preroll().map(|(media, _)| media)
    }

    fn snapshot_replay_with_preroll(&mut self) -> Result<(MediaSnapshot, i64), EngineError> {
        if !self.state.can_save()
            && self.state != RecorderState::Error
            && self.state != RecorderState::Buffering
        {
            return Err(EngineError::WrongState {
                current: self.state,
                expected: "Ready, Degraded, Buffering, or Error with buffered replay",
            });
        }

        let (mut media, preroll_ms) = {
            let guard = self
                .buffer
                .lock()
                .map_err(|_| EngineError::MissingComponent)?;
            let newest = guard
                .newest_video_dts()
                .ok_or(EngineError::NothingBuffered)?;
            let oldest = guard.earliest_video_keyframe_pts().unwrap_or(0);
            let requested = (newest - guard.retention_ms()).max(oldest);
            let outcome = guard.snapshot(requested)?;
            (outcome.media, outcome.keyframe_preroll_ticks)
        };
        media.captured_at_unix_ms = now_unix_ms();
        normalize_timestamps(&mut media);

        Ok((media, preroll_ms))
    }

    /// Move the muxer to the save worker that owns container I/O.
    pub fn take_muxer(&mut self) -> Result<Box<dyn Muxer>, EngineError> {
        self.muxer.take().ok_or(EngineError::MissingComponent)
    }

    /// Synchronous compatibility helper for engine-level tests that inject a
    /// recording muxer directly. The desktop controller uses `snapshot_replay`
    /// and a bounded save worker instead.
    pub fn save_replay(&mut self, final_path: &Path) -> Result<ClipMetadata, EngineError> {
        let (media, preroll_ms) = self.snapshot_replay_with_preroll()?;

        let muxer = self.muxer.as_mut().ok_or(EngineError::MissingComponent)?;
        let options = SaveOptions::default();
        let metadata = muxer.write_snapshot(&media, final_path, &options)?;
        diagnostics::info(
            "engine",
            &format!(
                "clip published at {} (requested preroll {} ms, duration {} ms)",
                final_path.display(),
                preroll_ms,
                metadata.duration_ms
            ),
        );
        Ok(metadata)
    }
}

enum WorkerScalingMode {
    Preferred2xPending {
        base_config: encoder_api::VideoEncoderConfig,
    },
    ActiveSupersampled2x,
    ActiveNative1x,
}

fn update_registered_video_descriptor(
    buffer: &SharedBuffer,
    descriptor: &StreamDescriptor,
) -> Result<(), String> {
    let mut guard = buffer.lock().map_err(|e| e.to_string())?;
    guard
        .register_stream(descriptor.clone())
        .map_err(|e| e.to_string())
}

struct VideoWorkerConfig {
    encoder_config: encoder_api::VideoEncoderConfig,
    scaling_plan: EngineScalingPlan,
    follow_source_dimensions: bool,
    capture_config: capture_api::VideoCaptureConfig,
    stream_descriptor: StreamDescriptor,
}

fn resolve_video_scaling(
    capture: &dyn VideoCapture,
    encoder: &mut dyn VideoEncoder,
    base_config: &encoder_api::VideoEncoderConfig,
    scaling_plan: EngineScalingPlan,
    stream_descriptor: &mut StreamDescriptor,
    worker_shared: &VideoWorkerShared,
) -> Result<WorkerScalingMode, String> {
    let VideoWorkerShared {
        buffer,
        scaling_status,
        events,
        ..
    } = worker_shared;
    match scaling_plan {
        EngineScalingPlan::Native1x => {
            encoder
                .set_gpu_frame_context(capture.gpu_frame_context())
                .map_err(|error| {
                    format!("could not attach GPU context to video encoder: {error}")
                })?;
            encoder
                .configure(base_config.clone())
                .map_err(|error| format!("could not configure video encoder: {error}"))?;
            stream_descriptor.width = Some(base_config.width);
            stream_descriptor.height = Some(base_config.height);
            update_registered_video_descriptor(buffer, stream_descriptor)?;
            if let Ok(mut status) = scaling_status.lock() {
                *status = VideoScalingStatus {
                    state: VideoScalingState::ActiveNative1x,
                    active_dimensions: Some((base_config.width, base_config.height)),
                    fallback_reason: None,
                };
            }
            Ok(WorkerScalingMode::ActiveNative1x)
        }
        EngineScalingPlan::Prefer2xWithFallback => {
            let gpu_ctx = capture.gpu_frame_context();
            if gpu_ctx.is_none() {
                let reason = ScalingFallbackReason::GpuContextUnavailable;
                encoder
                    .set_gpu_frame_context(None)
                    .map_err(|error| format!("could not clear GPU context on encoder: {error}"))?;
                encoder.configure(base_config.clone()).map_err(|error| {
                    format!("could not configure baseline video encoder: {error}")
                })?;
                stream_descriptor.width = Some(base_config.width);
                stream_descriptor.height = Some(base_config.height);
                update_registered_video_descriptor(buffer, stream_descriptor)?;
                if let Ok(mut status) = scaling_status.lock() {
                    *status = VideoScalingStatus {
                        state: VideoScalingState::ActiveNative1x,
                        active_dimensions: Some((base_config.width, base_config.height)),
                        fallback_reason: Some(reason.clone()),
                    };
                }
                events.publish(EngineEvent::Warning {
                    code: reason.code().to_string(),
                    message: reason.message().to_string(),
                });
                return Ok(WorkerScalingMode::ActiveNative1x);
            }

            let preferred_w = base_config.width.checked_mul(2);
            let preferred_h = base_config.height.checked_mul(2);
            let mut preferred_config = base_config.clone();
            let valid_2x = match (preferred_w, preferred_h) {
                (Some(w), Some(h)) => {
                    preferred_config.width = w;
                    preferred_config.height = h;
                    preferred_config.validate().is_ok()
                }
                _ => false,
            };

            if !valid_2x {
                let reason = ScalingFallbackReason::DimensionsOverflow;
                encoder.set_gpu_frame_context(gpu_ctx).map_err(|error| {
                    format!("could not attach GPU context to video encoder: {error}")
                })?;
                encoder.configure(base_config.clone()).map_err(|error| {
                    format!("could not configure baseline video encoder: {error}")
                })?;
                stream_descriptor.width = Some(base_config.width);
                stream_descriptor.height = Some(base_config.height);
                update_registered_video_descriptor(buffer, stream_descriptor)?;
                if let Ok(mut status) = scaling_status.lock() {
                    *status = VideoScalingStatus {
                        state: VideoScalingState::ActiveNative1x,
                        active_dimensions: Some((base_config.width, base_config.height)),
                        fallback_reason: Some(reason.clone()),
                    };
                }
                events.publish(EngineEvent::Warning {
                    code: reason.code().to_string(),
                    message: reason.message().to_string(),
                });
                return Ok(WorkerScalingMode::ActiveNative1x);
            }

            encoder.set_gpu_frame_context(gpu_ctx).map_err(|error| {
                format!("could not attach GPU context to video encoder: {error}")
            })?;

            match encoder.configure(preferred_config.clone()) {
                Ok(()) => {
                    stream_descriptor.width = Some(preferred_config.width);
                    stream_descriptor.height = Some(preferred_config.height);
                    update_registered_video_descriptor(buffer, stream_descriptor)?;
                    if let Ok(mut status) = scaling_status.lock() {
                        *status = VideoScalingStatus {
                            state: VideoScalingState::Pending,
                            active_dimensions: Some((
                                preferred_config.width,
                                preferred_config.height,
                            )),
                            fallback_reason: None,
                        };
                    }
                    Ok(WorkerScalingMode::Preferred2xPending {
                        base_config: base_config.clone(),
                    })
                }
                Err(negotiate_err) => {
                    let reason = ScalingFallbackReason::EncoderNegotiationFailed {
                        details: Some(negotiate_err.to_string()),
                    };
                    encoder.configure(base_config.clone()).map_err(|error| {
                        format!("could not configure baseline video encoder: {error}")
                    })?;
                    stream_descriptor.width = Some(base_config.width);
                    stream_descriptor.height = Some(base_config.height);
                    update_registered_video_descriptor(buffer, stream_descriptor)?;
                    if let Ok(mut status) = scaling_status.lock() {
                        *status = VideoScalingStatus {
                            state: VideoScalingState::ActiveNative1x,
                            active_dimensions: Some((base_config.width, base_config.height)),
                            fallback_reason: Some(reason.clone()),
                        };
                    }
                    events.publish(EngineEvent::Warning {
                        code: reason.code().to_string(),
                        message: reason.message().to_string(),
                    });
                    Ok(WorkerScalingMode::ActiveNative1x)
                }
            }
        }
    }
}

fn run_video_worker(
    mut capture: Box<dyn VideoCapture>,
    encoder: &mut dyn VideoEncoder,
    worker_config: VideoWorkerConfig,
    worker_shared: VideoWorkerShared,
) {
    let VideoWorkerConfig {
        mut encoder_config,
        scaling_plan,
        follow_source_dimensions,
        capture_config,
        mut stream_descriptor,
    } = worker_config;
    let VideoWorkerShared {
        buffer,
        shutdown,
        encoder_metrics,
        scaling_status,
        events,
    } = worker_shared.clone();

    let mut scaling_mode = match resolve_video_scaling(
        &*capture,
        encoder,
        &encoder_config,
        scaling_plan,
        &mut stream_descriptor,
        &worker_shared,
    ) {
        Ok(mode) => mode,
        Err(err) => {
            if let Ok(mut status) = scaling_status.lock() {
                status.state = VideoScalingState::Failed;
            }
            events.publish(EngineEvent::WorkerFailed {
                code: "VIDEO_INITIALIZATION_FAILED".to_string(),
                message: err,
            });
            let _ = capture.stop();
            return;
        }
    };

    if let Some(message) = publish_video_extradata(&buffer, &mut stream_descriptor, encoder) {
        events.publish(EngineEvent::WorkerFailed {
            code: "VIDEO_METADATA_FAILURE".to_string(),
            message,
        });
        let _ = capture.stop();
        return;
    }
    publish_encoder_events(encoder, &events);

    let mut primed_reported = false;
    let mut encoder_epoch = encoder.epoch();
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        match capture.next_event() {
            Ok(VideoCaptureEvent::Frame(frame)) => {
                let encode_result = match scaling_mode {
                    WorkerScalingMode::Preferred2xPending { ref base_config } => {
                        let base_cfg = base_config.clone();
                        let frame_clone = frame.clone();
                        match encoder.encode(frame) {
                            Ok(packets) => {
                                scaling_mode = WorkerScalingMode::ActiveSupersampled2x;
                                if let Ok(mut status) = scaling_status.lock() {
                                    status.state = VideoScalingState::ActiveSupersampled2x;
                                }
                                Ok(packets)
                            }
                            Err(first_frame_err) => {
                                let reason = ScalingFallbackReason::StartupProbeFailed {
                                    details: Some(first_frame_err.to_string()),
                                };
                                if let Err(cfg_err) = encoder.configure(base_cfg.clone()) {
                                    if let Ok(mut status) = scaling_status.lock() {
                                        status.state = VideoScalingState::Failed;
                                        status.fallback_reason = Some(reason);
                                    }
                                    events.publish(EngineEvent::WorkerFailed {
                                        code: "VIDEO_BASELINE_RETRY_CONFIGURE_FAILED".to_string(),
                                        message: cfg_err.to_string(),
                                    });
                                    break;
                                }
                                stream_descriptor.width = Some(base_cfg.width);
                                stream_descriptor.height = Some(base_cfg.height);
                                if let Err(epoch_err) =
                                    reset_video_buffer_epoch(&buffer, &stream_descriptor)
                                {
                                    if let Ok(mut status) = scaling_status.lock() {
                                        status.state = VideoScalingState::Failed;
                                    }
                                    events.publish(EngineEvent::WorkerFailed {
                                        code: "BUFFER_EPOCH_RESET_FAILED".to_string(),
                                        message: epoch_err,
                                    });
                                    break;
                                }
                                encoder_epoch = encoder.epoch();
                                primed_reported = false;

                                if let Ok(mut status) = scaling_status.lock() {
                                    *status = VideoScalingStatus {
                                        state: VideoScalingState::ActiveNative1x,
                                        active_dimensions: Some((base_cfg.width, base_cfg.height)),
                                        fallback_reason: Some(reason.clone()),
                                    };
                                }
                                events.publish(EngineEvent::Warning {
                                    code: reason.code().to_string(),
                                    message: reason.message().to_string(),
                                });
                                scaling_mode = WorkerScalingMode::ActiveNative1x;

                                encoder.encode(frame_clone)
                            }
                        }
                    }
                    _ => encoder.encode(frame),
                };

                match encode_result {
                    Ok(packets) => {
                        if let Some(message) = sync_encoder_epoch(
                            &buffer,
                            &stream_descriptor,
                            encoder,
                            &mut encoder_epoch,
                            &mut primed_reported,
                        ) {
                            events.publish(EngineEvent::WorkerFailed {
                                code: "BUFFER_EPOCH_RESET_FAILED".to_string(),
                                message,
                            });
                            break;
                        }
                        if let Some(message) =
                            publish_video_extradata(&buffer, &mut stream_descriptor, encoder)
                        {
                            events.publish(EngineEvent::WorkerFailed {
                                code: "VIDEO_METADATA_FAILURE".to_string(),
                                message,
                            });
                            break;
                        }
                        let fatal = insert_packets(&buffer, packets);
                        if let Some(message) = fatal {
                            events.publish(EngineEvent::WorkerFailed {
                                code: "BUFFER_INSERT_FAILED".to_string(),
                                message,
                            });
                            break;
                        }
                        publish_encoder_events(encoder, &events);
                        publish_encoder_metrics(&encoder_metrics, encoder);
                        if !primed_reported {
                            let primed = buffer.lock().map(|b| b.is_primed()).unwrap_or(false);
                            if primed {
                                primed_reported = true;
                                events.publish(EngineEvent::BufferPrimed);
                            }
                        }
                    }
                    Err(err) => {
                        let _ = sync_encoder_epoch(
                            &buffer,
                            &stream_descriptor,
                            encoder,
                            &mut encoder_epoch,
                            &mut primed_reported,
                        );
                        publish_encoder_events(encoder, &events);
                        publish_encoder_metrics(&encoder_metrics, encoder);
                        diagnostics::error(
                            "engine",
                            &format!("video worker encode failed [{}]: {err}", err.code()),
                        );
                        events.publish(EngineEvent::WorkerFailed {
                            code: err.code().to_string(),
                            message: err.to_string(),
                        });
                        break;
                    }
                }
            }
            Ok(VideoCaptureEvent::FormatChanged { width, height }) => {
                events.publish(EngineEvent::RecoveryStarted {
                    component: "display-format".to_string(),
                });
                events.publish(EngineEvent::Warning {
                    code: "CAPTURE_FORMAT_CHANGED".to_string(),
                    message: format!(
                        "capture format changed to {width}x{height}; rebuilding video epoch"
                    ),
                });
                if follow_source_dimensions {
                    encoder_config.width = width;
                    encoder_config.height = height;
                    if let Err(error) = encoder_config.validate() {
                        events.publish(EngineEvent::WorkerFailed {
                            code: "VIDEO_FORMAT_CHANGE_FAILED".to_string(),
                            message: format!(
                                "capture format {width}x{height} is not a valid native encoder size: {error}"
                            ),
                        });
                        break;
                    }
                }
                if let Err(message) = drain_video_encoder_into_buffer(encoder, &buffer) {
                    events.publish(EngineEvent::Warning {
                        code: "VIDEO_DRAIN_FAILED".to_string(),
                        message,
                    });
                }
                match resolve_video_scaling(
                    &*capture,
                    encoder,
                    &encoder_config,
                    scaling_plan,
                    &mut stream_descriptor,
                    &worker_shared,
                ) {
                    Ok(mode) => {
                        scaling_mode = mode;
                        if let Err(err) = reset_video_buffer_epoch(&buffer, &stream_descriptor) {
                            events.publish(EngineEvent::WorkerFailed {
                                code: "BUFFER_EPOCH_RESET_FAILED".to_string(),
                                message: err,
                            });
                            break;
                        }
                        encoder_epoch = encoder.epoch();
                        primed_reported = false;
                    }
                    Err(err) => {
                        events.publish(EngineEvent::WorkerFailed {
                            code: "VIDEO_FORMAT_CHANGE_FAILED".to_string(),
                            message: err,
                        });
                        break;
                    }
                }
                if let Some(message) =
                    publish_video_extradata(&buffer, &mut stream_descriptor, encoder)
                {
                    publish_encoder_events(encoder, &events);
                    publish_encoder_metrics(&encoder_metrics, encoder);
                    events.publish(EngineEvent::WorkerFailed {
                        code: "VIDEO_METADATA_FAILURE".to_string(),
                        message,
                    });
                    break;
                }
                publish_encoder_events(encoder, &events);
                publish_encoder_metrics(&encoder_metrics, encoder);
                events.publish(EngineEvent::RecoveryCompleted {
                    component: "display-format".to_string(),
                });
            }
            Ok(VideoCaptureEvent::SourceLost) => {
                events.publish(EngineEvent::Warning {
                    code: "CAPTURE_DEVICE_LOST".to_string(),
                    message: "video source lost; attempting recovery".to_string(),
                });
                events.publish(EngineEvent::RecoveryStarted {
                    component: "display".to_string(),
                });
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                if let Err(message) = drain_video_encoder_into_buffer(encoder, &buffer) {
                    events.publish(EngineEvent::Warning {
                        code: "VIDEO_RECOVERY_DRAIN_FAILED".to_string(),
                        message,
                    });
                }
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                if !restart_video_capture(&mut *capture, &capture_config, &events) {
                    events.publish(EngineEvent::WorkerFailed {
                        code: "CAPTURE_RECOVERY_EXHAUSTED".to_string(),
                        message: "video source recovery exhausted".to_string(),
                    });
                    break;
                }
                match resolve_video_scaling(
                    &*capture,
                    encoder,
                    &encoder_config,
                    scaling_plan,
                    &mut stream_descriptor,
                    &worker_shared,
                ) {
                    Ok(mode) => {
                        scaling_mode = mode;
                        if let Err(err) = reset_video_buffer_epoch(&buffer, &stream_descriptor) {
                            events.publish(EngineEvent::WorkerFailed {
                                code: "BUFFER_EPOCH_RESET_FAILED".to_string(),
                                message: err,
                            });
                            break;
                        }
                        encoder_epoch = encoder.epoch();
                        primed_reported = false;
                    }
                    Err(err) => {
                        events.publish(EngineEvent::WorkerFailed {
                            code: "VIDEO_RECOVERY_RECONFIGURE_FAILED".to_string(),
                            message: err,
                        });
                        break;
                    }
                }
                if let Some(message) =
                    publish_video_extradata(&buffer, &mut stream_descriptor, encoder)
                {
                    publish_encoder_events(encoder, &events);
                    publish_encoder_metrics(&encoder_metrics, encoder);
                    events.publish(EngineEvent::WorkerFailed {
                        code: "VIDEO_METADATA_FAILURE".to_string(),
                        message,
                    });
                    break;
                }
                publish_encoder_events(encoder, &events);
                publish_encoder_metrics(&encoder_metrics, encoder);
                events.publish(EngineEvent::RecoveryCompleted {
                    component: "display".to_string(),
                });
            }
            Err(err) => {
                if err.code() != "CAPTURE_END_OF_STREAM" {
                    diagnostics::error(
                        "engine",
                        &format!("video worker capture event failed [{}]: {err}", err.code()),
                    );
                    events.publish(EngineEvent::WorkerFailed {
                        code: err.code().to_string(),
                        message: err.to_string(),
                    });
                } else {
                    diagnostics::info("engine", "capture stream ended");
                }
                break;
            }
        }
    }

    match encoder.drain() {
        Ok(packets) => {
            let metadata_error = publish_video_extradata(&buffer, &mut stream_descriptor, encoder);
            if let Some(message) = metadata_error {
                events.publish(EngineEvent::WorkerFailed {
                    code: "VIDEO_METADATA_FAILURE".to_string(),
                    message,
                });
            } else if let Some(message) = insert_packets(&buffer, packets) {
                events.publish(EngineEvent::WorkerFailed {
                    code: "BUFFER_INSERT_FAILED".to_string(),
                    message,
                });
            } else if !primed_reported {
                let primed = buffer.lock().map(|b| b.is_primed()).unwrap_or(false);
                if primed {
                    events.publish(EngineEvent::BufferPrimed);
                }
            }
        }
        Err(err) => {
            eprintln!("worker drain error={err}");
            events.publish(EngineEvent::WorkerFailed {
                code: err.code().to_string(),
                message: err.to_string(),
            });
        }
    }
    publish_encoder_events(encoder, &events);
    publish_encoder_metrics(&encoder_metrics, encoder);
    log_encoder_metrics(encoder);
    let _ = capture.stop();
}

fn restart_video_capture(
    capture: &mut dyn VideoCapture,
    config: &capture_api::VideoCaptureConfig,
    events: &EngineEventPublisher,
) -> bool {
    let _ = capture.stop();
    for attempt in 1..=VIDEO_RESTART_LIMIT {
        std::thread::sleep(Duration::from_millis(50 * u64::from(attempt)));
        match capture.start(config.clone()) {
            Ok(()) => {
                diagnostics::info(
                    "capture",
                    &format!("display capture recovered on attempt {attempt}"),
                );
                return true;
            }
            Err(error) => {
                let exhausted = attempt == VIDEO_RESTART_LIMIT;
                diagnostics::warn(
                    "capture",
                    &format!(
                        "display capture recovery attempt {attempt}/{VIDEO_RESTART_LIMIT} failed: {error}"
                    ),
                );
                events.publish(EngineEvent::Warning {
                    code: if exhausted {
                        "CAPTURE_RECOVERY_EXHAUSTED".to_string()
                    } else {
                        error.code().to_string()
                    },
                    message: if exhausted {
                        format!(
                            "display capture recovery exhausted after {attempt} attempts: {error}"
                        )
                    } else {
                        format!("display capture recovery attempt {attempt} failed: {error}")
                    },
                });
                let _ = capture.stop();
            }
        }
    }
    false
}

fn drain_video_encoder_into_buffer(
    encoder: &mut dyn VideoEncoder,
    buffer: &SharedBuffer,
) -> Result<(), String> {
    let packets = encoder
        .drain()
        .map_err(|error| format!("could not drain video encoder: {error}"))?;
    if let Some(message) = insert_packets(buffer, packets) {
        return Err(format!("could not insert drained video packets: {message}"));
    }
    Ok(())
}

fn reset_video_buffer_epoch(
    buffer: &SharedBuffer,
    descriptor: &StreamDescriptor,
) -> Result<(), String> {
    match buffer.lock() {
        Ok(mut guard) => guard
            .start_new_epoch_preserving_streams()
            .map_err(|error| error.to_string())
            .and_then(|_| {
                guard
                    .register_stream(descriptor.clone())
                    .map_err(|error| error.to_string())
            }),
        Err(_) => Err("buffer mutex poisoned while starting a new epoch".to_string()),
    }
}

fn audio_stream_descriptor(
    stream_id: StreamId,
    name: &str,
    config: &AudioEncoderConfig,
) -> StreamDescriptor {
    StreamDescriptor {
        stream_id,
        media_type: MediaType::Audio,
        time_base: TimeBase::MILLISECOND,
        name: Some(name.to_string()),
        codec: "aac".to_string(),
        extradata: None,
        width: None,
        height: None,
        sample_rate: Some(config.sample_rate),
        channels: Some(config.channels),
        pixel_format: None,
    }
}

fn run_audio_worker(
    mut capture: Box<dyn AudioCapture>,
    mut encoder: Box<dyn AudioEncoder>,
    worker_config: AudioWorkerConfig,
    worker_shared: AudioWorkerShared,
) {
    let AudioWorkerConfig {
        capture_config,
        stream_id,
        label,
        encoder_config,
        sync_config,
    } = worker_config;
    let AudioWorkerShared {
        buffer,
        shutdown,
        sync_metrics,
        events,
    } = worker_shared;

    if let Err(error) = capture.start(capture_config.clone()) {
        events.publish(EngineEvent::Warning {
            code: error.code().to_string(),
            message: format!("{label} capture could not start: {error}"),
        });
        return;
    }

    let mut synchronizer = match AudioSynchronizer::new(sync_config) {
        Ok(synchronizer) => synchronizer,
        Err(error) => {
            events.publish(EngineEvent::Warning {
                code: "AUDIO_SYNC_FAILURE".to_string(),
                message: format!("{label} synchronizer could not start: {error}"),
            });
            let _ = capture.stop();
            return;
        }
    };
    if let Err(error) = encoder.configure(encoder_config) {
        events.publish(EngineEvent::Warning {
            code: error.code().to_string(),
            message: format!("{label} audio encoder could not start: {error}"),
        });
        let _ = capture.stop();
        return;
    }
    if let Some(extradata) = encoder.codec_extradata() {
        let metadata_result = buffer
            .lock()
            .map_err(|_| "audio stream metadata buffer mutex poisoned".to_string())
            .and_then(|mut guard| {
                guard
                    .set_stream_extradata(stream_id, Some(extradata))
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = metadata_result {
            events.publish(EngineEvent::Warning {
                code: "AUDIO_METADATA_FAILURE".to_string(),
                message: format!("{label} audio stream metadata could not be published: {error}"),
            });
            let _ = capture.stop();
            return;
        }
    }

    let mut restart_attempts = 0_u32;
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        match capture.next_event() {
            Ok(AudioCaptureEvent::Frames(mut frame)) => {
                frame.stream_id = stream_id;
                match synchronizer.process(frame) {
                    Ok(frame) => match encoder.encode(frame) {
                        Ok(packets) => {
                            if let Some(message) = insert_packets(&buffer, packets) {
                                events.publish(EngineEvent::Warning {
                                    code: "BUFFER_INSERT_FAILED".to_string(),
                                    message: format!("{label} audio: {message}"),
                                });
                                break;
                            }
                            restart_attempts = 0;
                            publish_audio_metrics(&sync_metrics, stream_id, &synchronizer);
                        }
                        Err(error) => {
                            events.publish(EngineEvent::Warning {
                                code: error.code().to_string(),
                                message: format!("{label} audio encode failed: {error}"),
                            });
                            break;
                        }
                    },
                    Err(error) => {
                        events.publish(EngineEvent::Warning {
                            code: "AUDIO_SYNC_FAILURE".to_string(),
                            message: format!("{label} audio synchronization failed: {error}"),
                        });
                        synchronizer.reset_source();
                        continue;
                    }
                }
            }
            Ok(AudioCaptureEvent::DeviceLost) => {
                events.publish(EngineEvent::Warning {
                    code: "AUDIO_DEVICE_LOST".to_string(),
                    message: format!("{label} audio device was lost; attempting recovery"),
                });
                if shutdown.load(Ordering::SeqCst)
                    || !restart_audio_capture(
                        &mut *capture,
                        &capture_config,
                        &label,
                        &mut restart_attempts,
                        &events,
                    )
                {
                    break;
                }
                synchronizer.reset_source();
            }
            Err(AudioCaptureError::Timeout) => {}
            Err(AudioCaptureError::EndOfStream) => {
                diagnostics::info("audio", &format!("{label} capture stream ended"));
                break;
            }
            Err(AudioCaptureError::DeviceChanged) => {
                events.publish(EngineEvent::Warning {
                    code: "AUDIO_DEVICE_CHANGED".to_string(),
                    message: format!("{label} audio device changed; attempting recovery"),
                });
                if shutdown.load(Ordering::SeqCst)
                    || !restart_audio_capture(
                        &mut *capture,
                        &capture_config,
                        &label,
                        &mut restart_attempts,
                        &events,
                    )
                {
                    break;
                }
                synchronizer.reset_source();
            }
            Err(error) => {
                events.publish(EngineEvent::Warning {
                    code: error.code().to_string(),
                    message: format!("{label} audio capture failed: {error}"),
                });
                break;
            }
        }
    }

    if let Err(error) = encoder.drain().and_then(|packets| {
        insert_packets(&buffer, packets).map_or(Ok(()), |message| {
            Err(encoder_api::EncoderError::EncodeFailed { details: message })
        })
    }) {
        events.publish(EngineEvent::Warning {
            code: error.code().to_string(),
            message: format!("{label} audio drain failed: {error}"),
        });
    }
    publish_audio_metrics(&sync_metrics, stream_id, &synchronizer);
    let _ = capture.stop();
}

fn restart_audio_capture(
    capture: &mut dyn AudioCapture,
    config: &AudioCaptureConfig,
    label: &str,
    attempts: &mut u32,
    events: &EngineEventPublisher,
) -> bool {
    let _ = capture.stop();
    while *attempts < AUDIO_RESTART_LIMIT {
        *attempts += 1;
        std::thread::sleep(Duration::from_millis(25 * u64::from(*attempts)));
        match capture.start(config.clone()) {
            Ok(()) => {
                diagnostics::info(
                    "audio",
                    &format!("{label} audio capture recovered on attempt {attempts}"),
                );
                return true;
            }
            Err(error) => {
                let exhausted = *attempts >= AUDIO_RESTART_LIMIT;
                events.publish(EngineEvent::Warning {
                    code: if exhausted {
                        "AUDIO_RECOVERY_EXHAUSTED".to_string()
                    } else {
                        error.code().to_string()
                    },
                    message: if exhausted {
                        format!(
                            "{label} audio recovery exhausted after {attempts} attempts: {error}"
                        )
                    } else {
                        format!("{label} audio recovery attempt {attempts} failed: {error}")
                    },
                });
                let _ = capture.stop();
            }
        }
    }
    false
}

fn publish_audio_metrics(
    metrics: &Arc<Mutex<BTreeMap<StreamId, AudioSyncMetrics>>>,
    stream_id: StreamId,
    synchronizer: &AudioSynchronizer,
) {
    if let Ok(mut target) = metrics.lock() {
        target.insert(stream_id, synchronizer.metrics());
    }
}

fn insert_packets(
    buffer: &SharedBuffer,
    packets: Vec<media_types::EncodedPacket>,
) -> Option<String> {
    for packet in packets {
        // Insert takes the buffer lock only briefly; the buffer assigns the
        // authoritative sequence.
        let result = match buffer.lock() {
            Ok(mut guard) => guard.insert(packet).map_err(|err| err.to_string()),
            Err(_) => Err("buffer mutex poisoned".to_string()),
        };
        if let Err(message) = result {
            return Some(message);
        }
    }
    None
}

fn sync_encoder_epoch(
    buffer: &SharedBuffer,
    descriptor: &StreamDescriptor,
    encoder: &dyn VideoEncoder,
    encoder_epoch: &mut u64,
    primed_reported: &mut bool,
) -> Option<String> {
    let current_epoch = encoder.epoch();
    if current_epoch == *encoder_epoch {
        return None;
    }
    let result = reset_video_buffer_epoch(buffer, descriptor);
    if result.is_ok() {
        *encoder_epoch = current_epoch;
        *primed_reported = false;
        None
    } else {
        result.err()
    }
}

fn publish_video_extradata(
    buffer: &SharedBuffer,
    descriptor: &mut StreamDescriptor,
    encoder: &dyn VideoEncoder,
) -> Option<String> {
    let extradata = encoder.codec_extradata()?;
    if descriptor.extradata.as_ref() == Some(&extradata) {
        return None;
    }
    let result = match buffer.lock() {
        Ok(mut guard) => guard
            .set_stream_extradata(descriptor.stream_id, Some(extradata.clone()))
            .map_err(|err| err.to_string()),
        Err(_) => Err("buffer mutex poisoned while publishing video metadata".to_string()),
    };
    match result {
        Ok(()) => {
            descriptor.extradata = Some(extradata);
            None
        }
        Err(message) => Some(message),
    }
}

fn publish_encoder_events(encoder: &mut dyn VideoEncoder, events: &EngineEventPublisher) {
    for event in encoder.take_events() {
        match event {
            EncoderEvent::BackendSelected {
                backend,
                hardware_accelerated,
            } => diagnostics::info(
                "encoder",
                &format!(
                    "backend selected: {backend} (hardware_accelerated={hardware_accelerated})"
                ),
            ),
            EncoderEvent::Fallback {
                from_backend,
                to_backend,
                reason,
                epoch,
            } => {
                events.publish(EngineEvent::Warning {
                    code: "ENCODER_FALLBACK".to_string(),
                    message: format!(
                        "encoder changed from {from_backend} to {to_backend} at epoch {epoch}: {reason}"
                    ),
                });
            }
        }
    }
}

fn publish_encoder_metrics(metrics: &Arc<Mutex<EncoderMetrics>>, encoder: &dyn VideoEncoder) {
    if let Ok(mut target) = metrics.lock() {
        *target = encoder.metrics();
    }
}

fn log_encoder_metrics(encoder: &dyn VideoEncoder) {
    let snapshot = encoder.metrics();
    diagnostics::info(
        "encoder",
        &format!(
            "session metrics: submitted={}, encoded={}, dropped={}, failures={}, overloads={}, fallback_count={}, epoch={}",
            snapshot.frames_submitted,
            snapshot.frames_encoded,
            snapshot.frames_dropped,
            snapshot.encode_failures,
            snapshot.overload_events,
            snapshot.fallback_count,
            snapshot.epoch,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::time::Instant;

    use capture_api::{VideoCaptureConfig, VideoCaptureError};
    use encoder_api::{EncoderCapabilities, VideoEncoderConfig};
    use media_types::{
        EncodedPacket, FramePayload, PacketPayload, PixelFormat, VideoFrame, VideoSourceInfo,
    };
    #[derive(Default)]
    struct EngineTestMuxer;

    impl Muxer for EngineTestMuxer {
        fn write_snapshot(
            &mut self,
            snapshot: &MediaSnapshot,
            final_path: &Path,
            _options: &SaveOptions,
        ) -> muxer::Result<ClipMetadata> {
            Ok(ClipMetadata {
                path: final_path.to_path_buf(),
                duration_ms: 0,
                size_bytes: 0,
                created_at_unix_ms: snapshot.captured_at_unix_ms,
                video_codec: snapshot
                    .streams
                    .iter()
                    .find(|s| s.descriptor.media_type == MediaType::Video)
                    .map(|s| s.descriptor.codec.clone()),
                audio_codecs: Vec::new(),
            })
        }
    }

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

    struct MockAv1EncoderWithAvcCapabilities {
        configured_codec: Option<String>,
        epoch: u64,
        extradata: Option<PacketPayload>,
    }

    impl MockAv1EncoderWithAvcCapabilities {
        fn new() -> Self {
            Self {
                configured_codec: None,
                epoch: 0,
                extradata: None,
            }
        }
    }

    impl VideoEncoder for MockAv1EncoderWithAvcCapabilities {
        fn capabilities(&self) -> EncoderCapabilities {
            // Placeholder capabilities advertising AVC prior to configuration,
            // reproducing FallbackVideoEncoder unconfigured state.
            EncoderCapabilities {
                backend_name: "unconfigured-placeholder".to_string(),
                codec: "avc".to_string(),
                hardware_accelerated: false,
                max_width: 0,
                max_height: 0,
                supported_fps: Vec::new(),
                supported_pixel_formats: vec![PixelFormat::Nv12],
            }
        }

        fn configure(&mut self, config: VideoEncoderConfig) -> encoder_api::Result<()> {
            self.configured_codec = Some(config.codec);
            Ok(())
        }

        fn encode(&mut self, frame: VideoFrame) -> encoder_api::Result<Vec<EncodedPacket>> {
            // Outputs AV1 raw OBU payload with no H.264 SPS/PPS NAL headers
            Ok(vec![EncodedPacket {
                stream_id: frame.stream_id,
                media_type: MediaType::Video,
                pts: frame.pts,
                dts: frame.pts,
                duration: 33,
                time_base: frame.time_base,
                is_keyframe: true,
                sequence: 0,
                payload: PacketPayload::from(vec![0x12, 0x00, 0x0a, 0x0b, 0x0c]),
            }])
        }

        fn drain(&mut self) -> encoder_api::Result<Vec<EncodedPacket>> {
            Ok(Vec::new())
        }

        fn codec_extradata(&self) -> Option<PacketPayload> {
            self.extradata.clone()
        }

        fn epoch(&self) -> u64 {
            self.epoch
        }
    }

    #[test]
    fn av1_stream_descriptor_uses_configured_codec_and_snapshots_without_sps_pps() {
        let mut engine = RecorderEngine::new(
            EngineConfig {
                retention_ms: 1,
                video_encoder: VideoEncoderConfig {
                    codec: "av1".to_string(),
                    width: 1_920,
                    height: 1_080,
                    fps: 60,
                    ..VideoEncoderConfig::default()
                },
                ..EngineConfig::default()
            },
            Box::new(TwoFrameCapture::new()),
            Box::new(MockAv1EncoderWithAvcCapabilities::new()),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine with AV1 config");

        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        engine.pump();

        assert_eq!(engine.state(), RecorderState::Ready);
        assert_eq!(engine.buffered_packet_count(), 2);

        let snapshot = engine.snapshot_replay().expect("replay snapshot");
        let video_stream = snapshot
            .streams
            .iter()
            .find(|s| s.descriptor.media_type == MediaType::Video)
            .expect("video stream in snapshot");
        assert_eq!(video_stream.descriptor.codec, "av1");

        engine.stop().expect("stop engine");
    }

    #[test]
    fn av1_descriptor_preserves_codec_across_extradata_and_epoch_sync() {
        struct EpochAdvancingAv1Encoder {
            calls: usize,
            epoch: u64,
            extradata: Option<PacketPayload>,
        }

        impl VideoEncoder for EpochAdvancingAv1Encoder {
            fn capabilities(&self) -> EncoderCapabilities {
                EncoderCapabilities {
                    backend_name: "unconfigured-placeholder".to_string(),
                    codec: "avc".to_string(),
                    hardware_accelerated: false,
                    max_width: 0,
                    max_height: 0,
                    supported_fps: Vec::new(),
                    supported_pixel_formats: vec![PixelFormat::Nv12],
                }
            }

            fn configure(&mut self, _config: VideoEncoderConfig) -> encoder_api::Result<()> {
                Ok(())
            }

            fn encode(&mut self, frame: VideoFrame) -> encoder_api::Result<Vec<EncodedPacket>> {
                self.calls += 1;
                if self.calls == 1 {
                    self.extradata = Some(PacketPayload::from(vec![0x81, 0x00, 0x0c, 0x00]));
                } else if self.calls == 2 {
                    self.epoch = 1;
                }
                Ok(vec![EncodedPacket {
                    stream_id: frame.stream_id,
                    media_type: MediaType::Video,
                    pts: frame.pts,
                    dts: frame.pts,
                    duration: 33,
                    time_base: frame.time_base,
                    is_keyframe: true,
                    sequence: 0,
                    payload: PacketPayload::from(vec![0x12, 0x00, 0x0a, 0x0b, 0x0c]),
                }])
            }

            fn drain(&mut self) -> encoder_api::Result<Vec<EncodedPacket>> {
                Ok(Vec::new())
            }

            fn codec_extradata(&self) -> Option<PacketPayload> {
                self.extradata.clone()
            }

            fn epoch(&self) -> u64 {
                self.epoch
            }
        }

        let mut engine = RecorderEngine::new(
            EngineConfig {
                retention_ms: 1,
                video_encoder: VideoEncoderConfig {
                    codec: "av1".to_string(),
                    width: 1_920,
                    height: 1_080,
                    fps: 60,
                    ..VideoEncoderConfig::default()
                },
                ..EngineConfig::default()
            },
            Box::new(TwoFrameCapture::with_frame_count(3)),
            Box::new(EpochAdvancingAv1Encoder {
                calls: 0,
                epoch: 0,
                extradata: None,
            }),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine with AV1 config");

        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        engine.pump();

        let snapshot = engine.snapshot_replay().expect("replay snapshot");
        let video_stream = snapshot
            .streams
            .iter()
            .find(|s| s.descriptor.media_type == MediaType::Video)
            .expect("video stream in snapshot");
        assert_eq!(video_stream.descriptor.codec, "av1");
        assert_eq!(
            video_stream
                .descriptor
                .extradata
                .as_ref()
                .map(|p| p.as_slice()),
            Some(&[0x81, 0x00, 0x0c, 0x00][..])
        );

        engine.stop().expect("stop engine");
    }

    struct MockGpuResolver;
    impl encoder_api::GpuFrameResolver for MockGpuResolver {
        fn resolve(
            &self,
            _handle: media_types::GpuFrameHandle,
        ) -> std::result::Result<
            Arc<encoder_api::GpuFrameResource>,
            encoder_api::GpuFrameResolveError,
        > {
            Ok(Arc::new(()))
        }
    }

    struct ScalingMockCapture {
        frames: VecDeque<VideoCaptureEvent>,
        started: bool,
        has_gpu: bool,
    }

    impl ScalingMockCapture {
        fn with_gpu(frames: Vec<VideoCaptureEvent>) -> Self {
            Self {
                frames: frames.into(),
                started: false,
                has_gpu: true,
            }
        }

        fn without_gpu(frames: Vec<VideoCaptureEvent>) -> Self {
            Self {
                frames: frames.into(),
                started: false,
                has_gpu: false,
            }
        }
    }

    impl VideoCapture for ScalingMockCapture {
        fn enumerate_sources(&self) -> capture_api::Result<Vec<VideoSourceInfo>> {
            Ok(vec![VideoSourceInfo {
                id: "display-1".to_string(),
                name: "Mock Display".to_string(),
                is_primary: true,
                width: 1920,
                height: 1080,
            }])
        }

        fn start(&mut self, _config: VideoCaptureConfig) -> capture_api::Result<()> {
            self.started = true;
            Ok(())
        }

        fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
            if !self.started {
                return Err(VideoCaptureError::Backend {
                    details: "not started".to_string(),
                });
            }
            self.frames
                .pop_front()
                .ok_or(VideoCaptureError::EndOfStream)
        }

        fn stop(&mut self) -> capture_api::Result<()> {
            self.started = false;
            Ok(())
        }

        fn gpu_frame_context(&self) -> Option<encoder_api::GpuFrameContext> {
            if self.has_gpu {
                Some(encoder_api::GpuFrameContext::new(Arc::new(MockGpuResolver)))
            } else {
                None
            }
        }
    }

    struct ScalingMockEncoder {
        configured_configs: Arc<Mutex<Vec<VideoEncoderConfig>>>,
        fail_if_dimensions: Option<(u32, u32)>,
        fail_on_first_2x_encode: bool,
        fail_on_first_1x_encode: bool,
        fail_on_later_encode: bool,
        encode_calls: usize,
        current_config: Option<VideoEncoderConfig>,
    }

    impl ScalingMockEncoder {
        fn new() -> Self {
            Self {
                configured_configs: Arc::new(Mutex::new(Vec::new())),
                fail_if_dimensions: None,
                fail_on_first_2x_encode: false,
                fail_on_first_1x_encode: false,
                fail_on_later_encode: false,
                encode_calls: 0,
                current_config: None,
            }
        }
    }

    impl VideoEncoder for ScalingMockEncoder {
        fn capabilities(&self) -> EncoderCapabilities {
            EncoderCapabilities {
                backend_name: "scaling-mock".to_string(),
                codec: "avc".to_string(),
                hardware_accelerated: true,
                max_width: 7680,
                max_height: 4320,
                supported_fps: vec![30, 60, 120],
                supported_pixel_formats: vec![PixelFormat::Nv12],
            }
        }

        fn configure(&mut self, config: VideoEncoderConfig) -> encoder_api::Result<()> {
            if let Some((w, h)) = self.fail_if_dimensions {
                if config.width == w && config.height == h {
                    return Err(encoder_api::EncoderError::UnsupportedConfiguration {
                        reason: format!("dimensions {w}x{h} rejected by test"),
                    });
                }
            }
            self.configured_configs.lock().unwrap().push(config.clone());
            self.current_config = Some(config);
            Ok(())
        }

        fn encode(&mut self, frame: VideoFrame) -> encoder_api::Result<Vec<EncodedPacket>> {
            self.encode_calls += 1;
            let current_w = self.current_config.as_ref().map(|c| c.width).unwrap_or(0);
            if current_w > 1920 && self.fail_on_first_2x_encode && self.encode_calls == 1 {
                return Err(encoder_api::EncoderError::EncodeFailed {
                    details: "mock 2x first frame failure".to_string(),
                });
            }
            if current_w <= 1920 && self.fail_on_first_1x_encode {
                return Err(encoder_api::EncoderError::EncodeFailed {
                    details: "mock 1x first frame failure".to_string(),
                });
            }
            if self.fail_on_later_encode && self.encode_calls >= 2 {
                return Err(encoder_api::EncoderError::EncodeFailed {
                    details: "mock later frame failure".to_string(),
                });
            }
            Ok(vec![EncodedPacket {
                stream_id: frame.stream_id,
                media_type: MediaType::Video,
                pts: frame.pts,
                dts: frame.pts,
                duration: 33,
                time_base: frame.time_base,
                is_keyframe: true,
                sequence: self.encode_calls as u64,
                payload: PacketPayload::from(vec![0x00, 0x00, 0x00, 0x01, 0x65]),
            }])
        }

        fn drain(&mut self) -> encoder_api::Result<Vec<EncodedPacket>> {
            Ok(Vec::new())
        }
    }

    fn test_video_frame(pts: i64) -> VideoFrame {
        VideoFrame {
            stream_id: StreamId(0),
            width: 1920,
            height: 1080,
            pixel_format: PixelFormat::Nv12,
            pts,
            time_base: TimeBase::MILLISECOND,
            payload: FramePayload::Cpu(Arc::from(&[0_u8][..])),
        }
    }

    #[test]
    fn scaling_standard_unchanged_native_1x() {
        let capture = ScalingMockCapture::with_gpu(vec![
            VideoCaptureEvent::Frame(test_video_frame(0)),
            VideoCaptureEvent::Frame(test_video_frame(33)),
        ]);
        let encoder = ScalingMockEncoder::new();
        let configs = Arc::clone(&encoder.configured_configs);

        let mut engine = RecorderEngine::new(
            EngineConfig {
                retention_ms: 1,
                scaling_plan: EngineScalingPlan::Native1x,
                ..EngineConfig::default()
            },
            Box::new(capture),
            Box::new(encoder),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        engine.pump();

        let status = engine.scaling_status();
        assert_eq!(status.state, VideoScalingState::ActiveNative1x);
        assert_eq!(status.active_dimensions, Some((1920, 1080)));
        assert_eq!(status.fallback_reason, None);

        let recorded_configs = configs.lock().unwrap().clone();
        assert_eq!(recorded_configs.len(), 1);
        assert_eq!(recorded_configs[0].width, 1920);
        assert_eq!(recorded_configs[0].height, 1080);

        let snapshot = engine.snapshot_replay().expect("snapshot");
        let vid = snapshot
            .streams
            .iter()
            .find(|s| s.descriptor.media_type == MediaType::Video)
            .unwrap();
        assert_eq!(vid.descriptor.width, Some(1920));
        assert_eq!(vid.descriptor.height, Some(1080));

        engine.stop().expect("stop engine");
    }

    #[test]
    fn scaling_clarity_pending_then_active_2x() {
        let capture = ScalingMockCapture::with_gpu(vec![
            VideoCaptureEvent::Frame(test_video_frame(0)),
            VideoCaptureEvent::Frame(test_video_frame(33)),
        ]);
        let encoder = ScalingMockEncoder::new();
        let configs = Arc::clone(&encoder.configured_configs);

        let mut engine = RecorderEngine::new(
            EngineConfig {
                retention_ms: 1,
                scaling_plan: EngineScalingPlan::Prefer2xWithFallback,
                ..EngineConfig::default()
            },
            Box::new(capture),
            Box::new(encoder),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine");

        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        engine.pump();

        let status = engine.scaling_status();
        assert_eq!(status.state, VideoScalingState::ActiveSupersampled2x);
        assert_eq!(status.active_dimensions, Some((3840, 2160)));
        assert_eq!(status.fallback_reason, None);

        let recorded_configs = configs.lock().unwrap().clone();
        assert_eq!(recorded_configs.len(), 1);
        assert_eq!(recorded_configs[0].width, 3840);
        assert_eq!(recorded_configs[0].height, 2160);

        let snapshot = engine.snapshot_replay().expect("snapshot");
        let vid = snapshot
            .streams
            .iter()
            .find(|s| s.descriptor.media_type == MediaType::Video)
            .unwrap();
        assert_eq!(vid.descriptor.width, Some(3840));
        assert_eq!(vid.descriptor.height, Some(2160));

        engine.stop().expect("stop engine");
    }

    #[test]
    fn scaling_clarity_absent_gpu_context_fallback() {
        let capture = ScalingMockCapture::without_gpu(vec![
            VideoCaptureEvent::Frame(test_video_frame(0)),
            VideoCaptureEvent::Frame(test_video_frame(33)),
        ]);
        let encoder = ScalingMockEncoder::new();
        let configs = Arc::clone(&encoder.configured_configs);

        let mut engine = RecorderEngine::new(
            EngineConfig {
                retention_ms: 1,
                scaling_plan: EngineScalingPlan::Prefer2xWithFallback,
                ..EngineConfig::default()
            },
            Box::new(capture),
            Box::new(encoder),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let events = engine.pump();

        assert!(events.iter().any(|e| matches!(
            e,
            EngineEvent::Warning { code, .. } if code == CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE
        )));

        let status = engine.scaling_status();
        assert_eq!(status.state, VideoScalingState::ActiveNative1x);
        assert_eq!(status.active_dimensions, Some((1920, 1080)));
        assert_eq!(
            status.fallback_reason,
            Some(ScalingFallbackReason::GpuContextUnavailable)
        );

        let recorded_configs = configs.lock().unwrap().clone();
        assert_eq!(recorded_configs.len(), 1);
        assert_eq!(recorded_configs[0].width, 1920);

        let snapshot = engine.snapshot_replay().expect("snapshot");
        let vid = snapshot
            .streams
            .iter()
            .find(|s| s.descriptor.media_type == MediaType::Video)
            .unwrap();
        assert_eq!(vid.descriptor.width, Some(1920));

        engine.stop().expect("stop engine");
    }

    #[test]
    fn scaling_clarity_checked_overflow_fallback() {
        let capture =
            ScalingMockCapture::with_gpu(vec![VideoCaptureEvent::Frame(test_video_frame(0))]);
        let encoder = ScalingMockEncoder::new();
        let configs = Arc::clone(&encoder.configured_configs);

        let mut engine = RecorderEngine::new(
            EngineConfig {
                retention_ms: 1,
                video_encoder: VideoEncoderConfig {
                    width: 3_000_000_000,
                    height: 1080,
                    fps: 60,
                    ..VideoEncoderConfig::default()
                },
                scaling_plan: EngineScalingPlan::Prefer2xWithFallback,
                ..EngineConfig::default()
            },
            Box::new(capture),
            Box::new(encoder),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let events = engine.pump();

        assert!(events.iter().any(|e| matches!(
            e,
            EngineEvent::Warning { code, .. } if code == CLARITY_DIMENSIONS_OVERFLOW_CODE
        )));

        let status = engine.scaling_status();
        assert_eq!(status.state, VideoScalingState::ActiveNative1x);
        assert_eq!(status.active_dimensions, Some((3_000_000_000, 1080)));
        assert_eq!(
            status.fallback_reason,
            Some(ScalingFallbackReason::DimensionsOverflow)
        );

        let recorded_configs = configs.lock().unwrap().clone();
        assert_eq!(recorded_configs.len(), 1);
        assert_eq!(recorded_configs[0].width, 3_000_000_000);

        engine.stop().expect("stop engine");
    }

    #[test]
    fn scaling_clarity_encoder_negotiation_failed_fallback() {
        let capture = ScalingMockCapture::with_gpu(vec![
            VideoCaptureEvent::Frame(test_video_frame(0)),
            VideoCaptureEvent::Frame(test_video_frame(33)),
        ]);
        let mut encoder = ScalingMockEncoder::new();
        encoder.fail_if_dimensions = Some((3840, 2160));
        let configs = Arc::clone(&encoder.configured_configs);

        let mut engine = RecorderEngine::new(
            EngineConfig {
                retention_ms: 1,
                scaling_plan: EngineScalingPlan::Prefer2xWithFallback,
                ..EngineConfig::default()
            },
            Box::new(capture),
            Box::new(encoder),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let events = engine.pump();

        assert!(events.iter().any(|e| matches!(
            e,
            EngineEvent::Warning { code, .. } if code == CLARITY_ENCODER_NEGOTIATION_FAILED_CODE
        )));

        let status = engine.scaling_status();
        assert_eq!(status.state, VideoScalingState::ActiveNative1x);
        assert_eq!(status.active_dimensions, Some((1920, 1080)));
        assert!(matches!(
            status.fallback_reason,
            Some(ScalingFallbackReason::EncoderNegotiationFailed { .. })
        ));

        let recorded_configs = configs.lock().unwrap().clone();
        assert_eq!(recorded_configs.len(), 1);
        assert_eq!(recorded_configs[0].width, 1920);

        let snapshot = engine.snapshot_replay().expect("snapshot");
        let vid = snapshot
            .streams
            .iter()
            .find(|s| s.descriptor.media_type == MediaType::Video)
            .unwrap();
        assert_eq!(vid.descriptor.width, Some(1920));

        engine.stop().expect("stop engine");
    }

    #[test]
    fn scaling_clarity_first_frame_failure_retries_at_1x_after_epoch_reset() {
        let capture = ScalingMockCapture::with_gpu(vec![
            VideoCaptureEvent::Frame(test_video_frame(0)),
            VideoCaptureEvent::Frame(test_video_frame(33)),
        ]);
        let mut encoder = ScalingMockEncoder::new();
        encoder.fail_on_first_2x_encode = true;
        let configs = Arc::clone(&encoder.configured_configs);

        let mut engine = RecorderEngine::new(
            EngineConfig {
                retention_ms: 1,
                scaling_plan: EngineScalingPlan::Prefer2xWithFallback,
                ..EngineConfig::default()
            },
            Box::new(capture),
            Box::new(encoder),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let events = engine.pump();

        assert!(events.iter().any(|e| matches!(
            e,
            EngineEvent::Warning { code, .. } if code == CLARITY_STARTUP_PROBE_FAILED_CODE
        )));

        let status = engine.scaling_status();
        assert_eq!(status.state, VideoScalingState::ActiveNative1x);
        assert_eq!(status.active_dimensions, Some((1920, 1080)));
        assert!(matches!(
            status.fallback_reason,
            Some(ScalingFallbackReason::StartupProbeFailed { .. })
        ));

        let recorded_configs = configs.lock().unwrap().clone();
        assert_eq!(recorded_configs.len(), 2);
        assert_eq!(recorded_configs[0].width, 3840);
        assert_eq!(recorded_configs[1].width, 1920);

        let snapshot = engine.snapshot_replay().expect("snapshot");
        let vid = snapshot
            .streams
            .iter()
            .find(|s| s.descriptor.media_type == MediaType::Video)
            .unwrap();
        assert_eq!(vid.descriptor.width, Some(1920));

        engine.stop().expect("stop engine");
    }

    #[test]
    fn scaling_clarity_baseline_retry_failure_becomes_worker_failed() {
        let capture =
            ScalingMockCapture::with_gpu(vec![VideoCaptureEvent::Frame(test_video_frame(0))]);
        let mut encoder = ScalingMockEncoder::new();
        encoder.fail_on_first_2x_encode = true;
        encoder.fail_on_first_1x_encode = true;

        let mut engine = RecorderEngine::new(
            EngineConfig {
                retention_ms: 1,
                scaling_plan: EngineScalingPlan::Prefer2xWithFallback,
                ..EngineConfig::default()
            },
            Box::new(capture),
            Box::new(encoder),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let events = engine.pump();

        assert_eq!(engine.state(), RecorderState::Error);
        assert!(events.iter().any(|e| matches!(
            e,
            EngineEvent::Warning { code, .. } if code == "ENCODER_ENCODE_FAILED"
        )));

        engine.stop().expect("stop engine");
    }

    #[test]
    fn scaling_clarity_later_frame_failure_does_not_downgrade() {
        let capture = ScalingMockCapture::with_gpu(vec![
            VideoCaptureEvent::Frame(test_video_frame(0)),
            VideoCaptureEvent::Frame(test_video_frame(33)),
        ]);
        let mut encoder = ScalingMockEncoder::new();
        encoder.fail_on_later_encode = true;
        let configs = Arc::clone(&encoder.configured_configs);

        let mut engine = RecorderEngine::new(
            EngineConfig {
                scaling_plan: EngineScalingPlan::Prefer2xWithFallback,
                ..EngineConfig::default()
            },
            Box::new(capture),
            Box::new(encoder),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        engine.pump();

        assert_eq!(engine.state(), RecorderState::Error);

        let recorded_configs = configs.lock().unwrap().clone();
        assert_eq!(recorded_configs.len(), 1);
        assert_eq!(recorded_configs[0].width, 3840);

        engine.stop().expect("stop engine");
    }

    #[test]
    fn scaling_format_change_recomputes_resolution() {
        let capture = ScalingMockCapture::with_gpu(vec![
            VideoCaptureEvent::Frame(test_video_frame(0)),
            VideoCaptureEvent::FormatChanged {
                width: 1280,
                height: 720,
            },
            VideoCaptureEvent::Frame(test_video_frame(33)),
        ]);
        let encoder = ScalingMockEncoder::new();
        let configs = Arc::clone(&encoder.configured_configs);

        let mut engine = RecorderEngine::new(
            EngineConfig {
                scaling_plan: EngineScalingPlan::Prefer2xWithFallback,
                follow_source_dimensions: true,
                ..EngineConfig::default()
            },
            Box::new(capture),
            Box::new(encoder),
            Box::new(EngineTestMuxer),
        );

        engine.start().expect("start engine");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !engine.worker_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        engine.pump();

        let recorded_configs = configs.lock().unwrap().clone();
        assert_eq!(recorded_configs.len(), 2);
        assert_eq!(recorded_configs[0].width, 3840);
        assert_eq!(recorded_configs[0].height, 2160);
        assert_eq!(recorded_configs[1].width, 2560);
        assert_eq!(recorded_configs[1].height, 1440);

        let status = engine.scaling_status();
        assert_eq!(status.state, VideoScalingState::ActiveSupersampled2x);
        assert_eq!(status.active_dimensions, Some((2560, 1440)));

        engine.stop().expect("stop engine");
    }
}
