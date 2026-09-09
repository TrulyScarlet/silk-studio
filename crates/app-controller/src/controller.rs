//! The controller itself: command execution, event assembly, notification
//! routing, and bounded save-job dispatch.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use capture_api::VideoCapture;
use configuration::VideoFidelityMode;
use encoder_api::VideoEncoder;
use muxer::Muxer;
use recorder_engine::{AudioInput, EngineConfig, EngineEvent, RecorderEngine, RecorderState};
use serde::{Deserialize, Serialize};

use crate::commands::Command;
use crate::events::ControllerEvent;
use crate::jobs::{SaveMetrics, SaveResult, SaveWork, SaveWorker};
use crate::notification::{NotificationContent, NotificationSink};

pub const ARCHIVAL_444_UNSUPPORTED_CODE: &str = "ARCHIVAL_444_UNSUPPORTED_ON_HARDWARE";
pub const ARCHIVAL_444_UNSUPPORTED_MESSAGE: &str =
    "Hardware encoder does not support 4:4:4 encoding; please use Enhanced clarity or Standard mode.";
pub const ARCHIVAL_PIPELINE_UNAVAILABLE_CODE: &str = ARCHIVAL_444_UNSUPPORTED_CODE;
pub const ARCHIVAL_PIPELINE_UNAVAILABLE_MESSAGE: &str = ARCHIVAL_444_UNSUPPORTED_MESSAGE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FidelityIssueKind {
    Fallback,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FidelityIssue {
    pub kind: FidelityIssueKind,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FidelityStatus {
    pub requested: VideoFidelityMode,
    pub active: Option<VideoFidelityMode>,
    pub issue: Option<FidelityIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FidelityAvailability {
    Available,
    RuntimeChecked,
    FallbackOnly,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FidelityCapability {
    pub mode: VideoFidelityMode,
    pub availability: FidelityAvailability,
    pub issue: Option<FidelityIssue>,
}

pub fn pipeline_fidelity_capabilities() -> Vec<FidelityCapability> {
    vec![
        FidelityCapability {
            mode: VideoFidelityMode::Standard,
            availability: FidelityAvailability::Available,
            issue: None,
        },
        FidelityCapability {
            mode: VideoFidelityMode::Clarity,
            availability: FidelityAvailability::RuntimeChecked,
            issue: None,
        },
        FidelityCapability {
            mode: VideoFidelityMode::Archival444,
            availability: FidelityAvailability::Unsupported,
            issue: Some(FidelityIssue {
                kind: FidelityIssueKind::Fallback,
                code: ARCHIVAL_444_UNSUPPORTED_CODE.to_string(),
                message: ARCHIVAL_444_UNSUPPORTED_MESSAGE.to_string(),
            }),
        },
    ]
}

/// Factory producing a fresh capture/encode/mux triple for each session. Real
/// backends are injected by the platform layer; tests inject mocks.
pub type DepsFactory =
    Box<dyn FnMut() -> (Box<dyn VideoCapture>, Box<dyn VideoEncoder>, Box<dyn Muxer>) + Send>;

/// Factory producing independently owned audio sources for each recording
/// session. It receives the current ordered enabled plans so device and source
/// changes are applied on the next start. Each source is moved to its own
/// engine worker.
pub type AudioDepsFactory = Box<dyn FnMut(&[AudioTrackPlan]) -> Vec<AudioInput> + Send>;

/// Neutral controller-side name for one configured audio source. The shape is
/// shared with configuration so the controller does not need desktop- or
/// microphone-specific cases.
pub type AudioTrackPlan = configuration::AudioTrackSettings;

/// Optional platform-neutral context used when organizing a saved clip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipAttribution {
    pub game_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ControllerSettings {
    pub output_dir: PathBuf,
    pub file_name_pattern: String,
    pub organize_by_game: bool,
    pub source_name: String,
    pub retention_ms: i64,
    pub minimum_free_space_bytes: u64,
    pub video_encoder_config: encoder_api::VideoEncoderConfig,
    /// Resolve encoder dimensions from the selected capture source and keep
    /// following capture format changes instead of scaling to a fixed output.
    pub follow_source_dimensions: bool,
    pub encoder_preference: encoder_api::EncoderPreference,
    pub audio_tracks: Vec<AudioTrackPlan>,
    pub audio_bitrate_kbps: u32,
    pub output_options: muxer::SaveOptions,
    pub fidelity_mode: VideoFidelityMode,
}

pub const DEFAULT_MINIMUM_FREE_SPACE_BYTES: u64 = 512 * 1024 * 1024;

impl Default for ControllerSettings {
    fn default() -> Self {
        Self {
            output_dir: std::env::temp_dir().join("silk-dev-clips"),
            file_name_pattern: configuration::DEFAULT_FILE_NAME_PATTERN.to_string(),
            organize_by_game: false,
            source_name: "display-1".to_string(),
            retention_ms: 400,
            minimum_free_space_bytes: DEFAULT_MINIMUM_FREE_SPACE_BYTES,
            video_encoder_config: encoder_api::VideoEncoderConfig::default(),
            follow_source_dimensions: false,
            encoder_preference: encoder_api::EncoderPreference::Auto,
            audio_tracks: configuration::AudioSettings::default().tracks,
            audio_bitrate_kbps: encoder_api::AudioEncoderConfig::default().bitrate_kbps,
            output_options: muxer::SaveOptions::default(),
            fidelity_mode: VideoFidelityMode::Standard,
        }
    }
}

pub struct Controller {
    engine: Option<RecorderEngine>,
    factory: DepsFactory,
    audio_factory: Option<AudioDepsFactory>,
    settings: ControllerSettings,
    sink: Arc<dyn NotificationSink>,
    save_worker: Option<SaveWorker>,
    save_metrics: SaveMetrics,
    reserved_paths: BTreeSet<PathBuf>,
    session_requested_fidelity: Option<VideoFidelityMode>,
}

impl Controller {
    pub fn new(factory: DepsFactory, sink: Arc<dyn NotificationSink>) -> Self {
        Self::with_settings(factory, sink, ControllerSettings::default())
    }

    pub fn with_settings(
        factory: DepsFactory,
        sink: Arc<dyn NotificationSink>,
        settings: ControllerSettings,
    ) -> Self {
        Self::with_settings_and_audio(factory, None, sink, settings)
    }

    pub fn with_audio_factory(
        factory: DepsFactory,
        audio_factory: AudioDepsFactory,
        sink: Arc<dyn NotificationSink>,
        settings: ControllerSettings,
    ) -> Self {
        Self::with_settings_and_audio(factory, Some(audio_factory), sink, settings)
    }

    fn with_settings_and_audio(
        factory: DepsFactory,
        audio_factory: Option<AudioDepsFactory>,
        sink: Arc<dyn NotificationSink>,
        settings: ControllerSettings,
    ) -> Self {
        Self {
            engine: None,
            factory,
            audio_factory,
            settings,
            sink,
            save_worker: None,
            save_metrics: SaveMetrics::default(),
            reserved_paths: BTreeSet::new(),
            session_requested_fidelity: None,
        }
    }

    /// Last known recorder state; `Stopped` when no engine exists.
    pub fn state(&self) -> RecorderState {
        self.engine
            .as_ref()
            .map(RecorderEngine::state)
            .unwrap_or(RecorderState::Stopped)
    }

    /// Runtime fidelity status including requested/active mode and any
    /// structured fallback/unsupported issues.
    pub fn fidelity_status(&self) -> FidelityStatus {
        if let Some(engine) = &self.engine {
            let requested = self
                .session_requested_fidelity
                .unwrap_or(self.settings.fidelity_mode);
            let scaling = engine.scaling_status();
            let (active, issue) = match scaling.state {
                recorder_engine::VideoScalingState::Pending => (None, None),
                recorder_engine::VideoScalingState::ActiveSupersampled2x => {
                    (Some(VideoFidelityMode::Clarity), None)
                }
                recorder_engine::VideoScalingState::ActiveNative1x => {
                    let issue = scaling.fallback_reason.map(|reason| FidelityIssue {
                        kind: FidelityIssueKind::Fallback,
                        code: reason.code().to_string(),
                        message: reason.message().to_string(),
                    });
                    let active_mode =
                        if requested == VideoFidelityMode::Archival444 && issue.is_none() {
                            VideoFidelityMode::Archival444
                        } else {
                            VideoFidelityMode::Standard
                        };
                    (Some(active_mode), issue)
                }
                recorder_engine::VideoScalingState::Failed => {
                    let issue = scaling.fallback_reason.map(|reason| FidelityIssue {
                        kind: FidelityIssueKind::Fallback,
                        code: reason.code().to_string(),
                        message: reason.message().to_string(),
                    });
                    (None, issue)
                }
            };
            FidelityStatus {
                requested,
                active,
                issue,
            }
        } else {
            let requested = self.settings.fidelity_mode;
            let issue = if requested == VideoFidelityMode::Archival444 {
                Some(FidelityIssue {
                    kind: FidelityIssueKind::Fallback,
                    code: ARCHIVAL_444_UNSUPPORTED_CODE.to_string(),
                    message: ARCHIVAL_444_UNSUPPORTED_MESSAGE.to_string(),
                })
            } else {
                None
            };
            FidelityStatus {
                requested,
                active: None,
                issue,
            }
        }
    }

    /// Packets retained by the engine for replay purposes (test/telemetry
    /// surface; the UI uses aggregate readiness instead).
    pub fn buffered_packet_count(&self) -> usize {
        self.engine
            .as_ref()
            .map(|e| e.buffered_packet_count())
            .unwrap_or(0)
    }

    /// Number of audio sources that have produced synchronized output in the
    /// current session. This is a diagnostics surface, not media data.
    pub fn audio_stream_count(&self) -> usize {
        self.engine
            .as_ref()
            .map(|engine| engine.audio_sync_metrics().len())
            .unwrap_or(0)
    }

    /// Buffered video span in milliseconds for the status surface.
    pub fn buffer_duration_ms(&self) -> i64 {
        self.engine
            .as_ref()
            .map(RecorderEngine::buffer_duration_ms)
            .unwrap_or(0)
    }

    /// Apply settings that are safe to change while a session is active.
    /// Output naming takes effect on the next save; retention is re-evicted
    /// immediately by the replay buffer.
    pub fn update_settings(&mut self, settings: ControllerSettings) -> Result<(), String> {
        if settings.retention_ms <= 0 {
            return Err("replay retention must be positive".to_string());
        }
        let mut validation_config = settings.video_encoder_config.clone();
        if settings.follow_source_dimensions {
            // Native dimensions are deliberately unresolved until start, when
            // the selected capture backend can enumerate the real source.
            validation_config.width = 2;
            validation_config.height = 2;
        }
        validation_config
            .validate()
            .map_err(|error| error.to_string())?;
        validate_audio_track_count(&settings.audio_tracks)?;
        if settings.audio_bitrate_kbps == 0 {
            return Err("audio bitrate must be positive".to_string());
        }
        if let Some(engine) = self.engine.as_mut() {
            engine
                .set_replay_retention_ms(settings.retention_ms)
                .map_err(|error| error.to_string())?;
        } else {
            self.session_requested_fidelity = None;
        }
        self.settings = settings;
        Ok(())
    }

    /// Latest bounded save-pipeline telemetry. The worker owns live updates;
    /// the controller retains the last snapshot after shutdown.
    pub fn save_metrics(&self) -> SaveMetrics {
        self.save_worker
            .as_ref()
            .map(SaveWorker::metrics)
            .unwrap_or_else(|| self.save_metrics.clone())
    }

    /// Drain pending engine transitions without executing a command (the
    /// UI bridge calls this on its event timer). Emits StatusChanged etc.
    pub fn poll(&mut self) -> Vec<ControllerEvent> {
        let mut events = match self.engine.as_mut() {
            None => Vec::new(),
            Some(engine) => map_engine_events(engine.pump()),
        };
        let save_results = self
            .save_worker
            .as_ref()
            .map(SaveWorker::drain)
            .unwrap_or_default();
        events.extend(self.map_save_results(save_results));
        events
    }

    /// Execute one command and return the events it produced. Pending
    /// engine transitions are drained first so state-dependent commands
    /// see current reality; events are plain data for the UI bridge.
    pub fn execute(&mut self, command: &Command) -> Vec<ControllerEvent> {
        self.execute_with_attribution(command, None)
    }

    /// Execute one command, optionally attributing a saved replay to a game.
    /// Attribution is used only while planning the save path.
    pub fn execute_with_attribution(
        &mut self,
        command: &Command,
        attribution: Option<&ClipAttribution>,
    ) -> Vec<ControllerEvent> {
        let mut events = self.poll();
        let produced = match command {
            Command::StartCapture => self.start(),
            Command::StopCapture => self.stop(),
            Command::SaveReplay => self.save_replay(attribution),
            Command::Poll => Vec::new(),
            Command::Ping { message } => self.ping(message),
        };
        events.extend(produced);
        // A command may have been the one that primed the buffer, or a fast
        // save worker may have completed the request just submitted.
        events.extend(self.poll());
        events
    }

    fn start(&mut self) -> Vec<ControllerEvent> {
        if let Some(ref engine) = self.engine {
            if engine.state() != RecorderState::Error {
                return vec![reject(
                    Command::StartCapture.name(),
                    "recorder already started",
                )];
            }
            let mut old_engine = self.engine.take().unwrap();
            let _ = old_engine.stop();
        }
        if let Err(reason) = validate_audio_track_count(&self.settings.audio_tracks) {
            return vec![reject(Command::StartCapture.name(), &reason)];
        }
        let (video, video_encoder, muxer) = (self.factory)();

        let mut video_encoder_config = self.settings.video_encoder_config.clone();
        if self.settings.fidelity_mode == VideoFidelityMode::Archival444 {
            let supported = video_encoder
                .capabilities()
                .supported_pixel_formats
                .contains(&media_types::PixelFormat::Ayuv);
            if !supported {
                self.session_requested_fidelity = None;
                let issue = FidelityIssue {
                    kind: FidelityIssueKind::Fallback,
                    code: ARCHIVAL_444_UNSUPPORTED_CODE.to_string(),
                    message: ARCHIVAL_444_UNSUPPORTED_MESSAGE.to_string(),
                };
                return vec![
                    reject(Command::StartCapture.name(), &issue.message),
                    ControllerEvent::Warning {
                        code: issue.code,
                        message: issue.message,
                    },
                ];
            }
            video_encoder_config.pixel_format = media_types::PixelFormat::Ayuv;
        }

        let mut events =
            match cleanup_staged_files(&self.settings.output_dir, self.settings.organize_by_game) {
                Ok(0) => Vec::new(),
                Ok(count) => {
                    diagnostics::info(
                        "controller",
                        &format!("removed {count} stale staged clip(s)"),
                    );
                    Vec::new()
                }
                Err(error) => {
                    diagnostics::warn(
                        "controller",
                        &format!("staged clip cleanup failed: {error}"),
                    );
                    vec![ControllerEvent::Warning {
                        code: "STAGED_FILE_CLEANUP_FAILED".to_string(),
                        message: error.to_string(),
                    }]
                }
            };
        let audio_plans: Vec<AudioTrackPlan> = self
            .settings
            .audio_tracks
            .iter()
            .filter(|track| track.enabled)
            .cloned()
            .collect();
        let audio_inputs = self
            .audio_factory
            .as_mut()
            .map(|factory| factory(&audio_plans))
            .unwrap_or_default();
        let scaling_plan = match self.settings.fidelity_mode {
            VideoFidelityMode::Clarity => recorder_engine::EngineScalingPlan::Prefer2xWithFallback,
            _ => recorder_engine::EngineScalingPlan::Native1x,
        };
        let engine_config = EngineConfig {
            retention_ms: self.settings.retention_ms,
            video_source_id: self.settings.source_name.clone(),
            video_encoder: video_encoder_config,
            follow_source_dimensions: self.settings.follow_source_dimensions,
            scaling_plan,
            audio_encoder: encoder_api::AudioEncoderConfig {
                bitrate_kbps: self.settings.audio_bitrate_kbps,
                ..encoder_api::AudioEncoderConfig::default()
            },
            ..EngineConfig::default()
        };
        let mut engine = RecorderEngine::new_with_audio(
            engine_config,
            video,
            video_encoder,
            audio_inputs,
            muxer,
        );

        match engine.start() {
            Ok(()) => {
                match engine
                    .take_muxer()
                    .map_err(|error| error.to_string())
                    .and_then(|muxer| SaveWorker::new(muxer).map_err(|error| error.to_string()))
                {
                    Ok(worker) => self.save_worker = Some(worker),
                    Err(error) => {
                        diagnostics::error(
                            "controller",
                            &format!("save worker could not start: {error}"),
                        );
                        events.push(ControllerEvent::Warning {
                            code: "SAVE_WORKER_START_FAILED".to_string(),
                            message: error,
                        });
                        let _ = engine.stop();
                        events.extend(map_engine_events(engine.pump()));
                        self.sink.notify(&NotificationContent::error(
                            "Silk could not start capture",
                            "save worker could not start",
                        ));
                    }
                }
                if engine.state() != RecorderState::Stopped {
                    events.extend(map_engine_events(engine.pump()));
                    diagnostics::info("controller", "capture started");
                    self.session_requested_fidelity = Some(self.settings.fidelity_mode);
                    self.engine = Some(engine);
                } else {
                    self.session_requested_fidelity = None;
                }
            }
            Err(err) => {
                self.session_requested_fidelity = None;
                diagnostics::error("controller", &format!("start failed: {err}"));
                events.push(ControllerEvent::StatusChanged {
                    state: RecorderState::Error.to_string(),
                });
                events.push(ControllerEvent::Warning {
                    code: err.code().to_string(),
                    message: err.to_string(),
                });
                let _ = engine.stop();
                events.extend(map_engine_events(engine.pump()));
                self.sink.notify(&NotificationContent::error(
                    "Silk could not start capture",
                    err.to_string(),
                ));
            }
        }
        events
    }

    fn stop(&mut self) -> Vec<ControllerEvent> {
        self.session_requested_fidelity = None;
        let Some(mut engine) = self.engine.take() else {
            return vec![reject(
                Command::StopCapture.name(),
                "recorder is not running",
            )];
        };
        let mut events = Vec::new();
        match engine.stop() {
            Ok(()) => {
                events.extend(map_engine_events(engine.pump()));
                diagnostics::info("controller", "capture stopped");
            }
            Err(err) => {
                events.push(ControllerEvent::Warning {
                    code: err.code().to_string(),
                    message: err.to_string(),
                });
            }
        }
        let save_results = if let Some(mut worker) = self.save_worker.take() {
            let results = worker.shutdown();
            self.save_metrics = worker.metrics();
            results
        } else {
            Vec::new()
        };
        events.extend(self.map_save_results(save_results));
        events.push(ControllerEvent::StatusChanged {
            state: RecorderState::Stopped.to_string(),
        });
        events
    }

    fn save_replay(&mut self, attribution: Option<&ClipAttribution>) -> Vec<ControllerEvent> {
        // Immutable pre-checks before taking the engine borrow.
        let current_state = self.engine.as_ref().map(|e| e.state());
        let Some(state) = current_state else {
            return vec![reject(
                Command::SaveReplay.name(),
                "recorder is not running",
            )];
        };
        let buffered_ms = self.buffer_duration_ms();
        let retention_ms = self.settings.retention_ms;
        let packet_count = self.buffered_packet_count();
        let can_save_now = state.can_save()
            || ((state == RecorderState::Buffering || state == RecorderState::Error)
                && buffered_ms >= 1000
                && packet_count > 0);

        if !can_save_now {
            return vec![ControllerEvent::CommandRejected {
                command: Command::SaveReplay.name().to_string(),
                reason: format!(
                    "buffer not ready yet (state '{state}'; buffered {buffered_ms}/{retention_ms} ms across {packet_count} packets)"
                ),
            }];
        }

        let clip_path = self.next_clip_path(attribution);
        let snapshot = match self.engine.as_mut() {
            Some(engine) => match engine.snapshot_replay() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.reserved_paths.remove(&clip_path);
                    return vec![ControllerEvent::SaveFailed {
                        code: error.code().to_string(),
                        message: error.to_string(),
                    }];
                }
            },
            None => {
                self.reserved_paths.remove(&clip_path);
                return vec![reject(
                    Command::SaveReplay.name(),
                    "recorder is not running",
                )];
            }
        };

        let Some(worker) = self.save_worker.as_ref() else {
            self.reserved_paths.remove(&clip_path);
            return vec![ControllerEvent::SaveFailed {
                code: "SAVE_WORKER_UNAVAILABLE".to_string(),
                message: "save worker is not running".to_string(),
            }];
        };
        let submit = worker.submit(SaveWork {
            snapshot,
            clip_path: clip_path.clone(),
            minimum_free_space_bytes: self.settings.minimum_free_space_bytes,
            options: self.settings.output_options,
        });
        match submit {
            Ok(()) => vec![ControllerEvent::SaveQueued {
                path: clip_path.display().to_string(),
            }],
            Err(error) => {
                self.reserved_paths.remove(&clip_path);
                vec![ControllerEvent::SaveFailed {
                    code: match &error {
                        crate::jobs::JobQueueError::QueueFull { .. } => "SAVE_QUEUE_FULL",
                        crate::jobs::JobQueueError::WorkerStopped => "SAVE_WORKER_STOPPED",
                    }
                    .to_string(),
                    message: error.to_string(),
                }]
            }
        }
    }

    fn ping(&mut self, message: &str) -> Vec<ControllerEvent> {
        let body = format!("pong:{message}");
        self.sink
            .notify(&NotificationContent::info("Silk", body.clone()));
        vec![ControllerEvent::Notification {
            level: "info".to_string(),
            title: "Silk".to_string(),
            body,
        }]
    }

    fn next_clip_path(&mut self, attribution: Option<&ClipAttribution>) -> PathBuf {
        let now = chrono::Local::now();
        let date = now.format("%Y-%m-%d").to_string();
        let time = now.format("%H-%M-%S").to_string();
        let mut collision_index = 1_u32;
        loop {
            let candidate =
                render_clip_path(&self.settings, attribution, &date, &time, collision_index);
            if !candidate.exists()
                && !staged_path(&candidate).exists()
                && !staged_lock_path(&candidate).exists()
                && !self.reserved_paths.contains(&candidate)
            {
                self.reserved_paths.insert(candidate.clone());
                return candidate;
            }
            collision_index = collision_index.saturating_add(1);
        }
    }

    fn map_save_results(&mut self, results: Vec<SaveResult>) -> Vec<ControllerEvent> {
        let mut events = Vec::new();
        for result in results {
            match result {
                SaveResult::Completed(metadata) => {
                    self.reserved_paths.remove(&metadata.path);
                    self.sink.notify(&NotificationContent::info(
                        "Replay saved",
                        format!(
                            "{} ({}, {} KiB)",
                            metadata
                                .path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            format_duration(metadata.duration_ms),
                            metadata.size_bytes / 1024
                        ),
                    ));
                    diagnostics::info(
                        "controller",
                        &format!("clip saved: {}", metadata.path.display()),
                    );
                    events.push(ControllerEvent::ClipSaved {
                        path: metadata.path.display().to_string(),
                        duration_ms: metadata.duration_ms,
                        size_bytes: metadata.size_bytes,
                    });
                }
                SaveResult::Failed { path, error } => {
                    self.reserved_paths.remove(&path);
                    self.sink.notify(&NotificationContent::error(
                        "Replay save failed",
                        error.to_string(),
                    ));
                    diagnostics::error("controller", &format!("save failed: {error}"));
                    events.push(ControllerEvent::SaveFailed {
                        code: error.code().to_string(),
                        message: error.to_string(),
                    });
                }
            }
        }
        events
    }
}

fn render_file_name(
    pattern: &str,
    date: &str,
    time: &str,
    source: &str,
    collision_index: u32,
) -> String {
    let rendered = pattern
        .replace("{date}", date)
        .replace("{time}", time)
        .replace("{source}", source)
        .replace("{index}", &collision_index.to_string());
    let mut sanitized = sanitize_file_name(&rendered);
    if !pattern.contains("{index}") && collision_index > 1 {
        sanitized.push_str(&format!("_{}", collision_index - 1));
    }
    sanitized
}

pub fn clean_source_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "display-1".to_string();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("display") {
        if let Some(idx) = lower.find("display") {
            let suffix = &trimmed[idx + 7..];
            let num = suffix.trim_matches(|c: char| !c.is_ascii_digit());
            if !num.is_empty() {
                return format!("display-{num}");
            }
            return "display-1".to_string();
        }
    }
    sanitize_file_name(trimmed)
}

fn render_clip_path(
    settings: &ControllerSettings,
    attribution: Option<&ClipAttribution>,
    date: &str,
    time: &str,
    collision_index: u32,
) -> PathBuf {
    let clean_source = clean_source_name(&settings.source_name);
    let stem = render_file_name(
        &settings.file_name_pattern,
        date,
        time,
        &clean_source,
        collision_index,
    );
    let output_dir = if settings.organize_by_game {
        settings.output_dir.join(sanitize_game_folder(attribution))
    } else {
        settings.output_dir.clone()
    };
    output_dir.join(format!("{stem}.mp4"))
}

const UNCATEGORIZED_GAME_FOLDER: &str = "Uncategorized";
const MAX_GAME_FOLDER_LENGTH: usize = 64;

fn sanitize_file_name(value: &str) -> String {
    sanitize_windows_component(value, "Clip")
}

/// Clean and normalize a raw process name or window title into a clean game title.
///
/// Strips engine build tags like `-Win64-Shipping`, normalizes internal executable
/// codenames (e.g., `Discovery-D` -> `The Finals`, `r5apex` -> `Apex Legends`),
/// and converts raw identifiers to clean, readable game folder names.
pub fn clean_game_name(raw_name: &str, window_title: Option<&str>) -> String {
    let trimmed = raw_name.trim();
    if trimmed.is_empty() {
        return UNCATEGORIZED_GAME_FOLDER.to_string();
    }

    if let Some(title) = window_title {
        let title_trimmed = title.trim();
        let lower_title = title_trimmed.to_ascii_lowercase();
        if !title_trimmed.is_empty()
            && !lower_title.contains("default ime")
            && !lower_title.contains("msctfime ui")
            && !lower_title.contains("program manager")
        {
            if lower_title == "the finals" || lower_title.starts_with("the finals") {
                return "The Finals".to_string();
            }
            if lower_title == "valorant" || lower_title.starts_with("valorant") {
                return "Valorant".to_string();
            }
        }
    }

    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "discovery" | "discovery-d" | "discovery-shipping" | "discovery_shipping" => {
            return "The Finals".to_string();
        }
        "valorant" | "valorant-win64-shipping" => {
            return "Valorant".to_string();
        }
        "r5apex" | "r5apex_dx12" => {
            return "Apex Legends".to_string();
        }
        "cs2" => {
            return "Counter-Strike 2".to_string();
        }
        "csgo" => {
            return "Counter-Strike Global Offensive".to_string();
        }
        "fortniteclient-win64-shipping" | "fortniteclient" | "fortnitelauncher" => {
            return "Fortnite".to_string();
        }
        "overwatch" | "overwatch2" => {
            return "Overwatch 2".to_string();
        }
        "leagueclient" | "leagueclientux" | "league of legends" => {
            return "League of Legends".to_string();
        }
        "rocketleague" => {
            return "Rocket League".to_string();
        }
        "destiny2" => {
            return "Destiny 2".to_string();
        }
        "rainbowsix" | "rainbowsix_vulkan" => {
            return "Rainbow Six Siege".to_string();
        }
        "deadlock" => {
            return "Deadlock".to_string();
        }
        "gta5" | "playgtav" => {
            return "Grand Theft Auto V".to_string();
        }
        "robloxplayerbeta" | "robloxplayer" => {
            return "Roblox".to_string();
        }
        "genshinimpact" => {
            return "Genshin Impact".to_string();
        }
        "starrail" => {
            return "Honkai Star Rail".to_string();
        }
        "zenlesszonezero" => {
            return "Zenless Zone Zero".to_string();
        }
        "marvelrivals-win64-shipping" | "marvelrivals" => {
            return "Marvel Rivals".to_string();
        }
        "tslgame" => {
            return "PUBG Battlegrounds".to_string();
        }
        "helldivers2" => {
            return "Helldivers 2".to_string();
        }
        "explorer" => {
            return "Desktop".to_string();
        }
        _ => {}
    }

    let mut cleaned = trimmed.to_string();
    const SUFFIXES: &[&str] = &[
        "-win64-shipping",
        "_win64_shipping",
        "-win32-shipping",
        "_win32_shipping",
        "-shipping",
        "_shipping",
        "-win64",
        "_win64",
        "-win32",
        "_win32",
        "-x64",
        "_x64",
        "-x86",
        "_x86",
        "_retail",
        "-retail",
        "_release",
        "-release",
        "client-win64-shipping",
    ];

    for suffix in SUFFIXES {
        if cleaned.to_ascii_lowercase().ends_with(suffix) {
            let new_len = cleaned.len() - suffix.len();
            cleaned.truncate(new_len);
            break;
        }
    }

    let cleaned = cleaned.replace(['_', '-'], " ");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return UNCATEGORIZED_GAME_FOLDER.to_string();
    }

    let is_all_caps = cleaned
        .chars()
        .all(|c| !c.is_alphabetic() || c.is_uppercase());
    let is_all_lower = cleaned
        .chars()
        .all(|c| !c.is_alphabetic() || c.is_lowercase());
    if is_all_caps || is_all_lower {
        let mut result = String::new();
        let mut capitalize_next = true;
        for c in cleaned.chars() {
            if c.is_whitespace() {
                result.push(c);
                capitalize_next = true;
            } else if capitalize_next {
                result.extend(c.to_uppercase());
                capitalize_next = false;
            } else {
                result.extend(c.to_lowercase());
            }
        }
        result
    } else {
        cleaned.to_string()
    }
}

