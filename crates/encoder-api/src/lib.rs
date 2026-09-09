//! Encoder abstractions (spec §13.6).
//!
//! Backend-specific types must not escape backend crates. Capability ranking,
//! fallback, epochs, and metrics use this crate's backend-neutral contracts.
//! GPU resources are carried through the type-erased resolver below so this
//! crate does not depend on D3D11 or a particular encoder API.

use std::any::Any;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use media_types::{
    AudioFrame, EncodedPacket, FramePayload, GpuFrameHandle, PacketPayload, PixelFormat, VideoFrame,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T, E = EncoderError> = std::result::Result<T, E>;

pub const DEFAULT_KEYFRAME_INTERVAL_SECONDS: f64 = 2.0;
const LATENCY_HISTORY_LIMIT: usize = 256;

/// A native GPU resource owned by the platform capture backend.
///
/// The concrete resource stays in the backend crate. An encoder backend may
/// downcast this value only to a resource type it explicitly supports. The
/// resource is kept alive by [`GpuFrameLease`] for the duration of one encode
/// operation; no CPU readback is implied by this abstraction.
pub type GpuFrameResource = dyn Any + Send + Sync;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GpuFrameResolveError {
    #[error("video frame does not contain a GPU handle")]
    NotGpuFrame,

    #[error("GPU frame handle {handle:?} is stale or unknown")]
    StaleHandle { handle: GpuFrameHandle },

    #[error("GPU frame context is unavailable: {details}")]
    ContextUnavailable { details: String },

    #[error("GPU frame resource is incompatible: {details}")]
    Incompatible { details: String },
}

/// Resolves a capture-owned opaque handle into a retained native resource.
///
/// Implementations must not block on disk or UI work. A stale handle is a
/// normal result when a bounded capture staging ring has already evicted the
/// resource; the encoder decides whether to drop that frame or enter its
/// fallback path.
pub trait GpuFrameResolver: Send + Sync {
    fn resolve(
        &self,
        handle: GpuFrameHandle,
    ) -> std::result::Result<Arc<GpuFrameResource>, GpuFrameResolveError>;

    /// Optional session-scoped resource such as the D3D device that owns all
    /// resolved textures. Encoders may use it to configure a native GPU path
    /// before the first frame arrives.
    fn session_resource(&self) -> Option<Arc<GpuFrameResource>> {
        None
    }
}

/// Encoder-side access to the resolver for one capture session.
///
/// The context is installed before the first GPU frame is encoded and is
/// cleared after the encoder drains. Cloning the context clones only the
/// resolver reference; each lease retains its resource independently.
#[derive(Clone)]
pub struct GpuFrameContext {
    resolver: Arc<dyn GpuFrameResolver>,
}

impl GpuFrameContext {
    pub fn new(resolver: Arc<dyn GpuFrameResolver>) -> Self {
        Self { resolver }
    }

    /// Resolve the GPU payload carried by a video frame.
    pub fn resolve(
        &self,
        frame: &VideoFrame,
    ) -> std::result::Result<GpuFrameLease, GpuFrameResolveError> {
        let handle = match &frame.payload {
            FramePayload::Gpu(handle) => *handle,
            FramePayload::Cpu(_) => return Err(GpuFrameResolveError::NotGpuFrame),
        };
        self.resolve_handle(handle)
    }

    /// Resolve a handle directly when the encoder has already validated the
    /// frame metadata.
    pub fn resolve_handle(
        &self,
        handle: GpuFrameHandle,
    ) -> std::result::Result<GpuFrameLease, GpuFrameResolveError> {
        let resource = self.resolver.resolve(handle)?;
        Ok(GpuFrameLease { handle, resource })
    }

    /// Return an optional session-scoped GPU resource supplied by the capture
    /// backend, such as the device/context used by all frame textures.
    pub fn session_resource(&self) -> Option<Arc<GpuFrameResource>> {
        self.resolver.session_resource()
    }
}

/// A resolved resource lease. Keeping this value alive keeps the native
/// texture/resource alive even if the capture staging ring evicts its handle.
#[derive(Clone)]
pub struct GpuFrameLease {
    handle: GpuFrameHandle,
    resource: Arc<GpuFrameResource>,
}

impl GpuFrameLease {
    pub fn handle(&self) -> GpuFrameHandle {
        self.handle
    }

    pub fn resource(&self) -> &GpuFrameResource {
        &self.resource
    }

    /// Convenience for a backend adapter that supports a concrete native
    /// resource type.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.resource.downcast_ref::<T>()
    }
}

#[derive(Debug, Error)]
pub enum EncoderError {
    #[error("{backend} encoder initialization failed: {details}")]
    InitializationFailed { backend: String, details: String },

    #[error("{backend} encoder overloaded")]
    Overloaded { backend: String },

    #[error("encoder used before configuration")]
    NotConfigured,

    #[error("unsupported encoder configuration: {reason}")]
    UnsupportedConfiguration { reason: String },

    #[error("encode failed: {details}")]
    EncodeFailed { details: String },
}

impl EncoderError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InitializationFailed { .. } => "ENCODER_INITIALIZATION_FAILED",
            Self::Overloaded { .. } => "ENCODER_OVERLOADED",
            Self::NotConfigured | Self::UnsupportedConfiguration { .. } => "ENCODER_NOT_AVAILABLE",
            Self::EncodeFailed { .. } => "ENCODER_ENCODE_FAILED",
        }
    }
}

pub fn normalize_codec(codec: &str) -> &str {
    if codec.eq_ignore_ascii_case("avc")
        || codec.eq_ignore_ascii_case("h264")
        || codec.eq_ignore_ascii_case("avc1")
    {
        "avc"
    } else if codec.eq_ignore_ascii_case("hevc")
        || codec.eq_ignore_ascii_case("h265")
        || codec.eq_ignore_ascii_case("hvc1")
    {
        "hevc"
    } else if codec.eq_ignore_ascii_case("av1") || codec.eq_ignore_ascii_case("av01") {
        "av1"
    } else {
        codec
    }
}