fn sanitize_game_folder(attribution: Option<&ClipAttribution>) -> String {
    let Some(game_name) = attribution.and_then(|attribution| attribution.game_name.as_deref())
    else {
        return UNCATEGORIZED_GAME_FOLDER.to_string();
    };
    if game_name.trim().is_empty() {
        return UNCATEGORIZED_GAME_FOLDER.to_string();
    }

    let sanitized = sanitize_windows_component(game_name.trim(), UNCATEGORIZED_GAME_FOLDER);
    let truncated: String = sanitized.chars().take(MAX_GAME_FOLDER_LENGTH).collect();
    sanitize_windows_component(&truncated, UNCATEGORIZED_GAME_FOLDER)
}

fn sanitize_windows_component(value: &str, empty_fallback: &str) -> String {
    const RESERVED_NAMES: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    const INVALID: [char; 9] = ['<', '>', ':', '"', '|', '?', '*', '/', '\\'];

    let mut sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_control() || INVALID.contains(&character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    while sanitized.ends_with(['.', ' ']) {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        return empty_fallback.to_string();
    }

    let stem = sanitized.split('.').next().unwrap_or_default();
    if RESERVED_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        sanitized.insert(0, '_');
    }
    sanitized
}

fn staged_path(path: &Path) -> PathBuf {
    let mut staged = path.as_os_str().to_os_string();
    staged.push(".part");
    PathBuf::from(staged)
}

fn staged_lock_path(path: &Path) -> PathBuf {
    let mut staged = staged_path(path).into_os_string();
    staged.push(".lock");
    PathBuf::from(staged)
}

fn cleanup_staged_files(output_dir: &Path, organized: bool) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(output_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut removed = 0_usize;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file() && is_staged_artifact(&path) {
            std::fs::remove_file(path)?;
            removed += 1;
        } else if organized && entry.file_type()?.is_dir() {
            removed += cleanup_staged_files_in_directory(&path)?;
        }
    }
    Ok(removed)
}

fn cleanup_staged_files_in_directory(directory: &Path) -> std::io::Result<usize> {
    let entries = std::fs::read_dir(directory)?;
    let mut removed = 0_usize;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_file() && is_staged_artifact(&path) {
            std::fs::remove_file(path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn is_staged_artifact(path: &Path) -> bool {
    let name = path.file_name().map(|name| name.to_string_lossy());
    name.is_some_and(|name| name.ends_with(".part") || name.ends_with(".part.lock"))
}

fn reject(command: &str, reason: &str) -> ControllerEvent {
    ControllerEvent::CommandRejected {
        command: command.to_string(),
        reason: reason.to_string(),
    }
}

fn validate_audio_track_count(tracks: &[AudioTrackPlan]) -> Result<(), String> {
    if tracks.len() > configuration::MAX_AUDIO_TRACKS {
        return Err(format!(
            "audio track count must not exceed {}",
            configuration::MAX_AUDIO_TRACKS
        ));
    }
    let enabled = tracks.iter().filter(|track| track.enabled).count();
    if enabled > configuration::MAX_AUDIO_TRACKS {
        return Err(format!(
            "enabled audio track count must not exceed {}",
            configuration::MAX_AUDIO_TRACKS
        ));
    }
    for track in tracks {
        if !track.gain.is_finite() || !(0.0..=8.0).contains(&track.gain) {
            return Err(format!(
                "audio track '{}' gain must be finite and between 0 and 8",
                track.id
            ));
        }
    }
    Ok(())
}

fn map_engine_events(events: Vec<EngineEvent>) -> Vec<ControllerEvent> {
    events
        .into_iter()
        .filter_map(|event| match event {
            // Internal signals; `RecorderEngine::pump` consumes them and
            // surfaces validated state changes instead.
            EngineEvent::BufferPrimed => None,
            EngineEvent::RecoveryStarted { .. } | EngineEvent::RecoveryCompleted { .. } => None,
            EngineEvent::StateChanged(state) => Some(ControllerEvent::StatusChanged {
                state: state.to_string(),
            }),
            EngineEvent::Warning { code, message } => {
                Some(ControllerEvent::Warning { code, message })
            }
            EngineEvent::WorkerFailed { code, message } => {
                Some(ControllerEvent::Warning { code, message })
            }
        })
        .collect()
}

fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    let fraction = ms % 1000;
    format!("{seconds}.{fraction:02}")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent};
    use configuration::{AudioSourceKind, AudioTrackSettings, VideoFidelityMode};
    use media_types::{PixelFormat, StreamId};
    use recorder_engine::{
        RecorderState, CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE,
        CLARITY_GPU_CONTEXT_UNAVAILABLE_MESSAGE,
    };
    use test_support::{
        MockAudioEncoder, MockMuxer, MockVideoEncoder, ScriptedAudioCapture, ScriptedVideoCapture,
    };

    use super::{
        clean_game_name, cleanup_staged_files, pipeline_fidelity_capabilities, render_clip_path,
        render_file_name, sanitize_file_name, sanitize_game_folder, validate_audio_track_count,
        AudioTrackPlan, ClipAttribution, Controller, ControllerSettings, DepsFactory,
        FidelityAvailability, FidelityCapability, FidelityIssue, FidelityIssueKind, FidelityStatus,
        ARCHIVAL_444_UNSUPPORTED_CODE, ARCHIVAL_444_UNSUPPORTED_MESSAGE, MAX_GAME_FOLDER_LENGTH,
    };
    use crate::{Command, ControllerEvent};
    use test_support::TempDir;

    struct TestGpuResolver;
    impl encoder_api::GpuFrameResolver for TestGpuResolver {
        fn resolve(
            &self,
            _handle: media_types::GpuFrameHandle,
        ) -> Result<Arc<encoder_api::GpuFrameResource>, encoder_api::GpuFrameResolveError> {
            Ok(Arc::new(()))
        }
    }

    fn track(id: &str, enabled: bool, device_id: Option<&str>) -> AudioTrackSettings {
        AudioTrackSettings {
            id: id.to_string(),
            name: id.to_string(),
            enabled,
            source_kind: AudioSourceKind::OutputLoopback,
            device_id: device_id.map(str::to_string),
            gain: 1.0,
        }
    }

    #[test]
    fn renders_tokens_and_collision_suffixes() {
        assert_eq!(
            render_file_name(
                "Clip_{date}_{time}_{source}",
                "2026-08-27",
                "12-34-56",
                "display-1",
                1
            ),
            "Clip_2026-08-27_12-34-56_display-1"
        );
        assert_eq!(
            render_file_name(
                "Clip_{date}_{time}_{source}",
                "2026-08-27",
                "12-34-56",
                "display-1",
                2
            ),
            "Clip_2026-08-27_12-34-56_display-1_1"
        );
        assert_eq!(
            render_file_name("Clip_{index}", "2026-08-27", "12-34-56", "display-1", 4),
            "Clip_4"
        );
    }

    #[test]
    fn sanitizes_windows_file_name_rules() {
        assert_eq!(sanitize_file_name(r#"bad:/\name"#), "bad___name");
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("clip. "), "clip");
        assert_eq!(sanitize_file_name(""), "Clip");
    }

    #[test]
    fn next_clip_path_consecutive_calls_never_collide() {
        let dir = TempDir::new("collision-test").expect("temp dir");
        let factory: DepsFactory = Box::new(|| {
            (
                Box::new(ScriptedVideoCapture::new(Vec::new())),
                Box::new(MockVideoEncoder::new()),
                Box::new(MockMuxer::default()),
            )
        });
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let mut controller = Controller::with_settings(
            factory,
            sink,
            ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                organize_by_game: true,
                ..ControllerSettings::default()
            },
        );
        let attr = ClipAttribution {
            game_name: Some("The Finals".to_string()),
        };
        let path1 = controller.next_clip_path(Some(&attr));
        let path2 = controller.next_clip_path(Some(&attr));
        assert_ne!(path1, path2);
    }

    #[test]
    fn renders_attributed_clip_path_under_sanitized_game_folder() {
        let dir = TempDir::new("attributed-path").expect("temp dir");
        let settings = ControllerSettings {
            output_dir: dir.path().to_path_buf(),
            organize_by_game: true,
            source_name: "display-1".to_string(),
            ..ControllerSettings::default()
        };
        let attribution = ClipAttribution {
            game_name: Some("Game/Name".to_string()),
        };

        assert_eq!(
            render_clip_path(&settings, Some(&attribution), "2026-08-27", "12-34-56", 1,),
            dir.path()
                .join("Game_Name")
                .join("Clip_2026-08-27_12-34-56_display-1.mp4")
        );
    }

    #[test]
    fn missing_or_blank_attribution_uses_uncategorized_folder() {
        let dir = TempDir::new("unknown-attribution").expect("temp dir");
        let settings = ControllerSettings {
            output_dir: dir.path().to_path_buf(),
            file_name_pattern: "clip".to_string(),
            organize_by_game: true,
            ..ControllerSettings::default()
        };
        let blank = ClipAttribution {
            game_name: Some("  \t".to_string()),
        };
        let expected = dir.path().join("Uncategorized").join("clip.mp4");

        assert_eq!(
            render_clip_path(&settings, None, "date", "time", 1),
            expected
        );
        assert_eq!(
            render_clip_path(&settings, Some(&blank), "date", "time", 1),
            expected
        );
    }

    #[test]
    fn sanitizes_and_caps_game_folder_component() {
        let attribution = |game_name: &str| ClipAttribution {
            game_name: Some(game_name.to_string()),
        };

        assert_eq!(
            sanitize_game_folder(Some(&attribution(r#"bad:/\name"#))),
            "bad___name"
        );
        assert_eq!(sanitize_game_folder(Some(&attribution("CON"))), "_CON");
        assert_eq!(
            sanitize_game_folder(Some(&attribution("..."))),
            "Uncategorized"
        );
        assert_eq!(
            sanitize_game_folder(Some(&attribution(""))),
            "Uncategorized"
        );

        let long_name = attribution(&"a".repeat(MAX_GAME_FOLDER_LENGTH + 10));
        let capped = sanitize_game_folder(Some(&long_name));
        assert_eq!(capped.len(), MAX_GAME_FOLDER_LENGTH);
        assert_eq!(capped, "a".repeat(MAX_GAME_FOLDER_LENGTH));
    }

    #[test]
    fn test_clean_game_name_normalization() {
        assert_eq!(clean_game_name("VALORANT-Win64-Shipping", None), "Valorant");
        assert_eq!(clean_game_name("Discovery-D", None), "The Finals");
        assert_eq!(
            clean_game_name("Discovery", Some("THE FINALS")),
            "The Finals"
        );
        assert_eq!(clean_game_name("r5apex", None), "Apex Legends");
        assert_eq!(
            clean_game_name("FortniteClient-Win64-Shipping", None),
            "Fortnite"
        );
        assert_eq!(clean_game_name("cs2", None), "Counter-Strike 2");
        assert_eq!(clean_game_name("explorer", None), "Desktop");
        assert_eq!(
            clean_game_name("my_custom_game_x64", None),
            "My Custom Game"
        );
        assert_eq!(clean_game_name("UnrealGame-Shipping", None), "UnrealGame");
        assert_eq!(clean_game_name("", None), "Uncategorized");
    }

    #[test]
    fn removes_staged_files_without_touching_completed_clips() {
        let dir = TempDir::new("staged-cleanup").expect("temp dir");
        std::fs::write(dir.path().join("old.mp4.part"), b"partial").expect("partial");
        std::fs::write(dir.path().join("old.mp4.part.lock"), b"lock").expect("lock");
        std::fs::write(dir.path().join("keep.mp4"), b"complete").expect("complete");

        assert_eq!(cleanup_staged_files(dir.path(), false).expect("cleanup"), 2);
        assert!(!dir.path().join("old.mp4.part").exists());
        assert!(!dir.path().join("old.mp4.part.lock").exists());
        assert!(dir.path().join("keep.mp4").exists());
    }

    #[test]
    fn removes_nested_staged_files_without_recursing_deeper() {
        let dir = TempDir::new("nested-staged-cleanup").expect("temp dir");
        let game = dir.path().join("Game");
        let deeper = game.join("nested");
        std::fs::create_dir_all(&deeper).expect("directories");
        let root_partial = dir.path().join("root.mp4.part");
        let nested_partial = game.join("nested.mp4.part");
        let deep_partial = deeper.join("deep.mp4.part");
        for path in [&root_partial, &nested_partial, &deep_partial] {
            std::fs::write(path, b"partial").expect("partial");
        }

        assert_eq!(cleanup_staged_files(dir.path(), true).expect("cleanup"), 2);
        assert!(!root_partial.exists());
        assert!(!nested_partial.exists());
        assert!(deep_partial.exists());
    }

    #[test]
    fn audio_factory_receives_current_ordered_enabled_plans_on_each_start() {
        let dir = TempDir::new("audio-plans").expect("temp dir");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let video_factory: DepsFactory = Box::new(|| {
            (
                Box::new(ScriptedVideoCapture::new(Vec::new()).with_continuous_frames())
                    as Box<dyn capture_api::VideoCapture>,
                Box::new(MockVideoEncoder::new()) as Box<dyn encoder_api::VideoEncoder>,
                Box::new(MockMuxer::default()) as Box<dyn muxer::Muxer>,
            )
        });
        let calls_clone = Arc::clone(&calls);
        let audio_factory = Box::new(move |plans: &[AudioTrackPlan]| {
            calls_clone.lock().expect("calls lock").push(plans.to_vec());
            Vec::new()
        });
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let initial_tracks = vec![
            track("disabled", false, None),
            track("first", true, Some("endpoint-1")),
            track("second", true, None),
        ];
        let expected_initial = initial_tracks[1..].to_vec();
        let initial = ControllerSettings {
            output_dir: dir.path().to_path_buf(),
            audio_tracks: initial_tracks,
            ..ControllerSettings::default()
        };
        let mut controller =
            Controller::with_audio_factory(video_factory, audio_factory, sink, initial);

        controller.start();
        assert_eq!(controller.state(), RecorderState::Buffering);
        controller.stop();

        let changed = ControllerSettings {
            output_dir: dir.path().to_path_buf(),
            audio_tracks: vec![track("replacement", true, Some("endpoint-9"))],
            ..ControllerSettings::default()
        };
        controller
            .update_settings(changed)
            .expect("updated controller settings");
        controller.start();
        controller.stop();

        let calls = calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], expected_initial);
        assert_eq!(
            calls[1],
            vec![track("replacement", true, Some("endpoint-9"))]
        );
    }

    #[test]
    fn controller_rejects_non_finite_or_out_of_range_audio_gain() {
        for gain in [f32::NEG_INFINITY, -0.01, 8.01, f32::INFINITY, f32::NAN] {
            let mut invalid = track("invalid", true, None);
            invalid.gain = gain;
            let error = validate_audio_track_count(&[invalid]).expect_err("invalid gain");
            assert!(error.contains("gain"), "unexpected error: {error}");
        }

        for gain in [0.0, 8.0] {
            let mut valid = track("valid", true, None);
            valid.gain = gain;
            validate_audio_track_count(&[valid]).expect("boundary gain is valid");
        }
    }

    #[test]
    fn factory_inputs_are_passed_to_engine_without_label_selection() {
        let dir = TempDir::new("audio-labels").expect("temp dir");
        let video_factory: DepsFactory = Box::new(|| {
            (
                Box::new(ScriptedVideoCapture::new(Vec::new()).with_continuous_frames())
                    as Box<dyn capture_api::VideoCapture>,
                Box::new(MockVideoEncoder::new()) as Box<dyn encoder_api::VideoEncoder>,
                Box::new(MockMuxer::default()) as Box<dyn muxer::Muxer>,
            )
        });
        let audio_factory = Box::new(|_plans: &[AudioTrackPlan]| {
            vec![recorder_engine::AudioInput::new(
                StreamId(42),
                "custom-label",
                Box::new(ScriptedAudioCapture::new(vec![
                    test_support::AudioStep::Chunks(1),
                ])),
                Box::new(MockAudioEncoder::default()),
            )]
        });
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let settings = ControllerSettings {
            output_dir: dir.path().to_path_buf(),
            ..ControllerSettings::default()
        };
        let mut controller =
            Controller::with_audio_factory(video_factory, audio_factory, sink, settings);

        controller.start();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while controller.audio_stream_count() == 0 && std::time::Instant::now() < deadline {
            controller.poll();
            std::thread::yield_now();
        }
        assert_eq!(controller.audio_stream_count(), 1);
        controller.stop();
    }

    #[test]
    fn early_save_replay_rejection_includes_buffer_metrics_in_reason() {
        let dir = TempDir::new("early-save-rejection").expect("temp dir");
        let factory: DepsFactory = Box::new(|| {
            (
                Box::new(ScriptedVideoCapture::new(Vec::new()))
                    as Box<dyn capture_api::VideoCapture>,
                Box::new(MockVideoEncoder::new()) as Box<dyn encoder_api::VideoEncoder>,
                Box::new(MockMuxer::default()) as Box<dyn muxer::Muxer>,
            )
        });
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let settings = ControllerSettings {
            output_dir: dir.path().to_path_buf(),
            retention_ms: 10_000,
            ..ControllerSettings::default()
        };
        let mut controller = Controller::with_settings(factory, sink, settings);

        controller.start();
        assert_eq!(controller.state(), RecorderState::Buffering);

        let events = controller.execute(&Command::SaveReplay);
        assert!(events.iter().any(|event| {
            matches!(
                event,
                ControllerEvent::CommandRejected {
                    command,
                    reason,
                } if command == "save_replay"
                    && reason == "buffer not ready yet (state 'Buffering'; buffered 0/10000 ms across 0 packets)"
            )
        }));
        controller.stop();
    }

    #[test]
    fn pipeline_fidelity_capabilities_reporting() {
        let caps = pipeline_fidelity_capabilities();
        assert_eq!(caps.len(), 3);

        assert_eq!(caps[0].mode, VideoFidelityMode::Standard);
        assert_eq!(caps[0].availability, FidelityAvailability::Available);
        assert_eq!(caps[0].issue, None);

        assert_eq!(caps[1].mode, VideoFidelityMode::Clarity);
        assert_eq!(caps[1].availability, FidelityAvailability::RuntimeChecked);
        assert_eq!(caps[1].issue, None);

        assert_eq!(caps[2].mode, VideoFidelityMode::Archival444);
        assert_eq!(caps[2].availability, FidelityAvailability::Unsupported);
        assert_eq!(
            caps[2].issue,
            Some(FidelityIssue {
                kind: FidelityIssueKind::Fallback,
                code: ARCHIVAL_444_UNSUPPORTED_CODE.to_string(),
                message: ARCHIVAL_444_UNSUPPORTED_MESSAGE.to_string(),
            })
        );
    }

    #[test]
    fn fidelity_status_stopped_state() {
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let factory: DepsFactory = Box::new(|| {
            (
                Box::new(ScriptedVideoCapture::new(Vec::new())),
                Box::new(MockVideoEncoder::new()),
                Box::new(MockMuxer::default()),
            )
        });
        let mut controller = Controller::with_settings(
            factory,
            sink,
            ControllerSettings {
                fidelity_mode: VideoFidelityMode::Standard,
                ..ControllerSettings::default()
            },
        );

        let status = controller.fidelity_status();
        assert_eq!(status.requested, VideoFidelityMode::Standard);
        assert_eq!(status.active, None);
        assert_eq!(status.issue, None);

        controller
            .update_settings(ControllerSettings {
                fidelity_mode: VideoFidelityMode::Clarity,
                ..ControllerSettings::default()
            })
            .expect("update settings");
        let status = controller.fidelity_status();
        assert_eq!(status.requested, VideoFidelityMode::Clarity);
        assert_eq!(status.active, None);
        assert_eq!(status.issue, None);

        controller
            .update_settings(ControllerSettings {
                fidelity_mode: VideoFidelityMode::Archival444,
                ..ControllerSettings::default()
            })
            .expect("update settings");
        let status = controller.fidelity_status();
        assert_eq!(status.requested, VideoFidelityMode::Archival444);
        assert_eq!(status.active, None);
        assert_eq!(
            status.issue,
            Some(FidelityIssue {
                kind: FidelityIssueKind::Fallback,
                code: ARCHIVAL_444_UNSUPPORTED_CODE.to_string(),
                message: ARCHIVAL_444_UNSUPPORTED_MESSAGE.to_string(),
            })
        );
    }

    #[test]
    fn fidelity_standard_start_success() {
        let dir = TempDir::new("fidelity-standard").expect("temp");
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let factory: DepsFactory = Box::new(|| {
            (
                Box::new(ScriptedVideoCapture::new(Vec::new()).with_continuous_frames()),
                Box::new(MockVideoEncoder::new()),
                Box::new(MockMuxer::default()),
            )
        });
        let mut controller = Controller::with_settings(
            factory,
            sink,
            ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Standard,
                ..ControllerSettings::default()
            },
        );

        let events = controller.start();
        assert_eq!(controller.state(), RecorderState::Buffering);
        assert!(!events
            .iter()
            .any(|e| matches!(e, ControllerEvent::Warning { .. })));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while controller.fidelity_status().active.is_none() && std::time::Instant::now() < deadline
        {
            controller.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let status = controller.fidelity_status();
        assert_eq!(status.requested, VideoFidelityMode::Standard);
        assert_eq!(status.active, Some(VideoFidelityMode::Standard));
        assert_eq!(status.issue, None);

        controller.stop();
        let status_stopped = controller.fidelity_status();
        assert_eq!(status_stopped.requested, VideoFidelityMode::Standard);
        assert_eq!(status_stopped.active, None);
        assert_eq!(status_stopped.issue, None);
    }

    #[test]
    fn fidelity_clarity_start_warning_and_fallback_status() {
        let dir = TempDir::new("fidelity-clarity-fallback").expect("temp");
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let factory: DepsFactory = Box::new(|| {
            (
                Box::new(ScriptedVideoCapture::new(Vec::new()).with_continuous_frames()),
                Box::new(MockVideoEncoder::new()),
                Box::new(MockMuxer::default()),
            )
        });
        let mut controller = Controller::with_settings(
            factory,
            sink,
            ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Clarity,
                ..ControllerSettings::default()
            },
        );

        let _events = controller.start();
        assert_eq!(controller.state(), RecorderState::Buffering);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while controller.fidelity_status().active.is_none() && std::time::Instant::now() < deadline
        {
            controller.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let status = controller.fidelity_status();
        assert_eq!(status.requested, VideoFidelityMode::Clarity);
        assert_eq!(status.active, Some(VideoFidelityMode::Standard));
        assert_eq!(
            status.issue,
            Some(FidelityIssue {
                kind: FidelityIssueKind::Fallback,
                code: CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE.to_string(),
                message: CLARITY_GPU_CONTEXT_UNAVAILABLE_MESSAGE.to_string(),
            })
        );

        controller.stop();
        let status_stopped = controller.fidelity_status();
        assert_eq!(status_stopped.requested, VideoFidelityMode::Clarity);
        assert_eq!(status_stopped.active, None);
        assert_eq!(status_stopped.issue, None);
    }

    #[test]
    fn fidelity_clarity_with_mock_gpu_context_reports_active_clarity() {
        let dir = TempDir::new("fidelity-clarity-gpu").expect("temp");
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let factory: DepsFactory = Box::new(|| {
            let gpu_ctx = encoder_api::GpuFrameContext::new(Arc::new(TestGpuResolver));
            (
                Box::new(
                    ScriptedVideoCapture::new(Vec::new())
                        .with_continuous_frames()
                        .with_gpu_context(gpu_ctx),
                ),
                Box::new(MockVideoEncoder::new()),
                Box::new(MockMuxer::default()),
            )
        });
        let mut controller = Controller::with_settings(
            factory,
            sink,
            ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Clarity,
                ..ControllerSettings::default()
            },
        );

        controller.start();
        assert_eq!(controller.state(), RecorderState::Buffering);

        // Wait until at least 1 frame is processed so the pending probe resolves
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while controller.buffered_packet_count() == 0 && std::time::Instant::now() < deadline {
            controller.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let status = controller.fidelity_status();
        assert_eq!(status.requested, VideoFidelityMode::Clarity);
        assert_eq!(status.active, Some(VideoFidelityMode::Clarity));
        assert_eq!(status.issue, None);

        controller.stop();
        let status_stopped = controller.fidelity_status();
        assert_eq!(status_stopped.requested, VideoFidelityMode::Clarity);
        assert_eq!(status_stopped.active, None);
        assert_eq!(status_stopped.issue, None);
    }

    #[test]
    fn fidelity_archival_start_hard_rejected_before_side_effects() {
        let dir = TempDir::new("fidelity-archival").expect("temp");
        let staged = dir.path().join("staged.mp4.part");
        std::fs::write(&staged, b"staged").expect("write staged");

        let factory_calls = Arc::new(Mutex::new(0));
        let factory_calls_clone = Arc::clone(&factory_calls);
        let factory: DepsFactory = Box::new(move || {
            *factory_calls_clone.lock().unwrap() += 1;
            (
                Box::new(ScriptedVideoCapture::new(Vec::new()).with_continuous_frames()),
                Box::new(MockVideoEncoder::new()),
                Box::new(MockMuxer::default()),
            )
        });
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let mut controller = Controller::with_settings(
            factory,
            sink,
            ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Archival444,
                ..ControllerSettings::default()
            },
        );

        let events = controller.start();
        assert_eq!(controller.state(), RecorderState::Stopped);
        assert_eq!(*factory_calls.lock().unwrap(), 1);
        // Staged file should NOT have been touched because archival start rejects before cleanup
        assert!(staged.exists());

        assert!(events.iter().any(|e| matches!(
            e,
            ControllerEvent::CommandRejected { command, reason }
                if command == "start_capture" && reason == ARCHIVAL_444_UNSUPPORTED_MESSAGE
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            ControllerEvent::Warning { code, message }
                if code == ARCHIVAL_444_UNSUPPORTED_CODE && message == ARCHIVAL_444_UNSUPPORTED_MESSAGE
        )));

        let status = controller.fidelity_status();
        assert_eq!(status.requested, VideoFidelityMode::Archival444);
        assert_eq!(status.active, None);
        assert_eq!(
            status.issue,
            Some(FidelityIssue {
                kind: FidelityIssueKind::Fallback,
                code: ARCHIVAL_444_UNSUPPORTED_CODE.to_string(),
                message: ARCHIVAL_444_UNSUPPORTED_MESSAGE.to_string(),
            })
        );
    }

    #[test]
    fn fidelity_archival_start_success_when_ayuv_supported() {
        let dir = TempDir::new("fidelity-archival-supported").expect("temp");
        let encoder_configs = Arc::new(Mutex::new(Vec::new()));
        let encoder_configs_clone = Arc::clone(&encoder_configs);
        let factory: DepsFactory = Box::new(move || {
            let mut encoder = MockVideoEncoder::new();
            encoder.supported_pixel_formats = Some(vec![PixelFormat::Nv12, PixelFormat::Ayuv]);
            encoder.configured_configs = Arc::clone(&encoder_configs_clone);
            (
                Box::new(ScriptedVideoCapture::new(Vec::new()).with_continuous_frames()),
                Box::new(encoder),
                Box::new(MockMuxer::default()),
            )
        });
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let mut controller = Controller::with_settings(
            factory,
            sink,
            ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Archival444,
                ..ControllerSettings::default()
            },
        );

        let events = controller.start();
        assert_eq!(controller.state(), RecorderState::Buffering);
        assert!(!events
            .iter()
            .any(|e| matches!(e, ControllerEvent::CommandRejected { .. })));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while controller.fidelity_status().active.is_none() && std::time::Instant::now() < deadline
        {
            controller.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let status = controller.fidelity_status();
        assert_eq!(status.requested, VideoFidelityMode::Archival444);
        assert_eq!(status.active, Some(VideoFidelityMode::Archival444));
        assert_eq!(status.issue, None);

        let configs = encoder_configs.lock().unwrap();
        assert!(!configs.is_empty());
        assert_eq!(configs[0].pixel_format, PixelFormat::Ayuv);

        controller.stop();
    }

    #[test]
    fn fidelity_status_on_failed_start() {
        struct FailingVideoCapture;
        impl VideoCapture for FailingVideoCapture {
            fn enumerate_sources(&self) -> capture_api::Result<Vec<media_types::VideoSourceInfo>> {
                Ok(Vec::new())
            }
            fn start(&mut self, _config: VideoCaptureConfig) -> capture_api::Result<()> {
                Err(VideoCaptureError::SourceUnavailable)
            }
            fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
                Err(VideoCaptureError::EndOfStream)
            }
            fn stop(&mut self) -> capture_api::Result<()> {
                Ok(())
            }
        }

        let dir = TempDir::new("fidelity-failed-start").expect("temp");
        let factory: DepsFactory = Box::new(|| {
            (
                Box::new(FailingVideoCapture),
                Box::new(MockVideoEncoder::new()),
                Box::new(MockMuxer::default()),
            )
        });
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let mut controller = Controller::with_settings(
            factory,
            sink,
            ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Clarity,
                ..ControllerSettings::default()
            },
        );

        controller.start();
        assert_eq!(controller.state(), RecorderState::Stopped);
        let status = controller.fidelity_status();
        assert_eq!(status.requested, VideoFidelityMode::Clarity);
        assert_eq!(status.active, None);
    }

    #[test]
    fn fidelity_live_update_settings_preserves_session_fidelity_until_stop() {
        let dir = TempDir::new("fidelity-live-update").expect("temp");
        let sink = Arc::new(super::super::notification::CollectingNotificationSink::new());
        let factory: DepsFactory = Box::new(|| {
            (
                Box::new(ScriptedVideoCapture::new(Vec::new()).with_continuous_frames()),
                Box::new(MockVideoEncoder::new()),
                Box::new(MockMuxer::default()),
            )
        });
        let mut controller = Controller::with_settings(
            factory,
            sink,
            ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Clarity,
                ..ControllerSettings::default()
            },
        );

        controller.start();
        assert_eq!(controller.state(), RecorderState::Buffering);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while controller.fidelity_status().active.is_none() && std::time::Instant::now() < deadline
        {
            controller.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let initial_status = controller.fidelity_status();
        assert_eq!(initial_status.requested, VideoFidelityMode::Clarity);
        assert_eq!(initial_status.active, Some(VideoFidelityMode::Standard));
        assert_eq!(
            initial_status.issue,
            Some(FidelityIssue {
                kind: FidelityIssueKind::Fallback,
                code: CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE.to_string(),
                message: CLARITY_GPU_CONTEXT_UNAVAILABLE_MESSAGE.to_string(),
            })
        );

        // Update settings while session is active to Archival444
        controller
            .update_settings(ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Archival444,
                ..ControllerSettings::default()
            })
            .expect("update settings during active capture");

        // The running session's status must remain unchanged
        let live_status = controller.fidelity_status();
        assert_eq!(live_status.requested, VideoFidelityMode::Clarity);
        assert_eq!(live_status.active, Some(VideoFidelityMode::Standard));
        assert_eq!(
            live_status.issue,
            Some(FidelityIssue {
                kind: FidelityIssueKind::Fallback,
                code: CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE.to_string(),
                message: CLARITY_GPU_CONTEXT_UNAVAILABLE_MESSAGE.to_string(),
            })
        );

        // Stop resets active session; now stopped status reflects the newly staged Archival444
        controller.stop();
        let stopped_status = controller.fidelity_status();
        assert_eq!(stopped_status.requested, VideoFidelityMode::Archival444);
        assert_eq!(stopped_status.active, None);
        assert_eq!(
            stopped_status.issue,
            Some(FidelityIssue {
                kind: FidelityIssueKind::Fallback,
                code: ARCHIVAL_444_UNSUPPORTED_CODE.to_string(),
                message: ARCHIVAL_444_UNSUPPORTED_MESSAGE.to_string(),
            })
        );

        // Update settings while stopped to Standard
        controller
            .update_settings(ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Standard,
                ..ControllerSettings::default()
            })
            .expect("update settings while stopped");
        let stopped_standard = controller.fidelity_status();
        assert_eq!(stopped_standard.requested, VideoFidelityMode::Standard);
        assert_eq!(stopped_standard.active, None);
        assert_eq!(stopped_standard.issue, None);

        // Start Standard session and update to Clarity while running
        controller.start();
        assert_eq!(controller.state(), RecorderState::Buffering);

        let deadline2 = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while controller.fidelity_status().active.is_none() && std::time::Instant::now() < deadline2
        {
            controller.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let standard_status = controller.fidelity_status();
        assert_eq!(standard_status.requested, VideoFidelityMode::Standard);
        assert_eq!(standard_status.active, Some(VideoFidelityMode::Standard));
        assert_eq!(standard_status.issue, None);

        controller
            .update_settings(ControllerSettings {
                output_dir: dir.path().to_path_buf(),
                fidelity_mode: VideoFidelityMode::Clarity,
                ..ControllerSettings::default()
            })
            .expect("update settings to clarity while running");
        let standard_live_status = controller.fidelity_status();
        assert_eq!(standard_live_status.requested, VideoFidelityMode::Standard);
        assert_eq!(
            standard_live_status.active,
            Some(VideoFidelityMode::Standard)
        );
        assert_eq!(standard_live_status.issue, None);

        controller.stop();
        let stopped_clarity = controller.fidelity_status();
        assert_eq!(stopped_clarity.requested, VideoFidelityMode::Clarity);
        assert_eq!(stopped_clarity.active, None);
        assert_eq!(stopped_clarity.issue, None);
    }

    #[test]
    fn fidelity_serialization_contracts() {
        let issue = FidelityIssue {
            kind: FidelityIssueKind::Fallback,
            code: CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE.to_string(),
            message: CLARITY_GPU_CONTEXT_UNAVAILABLE_MESSAGE.to_string(),
        };
        let status = FidelityStatus {
            requested: VideoFidelityMode::Clarity,
            active: Some(VideoFidelityMode::Standard),
            issue: Some(issue),
        };
        let val = serde_json::to_value(&status).expect("serialize fidelity status");
        assert_eq!(val["requested"], "clarity");
        assert_eq!(val["active"], "standard");
        assert_eq!(val["issue"]["kind"], "fallback");
        assert_eq!(val["issue"]["code"], CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE);

        let cap = FidelityCapability {
            mode: VideoFidelityMode::Clarity,
            availability: FidelityAvailability::RuntimeChecked,
            issue: None,
        };
        let cap_val = serde_json::to_value(&cap).expect("serialize capability");
        assert_eq!(cap_val["mode"], "clarity");
        assert_eq!(cap_val["availability"], "runtime_checked");
        assert_eq!(cap_val["issue"], serde_json::Value::Null);

        let cap_archival = FidelityCapability {
            mode: VideoFidelityMode::Archival444,
            availability: FidelityAvailability::Unsupported,
            issue: Some(FidelityIssue {
                kind: FidelityIssueKind::Fallback,
                code: ARCHIVAL_444_UNSUPPORTED_CODE.to_string(),
                message: ARCHIVAL_444_UNSUPPORTED_MESSAGE.to_string(),
            }),
        };
        let cap_archival_val = serde_json::to_value(&cap_archival).expect("serialize capability");
        assert_eq!(cap_archival_val["mode"], "archival_444");
        assert_eq!(cap_archival_val["availability"], "unsupported");
        assert_eq!(cap_archival_val["issue"]["kind"], "fallback");
    }
}