pub fn is_same_codec(a: &str, b: &str) -> bool {
    let norm_a = normalize_codec(a);
    let norm_b = normalize_codec(b);
    norm_a.eq_ignore_ascii_case(norm_b)
}

fn default_supported_pixel_formats() -> Vec<PixelFormat> {
    vec![PixelFormat::Nv12]
}

/// What a backend reports about itself before configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncoderCapabilities {
    pub backend_name: String,
    pub codec: String,
    pub hardware_accelerated: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub supported_fps: Vec<u32>,
    #[serde(default = "default_supported_pixel_formats")]
    pub supported_pixel_formats: Vec<PixelFormat>,
}

impl EncoderCapabilities {
    /// Whether this backend can accept the requested stream dimensions and
    /// frame rate. A successful trial configure remains authoritative because
    /// driver-reported limits are not always complete.
    pub fn supports(&self, config: &VideoEncoderConfig) -> bool {
        is_same_codec(&self.codec, &config.codec)
            && self.max_width >= config.width
            && self.max_height >= config.height
            && self.supported_fps.contains(&config.fps)
            && self.supported_pixel_formats.contains(&config.pixel_format)
    }

    /// Infer a vendor from the backend's diagnostic name. Native discovery
    /// should include a vendor name when the operating system exposes one;
    /// generic hardware is still ranked ahead of software when it does not.
    pub fn vendor(&self) -> EncoderVendor {
        EncoderVendor::from_backend_name(&self.backend_name, self.hardware_accelerated)
    }
}

/// Hardware family used only for deterministic ranking and diagnostics. It is
/// intentionally not a promise that a particular GPU is present or usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderVendor {
    Nvidia,
    Amd,
    Intel,
    GenericHardware,
    Software,
    Unknown,
}

impl EncoderVendor {
    fn from_backend_name(name: &str, hardware_accelerated: bool) -> Self {
        let name = name.to_ascii_lowercase();
        if name.contains("nvidia") || name.contains("nvenc") {
            return Self::Nvidia;
        }
        if name.contains("amd") || name.contains("amf") || name.contains("radeon") {
            return Self::Amd;
        }
        if name.contains("intel") || name.contains("qsv") {
            return Self::Intel;
        }
        if hardware_accelerated {
            Self::GenericHardware
        } else if name.contains("software") || name.contains("sw") {
            Self::Software
        } else {
            Self::Unknown
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Nvidia => 0,
            Self::Amd => 1,
            Self::Intel => 2,
            Self::GenericHardware => 3,
            Self::Unknown => 4,
            Self::Software => 5,
        }
    }
}

/// User-level policy applied during backend selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderPreference {
    #[default]
    Auto,
    HardwareOnly,
    SoftwareOnly,
}

/// A discovered backend and its opaque factory identifier. The identifier is
/// never interpreted outside the factory that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct EncoderCandidate {
    pub id: String,
    pub capabilities: EncoderCapabilities,
}

/// Return compatible candidates in the order in which they should be tried.
/// The sort is stable, so discovery order remains the final tie breaker.
pub fn rank_encoder_candidates(
    candidates: &[EncoderCandidate],
    config: &VideoEncoderConfig,
    preference: EncoderPreference,
) -> Vec<EncoderCandidate> {
    let mut ranked: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            candidate.capabilities.supports(config)
                && match preference {
                    EncoderPreference::Auto => true,
                    EncoderPreference::HardwareOnly => candidate.capabilities.hardware_accelerated,
                    EncoderPreference::SoftwareOnly => !candidate.capabilities.hardware_accelerated,
                }
        })
        .cloned()
        .collect();

    ranked.sort_by(|left, right| {
        let left_hardware = !left.capabilities.hardware_accelerated;
        let right_hardware = !right.capabilities.hardware_accelerated;
        left_hardware
            .cmp(&right_hardware)
            .then_with(|| {
                left.capabilities
                    .vendor()
                    .rank()
                    .cmp(&right.capabilities.vendor().rank())
            })
            .then_with(|| {
                left.capabilities
                    .backend_name
                    .cmp(&right.capabilities.backend_name)
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    ranked
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityPresetSpec {
    /// Higher numbers mean more quality at higher cost; mapping to
    /// backend presets is per-backend.
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoEncoderConfig {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: Option<u32>,
    pub preset_level: u8,
    /// Keyframe interval in seconds; default 2.0 (ENC-006).
    pub keyframe_interval_seconds: f64,
    pub pixel_format: PixelFormat,
}

impl Default for VideoEncoderConfig {
    fn default() -> Self {
        Self {
            codec: "avc".to_string(),
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_kbps: None,
            preset_level: 2,
            keyframe_interval_seconds: DEFAULT_KEYFRAME_INTERVAL_SECONDS,
            pixel_format: PixelFormat::Nv12,
        }
    }
}

impl VideoEncoderConfig {
    /// Validate settings before a backend is touched. Backend-specific trial
    /// configuration still runs after this common validation.
    pub fn validate(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "video dimensions must be positive".to_string(),
            });
        }
        if !self.width.is_multiple_of(2) || !self.height.is_multiple_of(2) {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "video output dimensions must be even".to_string(),
            });
        }
        if !matches!(self.fps, 30 | 60 | 120) {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: format!("fps must be 30, 60, or 120, got {}", self.fps),
            });
        }
        let normalized = normalize_codec(&self.codec);
        if !matches!(normalized, "avc" | "hevc" | "av1") {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: format!("unsupported video codec '{}'", self.codec),
            });
        }
        if !self.keyframe_interval_seconds.is_finite() || self.keyframe_interval_seconds <= 0.0 {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "keyframe interval must be a finite positive number".to_string(),
            });
        }
        if self.preset_level > 5 {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: format!(
                    "quality preset level must be at most 5, got {}",
                    self.preset_level
                ),
            });
        }
        if self.bitrate_kbps == Some(0) {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "bitrate must be positive when specified".to_string(),
            });
        }
        Ok(())
    }

    /// Number of encoded frames between keyframes, rounded to the nearest
    /// frame and clamped so every valid configuration has a keyframe cadence.
    pub fn keyframe_interval_frames(&self) -> u32 {
        (self.keyframe_interval_seconds * f64::from(self.fps))
            .round()
            .max(1.0) as u32
    }

    /// Convert a quality level into a conservative bitrate heuristic when the user
    /// did not provide an explicit bitrate. The backend may use this as a hint.
    pub fn suggested_bitrate_kbps(&self) -> u32 {
        self.bitrate_kbps.unwrap_or_else(|| {
            let pixels_per_second = u64::from(self.width)
                .saturating_mul(u64::from(self.height))
                .saturating_mul(u64::from(self.fps));
            let base = (pixels_per_second / 10_000).clamp(2_500, 40_000) as u32;
            match self.preset_level {
                0 => base.saturating_mul(2) / 3,
                1 => base.saturating_mul(4) / 5,
                2 => base,
                3 => base.saturating_mul(6) / 5,
                4 => base.saturating_mul(4) / 3,
                _ => base.saturating_mul(3) / 2,
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioEncoderConfig {
    pub bitrate_kbps: u32,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Default for AudioEncoderConfig {
    fn default() -> Self {
        Self {
            bitrate_kbps: 192,
            sample_rate: 48_000,
            channels: 2,
        }
    }
}

impl AudioEncoderConfig {
    pub fn validate(&self) -> Result<()> {
        if self.bitrate_kbps == 0 {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "audio bitrate must be positive".to_string(),
            });
        }
        if self.sample_rate == 0 {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "audio sample rate must be positive".to_string(),
            });
        }
        if self.channels == 0 {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "audio channel count must be positive".to_string(),
            });
        }
        Ok(())
    }
}

/// H.264 video encoding contract. `encode` consumes frames on the
/// dedicated encoding worker thread only (spec §14).
pub trait VideoEncoder: Send {
    fn capabilities(&self) -> EncoderCapabilities;
    fn configure(&mut self, config: VideoEncoderConfig) -> Result<()>;

    /// Provide the capture session's GPU resolver. Software encoders may
    /// ignore this; GPU encoders should reject an incompatible context before
    /// the first call to [`VideoEncoder::encode`].
    fn set_gpu_frame_context(&mut self, _context: Option<GpuFrameContext>) -> Result<()> {
        Ok(())
    }

    fn encode(&mut self, frame: VideoFrame) -> Result<Vec<EncodedPacket>>;
    fn drain(&mut self) -> Result<Vec<EncodedPacket>>;

    /// Codec initialization bytes for the stream descriptor, when the
    /// backend exposes them. Video backends may discover parameter sets only
    /// after the first output packet, so callers should check this while the
    /// session is running.
    fn codec_extradata(&self) -> Option<PacketPayload> {
        None
    }

    /// Health counters are optional for small test backends; production
    /// backends should override this with their bounded telemetry snapshot.
    fn metrics(&self) -> EncoderMetrics {
        EncoderMetrics {
            active_backend: Some(self.capabilities().backend_name),
            ..EncoderMetrics::default()
        }
    }

    /// Codec-parameter changes advance the replay-buffer epoch. Simple
    /// encoders remain in epoch zero.
    fn epoch(&self) -> u64 {
        0
    }

    /// Non-media events such as backend selection and fallback are drained by
    /// the engine on the encoding worker thread.
    fn take_events(&mut self) -> Vec<EncoderEvent> {
        Vec::new()
    }
}

/// AAC audio encoding contract.
pub trait AudioEncoder: Send {
    fn configure(&mut self, config: AudioEncoderConfig) -> Result<()>;
    fn encode(&mut self, frame: AudioFrame) -> Result<Vec<EncodedPacket>>;
    fn drain(&mut self) -> Result<Vec<EncodedPacket>>;

    /// Codec initialization bytes for the stream descriptor, when the codec
    /// requires them. Backends publish this after successful configuration;
    /// the engine copies it into the registered replay-buffer track before
    /// accepting the first packet.
    fn codec_extradata(&self) -> Option<PacketPayload> {
        None
    }
}

/// Snapshot of encoder health counters. Backends must keep their counters
/// bounded and update them on the encoding worker, never on the UI thread.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderMetrics {
    pub frames_submitted: u64,
    pub frames_encoded: u64,
    pub frames_dropped: u64,
    pub encode_failures: u64,
    pub overload_events: u64,
    pub fallback_count: u64,
    pub encode_latency_us_p50: u64,
    pub encode_latency_us_p95: u64,
    pub encode_latency_us_max: u64,
    pub active_backend: Option<String>,
    pub epoch: u64,
}

/// Events emitted by a video encoder so the engine can surface selection and
/// fallback decisions without exposing backend-specific types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncoderEvent {
    BackendSelected {
        backend: String,
        hardware_accelerated: bool,
    },
    Fallback {
        from_backend: String,
        to_backend: String,
        reason: String,
        epoch: u64,
    },
}

/// Factory boundary used by the fallback wrapper. Discovery and construction
/// stay in the backend crate; the wrapper only sees project-level metadata.
pub trait VideoEncoderFactory: Send {
    fn discover(&self) -> Result<Vec<EncoderCandidate>>;
    fn create(&self, candidate: &EncoderCandidate) -> Result<Box<dyn VideoEncoder>>;

    /// Release backend-owned discovery/runtime state after encoders have
    /// been dropped. Platform factories use this to pair thread-local COM or
    /// media-runtime setup with teardown; test factories can leave it empty.
    fn shutdown(&mut self) {}
}

/// Capability-ranked H.264 encoder with controlled runtime fallback.
///
/// A failed backend is discarded before the next candidate is configured.
/// Successful fallback increments the epoch; the recorder engine clears its
/// replay buffer before accepting packets from the new codec configuration.
pub struct FallbackVideoEncoder {
    factory: Box<dyn VideoEncoderFactory>,
    preference: EncoderPreference,
    candidates: Vec<EncoderCandidate>,
    active: Option<Box<dyn VideoEncoder>>,
    active_index: Option<usize>,
    config: Option<VideoEncoderConfig>,
    gpu_context: Option<GpuFrameContext>,
    metrics: EncoderMetrics,
    latency_history_us: VecDeque<u64>,
    events: Vec<EncoderEvent>,
    epoch: u64,
}

type ActivatedEncoder = (usize, Box<dyn VideoEncoder>);
type ActivationResult = (Option<ActivatedEncoder>, Vec<String>);

impl FallbackVideoEncoder {
    pub fn new(factory: Box<dyn VideoEncoderFactory>, preference: EncoderPreference) -> Self {
        Self {
            factory,
            preference,
            candidates: Vec::new(),
            active: None,
            active_index: None,
            config: None,
            gpu_context: None,
            metrics: EncoderMetrics::default(),
            latency_history_us: VecDeque::with_capacity(LATENCY_HISTORY_LIMIT),
            events: Vec::new(),
            epoch: 0,
        }
    }

    pub fn candidates(&self) -> &[EncoderCandidate] {
        &self.candidates
    }

    pub fn active_backend(&self) -> Option<EncoderCapabilities> {
        self.active.as_ref().map(|encoder| encoder.capabilities())
    }

    fn update_latency(&mut self, started: Instant) {
        let elapsed = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        if self.latency_history_us.len() == LATENCY_HISTORY_LIMIT {
            self.latency_history_us.pop_front();
        }
        self.latency_history_us.push_back(elapsed);
        let mut ordered: Vec<_> = self.latency_history_us.iter().copied().collect();
        ordered.sort_unstable();
        let percentile = |numerator: usize, denominator: usize| -> u64 {
            let index = ((ordered.len() * numerator).saturating_sub(1) / denominator)
                .min(ordered.len().saturating_sub(1));
            ordered.get(index).copied().unwrap_or(0)
        };
        self.metrics.encode_latency_us_p50 = percentile(1, 2);
        self.metrics.encode_latency_us_p95 = percentile(19, 20);
        self.metrics.encode_latency_us_max = ordered.last().copied().unwrap_or(0);
    }

    fn configure_candidate(
        &self,
        candidate: &EncoderCandidate,
        config: &VideoEncoderConfig,
    ) -> Result<Box<dyn VideoEncoder>> {
        let mut encoder = self.factory.create(candidate)?;
        if let Some(context) = self.gpu_context.clone() {
            encoder.set_gpu_frame_context(Some(context))?;
        }
        encoder.configure(config.clone())?;
        Ok(encoder)
    }

    fn activate_candidate(
        &self,
        index: usize,
        config: &VideoEncoderConfig,
    ) -> Result<Box<dyn VideoEncoder>> {
        self.configure_candidate(&self.candidates[index], config)
    }

    fn activate_next(&self, start_index: usize, config: &VideoEncoderConfig) -> ActivationResult {
        let mut failures = Vec::new();
        for index in start_index..self.candidates.len() {
            let candidate = &self.candidates[index];
            match self.configure_candidate(candidate, config) {
                Ok(encoder) => return (Some((index, encoder)), failures),
                Err(err) => {
                    failures.push(format!("{}: {}", candidate.capabilities.backend_name, err))
                }
            }
        }
        (None, failures)
    }

    fn placeholder_capabilities() -> EncoderCapabilities {
        EncoderCapabilities {
            backend_name: "unconfigured".to_string(),
            codec: "avc".to_string(),
            hardware_accelerated: false,
            max_width: 0,
            max_height: 0,
            supported_fps: Vec::new(),
            supported_pixel_formats: vec![PixelFormat::Nv12],
        }
    }
}

impl VideoEncoder for FallbackVideoEncoder {
    fn capabilities(&self) -> EncoderCapabilities {
        self.active_backend()
            .or_else(|| {
                self.candidates
                    .first()
                    .map(|candidate| candidate.capabilities.clone())
            })
            .unwrap_or_else(Self::placeholder_capabilities)
    }

    fn configure(&mut self, config: VideoEncoderConfig) -> Result<()> {
        config.validate()?;
        self.config = Some(config.clone());
        self.active = None;
        self.active_index = None;
        self.candidates =
            rank_encoder_candidates(&self.factory.discover()?, &config, self.preference);
        if self.candidates.is_empty() {
            return Err(EncoderError::InitializationFailed {
                backend: "selection".to_string(),
                details: "no compatible encoder candidates were discovered".to_string(),
            });
        }

        let (active, failures) = self.activate_next(0, &config);
        let Some((index, encoder)) = active else {
            return Err(EncoderError::InitializationFailed {
                backend: "selection".to_string(),
                details: failures.join("; "),
            });
        };

        self.active_index = Some(index);
        self.active = Some(encoder);
        self.epoch = self.epoch.saturating_add(1);
        self.metrics.epoch = self.epoch;
        self.metrics.active_backend = Some(self.capabilities().backend_name.clone());
        self.events.push(EncoderEvent::BackendSelected {
            backend: self.capabilities().backend_name.clone(),
            hardware_accelerated: self.capabilities().hardware_accelerated,
        });
        Ok(())
    }

    fn set_gpu_frame_context(&mut self, context: Option<GpuFrameContext>) -> Result<()> {
        self.gpu_context = context.clone();
        if let Some(active) = self.active.as_mut() {
            active.set_gpu_frame_context(context)?;
        }
        Ok(())
    }

    fn encode(&mut self, frame: VideoFrame) -> Result<Vec<EncodedPacket>> {
        let config = self.config.clone().ok_or(EncoderError::NotConfigured)?;
        let mut frame_for_retry = frame;
        let mut first_error: Option<EncoderError> = None;
        let mut index = self.active_index.ok_or(EncoderError::NotConfigured)?;
        let mut reinitialized = false;
        self.metrics.frames_submitted += 1;

        loop {
            let Some(mut active) = self.active.take() else {
                return Err(first_error.unwrap_or(EncoderError::NotConfigured));
            };
            let backend = active.capabilities().backend_name;
            let retry_frame = frame_for_retry.clone();
            let started = Instant::now();
            let result = active.encode(frame_for_retry);
            match result {
                Ok(packets) => {
                    self.update_latency(started);
                    self.metrics.frames_encoded += 1;
                    self.metrics.active_backend = Some(backend);
                    self.active = Some(active);
                    return Ok(packets);
                }
                Err(error) => {
                    self.update_latency(started);
                    self.metrics.encode_failures += 1;
                    first_error.get_or_insert(error);

                    // Give the active backend one controlled reinitialization
                    // before moving to a different codec implementation.
                    if !reinitialized {
                        let _ = active.drain();
                        if let Ok(replacement) = self.activate_candidate(index, &config) {
                            self.active = Some(replacement);
                            self.active_index = Some(index);
                            self.epoch = self.epoch.saturating_add(1);
                            self.metrics.epoch = self.epoch;
                            self.metrics.fallback_count += 1;
                            self.metrics.active_backend = Some(backend.clone());
                            self.events.push(EncoderEvent::Fallback {
                                from_backend: backend.clone(),
                                to_backend: backend.clone(),
                                reason: first_error
                                    .as_ref()
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| "encoder failure".to_string()),
                                epoch: self.epoch,
                            });
                            frame_for_retry = retry_frame;
                            reinitialized = true;
                            continue;
                        }
                        reinitialized = true;
                    }

                    let _ = active.drain();
                    let next = index.saturating_add(1);
                    let (replacement, failures) = self.activate_next(next, &config);
                    let Some((replacement_index, replacement_encoder)) = replacement else {
                        self.metrics.frames_dropped += 1;
                        let details = failures.join("; ");
                        return Err(EncoderError::EncodeFailed {
                            details: if details.is_empty() {
                                first_error
                                    .as_ref()
                                    .map(|error| format!("active encoder failed: {error}"))
                                    .unwrap_or_else(|| {
                                        "active encoder failed and no fallback remained".to_string()
                                    })
                            } else {
                                format!(
                                    "active encoder failed: {}; fallback attempts: {details}",
                                    first_error
                                        .as_ref()
                                        .map(ToString::to_string)
                                        .unwrap_or_else(|| "unknown encoder failure".to_string())
                                )
                            },
                        });
                    };

                    let replacement_backend = replacement_encoder.capabilities().backend_name;
                    self.active = Some(replacement_encoder);
                    self.active_index = Some(replacement_index);
                    self.epoch = self.epoch.saturating_add(1);
                    self.metrics.epoch = self.epoch;
                    self.metrics.fallback_count += 1;
                    self.metrics.active_backend = Some(replacement_backend.clone());
                    self.events.push(EncoderEvent::Fallback {
                        from_backend: backend,
                        to_backend: replacement_backend,
                        reason: first_error
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "encoder failure".to_string()),
                        epoch: self.epoch,
                    });
                    frame_for_retry = retry_frame;
                    index = replacement_index;
                }
            }
        }
    }

    fn drain(&mut self) -> Result<Vec<EncodedPacket>> {
        let Some(active) = self.active.as_mut() else {
            return Err(EncoderError::NotConfigured);
        };
        let result = active.drain();
        if result.is_ok() {
            active.set_gpu_frame_context(None)?;
            self.gpu_context = None;
        }
        result
    }

    fn codec_extradata(&self) -> Option<PacketPayload> {
        self.active
            .as_ref()
            .and_then(|encoder| encoder.codec_extradata())
    }

    fn metrics(&self) -> EncoderMetrics {
        let mut metrics = self.metrics.clone();
        if let Some(active) = &self.active {
            let active_metrics = active.metrics();
            metrics.overload_events = metrics
                .overload_events
                .saturating_add(active_metrics.overload_events);
            metrics.frames_dropped = metrics
                .frames_dropped
                .saturating_add(active_metrics.frames_dropped);
        }
        metrics
    }

    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn take_events(&mut self) -> Vec<EncoderEvent> {
        std::mem::take(&mut self.events)
    }
}

impl Drop for FallbackVideoEncoder {
    fn drop(&mut self) {
        // Drop active transforms before asking a platform factory to release
        // its discovery/runtime state (notably Media Foundation COM state).
        self.active.take();
        self.candidates.clear();
        self.factory.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_types::{MediaType, PacketPayload, PixelFormat, StreamId, TimeBase};
    use std::sync::Mutex;

    #[derive(Debug, PartialEq, Eq)]
    struct TestResource(u32);

    struct TestResolver {
        seen: Mutex<Vec<GpuFrameHandle>>,
    }

    impl GpuFrameResolver for TestResolver {
        fn resolve(
            &self,
            handle: GpuFrameHandle,
        ) -> std::result::Result<Arc<GpuFrameResource>, GpuFrameResolveError> {
            self.seen.lock().expect("resolver calls").push(handle);
            if handle == GpuFrameHandle(99) {
                return Err(GpuFrameResolveError::StaleHandle { handle });
            }
            Ok(Arc::new(TestResource(7)))
        }
    }

    fn frame(payload: FramePayload) -> VideoFrame {
        VideoFrame {
            stream_id: StreamId(0),
            width: 1920,
            height: 1080,
            pixel_format: PixelFormat::Bgra8,
            pts: 0,
            time_base: TimeBase::MILLISECOND,
            payload,
        }
    }

    #[test]
    fn context_rejects_cpu_payload_without_calling_resolver() {
        let resolver = Arc::new(TestResolver {
            seen: Mutex::new(Vec::new()),
        });
        let context = GpuFrameContext::new(resolver.clone());
        assert!(matches!(
            context.resolve(&frame(FramePayload::Cpu(Arc::from(&b"cpu"[..])))),
            Err(GpuFrameResolveError::NotGpuFrame)
        ));
        assert!(resolver.seen.lock().expect("resolver calls").is_empty());
    }

    #[test]
    fn context_returns_typed_lease_and_preserves_handle() {
        let resolver = Arc::new(TestResolver {
            seen: Mutex::new(Vec::new()),
        });
        let context = GpuFrameContext::new(resolver.clone());
        let handle = GpuFrameHandle(4);
        let lease = context
            .resolve(&frame(FramePayload::Gpu(handle)))
            .expect("GPU resource");

        assert_eq!(lease.handle(), handle);
        assert_eq!(lease.downcast_ref::<TestResource>(), Some(&TestResource(7)));
        assert_eq!(
            resolver.seen.lock().expect("resolver calls").as_slice(),
            &[handle]
        );
    }

    #[test]
    fn stale_handle_error_is_preserved() {
        let context = GpuFrameContext::new(Arc::new(TestResolver {
            seen: Mutex::new(Vec::new()),
        }));
        let handle = GpuFrameHandle(99);
        assert!(matches!(
            context.resolve_handle(handle),
            Err(GpuFrameResolveError::StaleHandle { handle: found }) if found == handle
        ));
    }

    fn candidate(id: &str, backend_name: &str, hardware_accelerated: bool) -> EncoderCandidate {
        EncoderCandidate {
            id: id.to_string(),
            capabilities: EncoderCapabilities {
                backend_name: backend_name.to_string(),
                codec: "avc".to_string(),
                hardware_accelerated,
                max_width: 3_840,
                max_height: 2_160,
                supported_fps: vec![30, 60, 120],
                supported_pixel_formats: vec![PixelFormat::Nv12],
            },
        }
    }

    #[test]
    fn codec_matching_and_normalization() {
        assert_eq!(normalize_codec("avc"), "avc");
        assert_eq!(normalize_codec("AVC"), "avc");
        assert_eq!(normalize_codec("h264"), "avc");
        assert_eq!(normalize_codec("H264"), "avc");
        assert_eq!(normalize_codec("avc1"), "avc");
        assert_eq!(normalize_codec("AVC1"), "avc");
        assert_eq!(normalize_codec("hevc"), "hevc");
        assert_eq!(normalize_codec("HEVC"), "hevc");
        assert_eq!(normalize_codec("h265"), "hevc");
        assert_eq!(normalize_codec("H265"), "hevc");
        assert_eq!(normalize_codec("hvc1"), "hevc");
        assert_eq!(normalize_codec("HVC1"), "hevc");
        assert_eq!(normalize_codec("av1"), "av1");
        assert_eq!(normalize_codec("AV1"), "av1");
        assert_eq!(normalize_codec("av01"), "av1");
        assert_eq!(normalize_codec("AV01"), "av1");

        assert!(is_same_codec("avc", "h264"));
        assert!(is_same_codec("AVC1", "H264"));
        assert!(is_same_codec("hevc", "h265"));
        assert!(is_same_codec("HVC1", "HEVC"));
        assert!(is_same_codec("av1", "AV01"));
        assert!(!is_same_codec("avc", "hevc"));
        assert!(!is_same_codec("av1", "avc"));
    }

    #[test]
    fn candidate_ranking_prefers_known_hardware_then_software() {
        let config = VideoEncoderConfig::default();
        let candidates = vec![
            candidate("sw", "software-h264", false),
            candidate("amd", "amd-amf", true),
            candidate("nvidia", "nvidia-nvenc", true),
            candidate("intel", "intel-qsv", true),
        ];

        let ranked = rank_encoder_candidates(&candidates, &config, EncoderPreference::Auto);
        assert_eq!(
            ranked
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["nvidia", "amd", "intel", "sw"]
        );
        assert_eq!(
            rank_encoder_candidates(&candidates, &config, EncoderPreference::HardwareOnly).len(),
            3
        );
        assert_eq!(
            rank_encoder_candidates(&candidates, &config, EncoderPreference::SoftwareOnly)
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["sw"]
        );

        // Test ranking for AV1 candidates
        let av1_config = VideoEncoderConfig {
            codec: "av01".to_string(),
            ..VideoEncoderConfig::default()
        };
        let av1_candidates = vec![
            EncoderCandidate {
                id: "av1-sw".to_string(),
                capabilities: EncoderCapabilities {
                    backend_name: "software-av1".to_string(),
                    codec: "av1".to_string(),
                    hardware_accelerated: false,
                    max_width: 3_840,
                    max_height: 2_160,
                    supported_fps: vec![30, 60, 120],
                    supported_pixel_formats: vec![PixelFormat::Nv12],
                },
            },
            EncoderCandidate {
                id: "av1-amd".to_string(),
                capabilities: EncoderCapabilities {
                    backend_name: "amd-amf-av1".to_string(),
                    codec: "av1".to_string(),
                    hardware_accelerated: true,
                    max_width: 3_840,
                    max_height: 2_160,
                    supported_fps: vec![30, 60, 120],
                    supported_pixel_formats: vec![PixelFormat::Nv12],
                },
            },
        ];
        let ranked_av1 =
            rank_encoder_candidates(&av1_candidates, &av1_config, EncoderPreference::Auto);
        assert_eq!(
            ranked_av1
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["av1-amd", "av1-sw"]
        );
    }

    #[test]
    fn configuration_validation_rejects_odd_dimensions_and_bad_cadence() {
        let mut config = VideoEncoderConfig {
            width: 1_921,
            ..VideoEncoderConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(EncoderError::UnsupportedConfiguration { .. })
        ));
        config.width = 1_920;
        config.fps = 24;
        assert!(config.validate().is_err());
        config.fps = 120;
        assert!(config.validate().is_ok());
        config.fps = 60;
        config.keyframe_interval_seconds = 2.0;
        assert_eq!(config.keyframe_interval_frames(), 120);

        for codec in [
            "avc", "AVC", "h264", "H264", "avc1", "AVC1", "hevc", "HEVC", "h265", "H265", "hvc1",
            "HVC1", "av1", "AV1", "av01", "AV01",
        ] {
            config.codec = codec.to_string();
            assert!(
                config.validate().is_ok(),
                "expected codec '{codec}' to be valid"
            );
        }
        for unsupported in [
            "vp9", "prores", "VP9", "PRORES", "vvc", "h266", "mp4v", "unknown", "",
        ] {
            config.codec = unsupported.to_string();
            assert!(
                config.validate().is_err(),
                "expected codec '{unsupported}' to be rejected"
            );
        }
    }

    #[test]
    fn audio_configuration_validation_rejects_zero_values() {
        assert!(AudioEncoderConfig {
            sample_rate: 0,
            ..AudioEncoderConfig::default()
        }
        .validate()
        .is_err());
        assert!(AudioEncoderConfig {
            channels: 0,
            ..AudioEncoderConfig::default()
        }
        .validate()
        .is_err());
        assert!(AudioEncoderConfig {
            bitrate_kbps: 0,
            ..AudioEncoderConfig::default()
        }
        .validate()
        .is_err());
    }

    struct FakeVideoEncoder {
        capabilities: EncoderCapabilities,
        fail_encode: bool,
        drop_encode: bool,
        frames_dropped: u64,
        configured: bool,
    }

    impl VideoEncoder for FakeVideoEncoder {
        fn capabilities(&self) -> EncoderCapabilities {
            self.capabilities.clone()
        }

        fn configure(&mut self, config: VideoEncoderConfig) -> Result<()> {
            config.validate()?;
            self.configured = true;
            Ok(())
        }

        fn encode(&mut self, frame: VideoFrame) -> Result<Vec<EncodedPacket>> {
            if !self.configured {
                return Err(EncoderError::NotConfigured);
            }
            if self.fail_encode {
                return Err(EncoderError::EncodeFailed {
                    details: "injected fake failure".to_string(),
                });
            }
            if self.drop_encode {
                self.frames_dropped = self.frames_dropped.saturating_add(1);
                return Ok(Vec::new());
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

        fn drain(&mut self) -> Result<Vec<EncodedPacket>> {
            Ok(Vec::new())
        }

        fn metrics(&self) -> EncoderMetrics {
            EncoderMetrics {
                frames_dropped: self.frames_dropped,
                active_backend: Some(self.capabilities.backend_name.clone()),
                ..EncoderMetrics::default()
            }
        }
    }

    struct FakeFactory {
        candidates: Vec<EncoderCandidate>,
    }

    impl VideoEncoderFactory for FakeFactory {
        fn discover(&self) -> Result<Vec<EncoderCandidate>> {
            Ok(self.candidates.clone())
        }

        fn create(&self, candidate: &EncoderCandidate) -> Result<Box<dyn VideoEncoder>> {
            Ok(Box::new(FakeVideoEncoder {
                capabilities: candidate.capabilities.clone(),
                fail_encode: candidate.id == "hardware",
                drop_encode: candidate.id == "drop",
                frames_dropped: 0,
                configured: false,
            }))
        }
    }

    #[test]
    fn runtime_failure_retries_then_switches_to_next_backend() {
        let candidates = vec![
            candidate("hardware", "nvidia-nvenc", true),
            candidate("software", "software-h264", false),
        ];
        let mut encoder = FallbackVideoEncoder::new(
            Box::new(FakeFactory { candidates }),
            EncoderPreference::Auto,
        );
        encoder
            .configure(VideoEncoderConfig::default())
            .expect("initial backend");
        let frame = VideoFrame {
            stream_id: StreamId(0),
            width: 1_920,
            height: 1_080,
            pixel_format: PixelFormat::Nv12,
            pts: 0,
            time_base: TimeBase::MILLISECOND,
            payload: FramePayload::Cpu(Arc::from(&[0_u8; 6][..])),
        };

        let packets = encoder.encode(frame).expect("software fallback");
        assert_eq!(packets.len(), 1);
        assert_eq!(encoder.capabilities().backend_name, "software-h264");
        assert_eq!(encoder.epoch(), 3);
        assert_eq!(encoder.metrics().fallback_count, 2);
        assert!(encoder
            .take_events()
            .iter()
            .any(|event| matches!(event, EncoderEvent::Fallback { .. })));
    }

    #[test]
    fn runtime_failure_preserves_first_error_without_a_configured_fallback() {
        let candidates = vec![candidate("hardware", "nvidia-nvenc", true)];
        let mut encoder = FallbackVideoEncoder::new(
            Box::new(FakeFactory { candidates }),
            EncoderPreference::Auto,
        );
        encoder
            .configure(VideoEncoderConfig::default())
            .expect("initial backend");
        let frame = VideoFrame {
            stream_id: StreamId(0),
            width: 1_920,
            height: 1_080,
            pixel_format: PixelFormat::Nv12,
            pts: 0,
            time_base: TimeBase::MILLISECOND,
            payload: FramePayload::Cpu(Arc::from(&[0_u8; 6][..])),
        };

        let error = encoder.encode(frame).expect_err("single backend must fail");
        assert!(matches!(
            error,
            EncoderError::EncodeFailed { details }
                if details.contains("injected fake failure")
        ));
    }

    #[test]
    fn backend_dropped_frames_are_included_in_fallback_metrics() {
        let candidates = vec![candidate("drop", "drop-h264", true)];
        let mut encoder = FallbackVideoEncoder::new(
            Box::new(FakeFactory { candidates }),
            EncoderPreference::Auto,
        );
        encoder
            .configure(VideoEncoderConfig::default())
            .expect("initial backend");
        let frame = VideoFrame {
            stream_id: StreamId(0),
            width: 1_920,
            height: 1_080,
            pixel_format: PixelFormat::Nv12,
            pts: 0,
            time_base: TimeBase::MILLISECOND,
            payload: FramePayload::Cpu(Arc::from(&[0_u8; 6][..])),
        };

        assert!(encoder
            .encode(frame)
            .expect("a dropped frame is not an encoder failure")
            .is_empty());
        assert_eq!(encoder.metrics().frames_dropped, 1);
        assert_eq!(encoder.metrics().fallback_count, 0);
    }

    #[test]
    fn suggested_bitrate_scaling_and_clamping() {
        // 720p30: 1280 * 720 * 30 = 27,648,000 pps -> 2,764 kbps (base)
        let config_720p30 = VideoEncoderConfig {
            width: 1280,
            height: 720,
            fps: 30,
            preset_level: 2,
            ..VideoEncoderConfig::default()
        };
        assert_eq!(config_720p30.suggested_bitrate_kbps(), 2_764);

        // 1080p60: 1920 * 1080 * 60 = 124,416,000 pps -> 12,441 kbps (base)
        let config_1080p60 = VideoEncoderConfig {
            width: 1920,
            height: 1080,
            fps: 60,
            preset_level: 2,
            ..VideoEncoderConfig::default()
        };
        assert_eq!(config_1080p60.suggested_bitrate_kbps(), 12_441);

        // 1440p60: 2560 * 1440 * 60 = 221,184,000 pps -> 22,118 kbps (base)
        let config_1440p60 = VideoEncoderConfig {
            width: 2560,
            height: 1440,
            fps: 60,
            preset_level: 2,
            ..VideoEncoderConfig::default()
        };
        assert_eq!(config_1440p60.suggested_bitrate_kbps(), 22_118);

        // 4K60: 3840 * 2160 * 60 = 497,664,000 pps -> 49,766 -> clamped to 40,000 kbps (base)
        let config_4k60 = VideoEncoderConfig {
            width: 3840,
            height: 2160,
            fps: 60,
            preset_level: 2,
            ..VideoEncoderConfig::default()
        };
        assert_eq!(config_4k60.suggested_bitrate_kbps(), 40_000);
    }

    #[test]
    fn suggested_bitrate_preset_ordering() {
        let base_config = VideoEncoderConfig {
            width: 1920,
            height: 1080,
            fps: 60,
            ..VideoEncoderConfig::default()
        };

        let rates: Vec<u32> = (0..=5)
            .map(|level| {
                VideoEncoderConfig {
                    preset_level: level,
                    ..base_config.clone()
                }
                .suggested_bitrate_kbps()
            })
            .collect();

        // Check strict monotonic ordering across presets
        for window in rates.windows(2) {
            assert!(
                window[0] < window[1],
                "expected preset rates to be strictly increasing: {:?}",
                rates
            );
        }

        assert_eq!(rates, vec![8_294, 9_952, 12_441, 14_929, 16_588, 18_661]);
    }

    #[test]
    fn suggested_bitrate_explicit_override() {
        let config = VideoEncoderConfig {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_kbps: Some(80_000),
            preset_level: 2,
            ..VideoEncoderConfig::default()
        };
        assert_eq!(config.suggested_bitrate_kbps(), 80_000);

        let config_4k = VideoEncoderConfig {
            width: 3840,
            height: 2160,
            fps: 60,
            bitrate_kbps: Some(80_000),
            preset_level: 5,
            ..VideoEncoderConfig::default()
        };
        assert_eq!(config_4k.suggested_bitrate_kbps(), 80_000);
    }

    #[test]
    fn capabilities_pixel_format_filtering_and_serde_compatibility() {
        let caps = EncoderCapabilities {
            backend_name: "test-backend".to_string(),
            codec: "avc".to_string(),
            hardware_accelerated: true,
            max_width: 3840,
            max_height: 2160,
            supported_fps: vec![30, 60],
            supported_pixel_formats: vec![PixelFormat::Nv12],
        };

        let nv12_config = VideoEncoderConfig {
            pixel_format: PixelFormat::Nv12,
            ..VideoEncoderConfig::default()
        };
        assert!(caps.supports(&nv12_config));

        let ayuv_config = VideoEncoderConfig {
            pixel_format: PixelFormat::Ayuv,
            ..VideoEncoderConfig::default()
        };
        assert!(!caps.supports(&ayuv_config));

        // Test historical JSON without supported_pixel_formats deserializes with NV12 default
        let historical_json = r#"{
            "backend_name": "historical",
            "codec": "avc",
            "hardware_accelerated": false,
            "max_width": 1920,
            "max_height": 1080,
            "supported_fps": [60]
        }"#;
        let deserialized: EncoderCapabilities =
            serde_json::from_str(historical_json).expect("historical deserializes");
        assert_eq!(
            deserialized.supported_pixel_formats,
            vec![PixelFormat::Nv12]
        );
        assert!(deserialized.supports(&nv12_config));
        assert!(!deserialized.supports(&ayuv_config));
    }
}
