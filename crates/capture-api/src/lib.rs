//! Video-capture abstraction (spec §13.2).
//!
//! Backends implement this trait; the engine never sees platform types.
//! The native Windows Graphics Capture implementation lives in
//! `capture-windows`.

use encoder_api::GpuFrameContext;
use media_types::{VideoFrame, VideoSourceInfo};
use thiserror::Error;

pub type Result<T, E = VideoCaptureError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum VideoCaptureError {
    #[error("no capture source is available")]
    SourceUnavailable,

    #[error("capture permission was denied by the operating system")]
    PermissionDenied,

    #[error("the capture source was lost")]
    DeviceLost,

    #[error("invalid video capture configuration: {reason}")]
    InvalidConfiguration { reason: String },

    #[error("the capture stream ended")]
    EndOfStream,

    #[error("video capture backend failure: {details}")]
    Backend { details: String },
}

impl VideoCaptureError {
    /// Stable code from the spec §20 error-category list.
    pub fn code(&self) -> &'static str {
        match self {
            Self::SourceUnavailable => "CAPTURE_SOURCE_UNAVAILABLE",
            Self::PermissionDenied => "CAPTURE_PERMISSION_DENIED",
            Self::DeviceLost => "CAPTURE_DEVICE_LOST",
            Self::InvalidConfiguration { .. } => "CAPTURE_INVALID_CONFIGURATION",
            Self::EndOfStream => "CAPTURE_END_OF_STREAM",
            Self::Backend { .. } => "CAPTURE_BACKEND_FAILURE",
        }
    }
}

/// Requested capture parameters for one display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoCaptureConfig {
    pub source_id: String,
    pub target_fps: u32,
}

/// Events produced by an active video capture session.
#[derive(Debug, Clone)]
pub enum VideoCaptureEvent {
    Frame(VideoFrame),
    FormatChanged {
        width: u32,
        height: u32,
    },
    /// The display disappeared; recovery is coordinated by the engine
    /// (spec §21.2), not inside the backend.
    SourceLost,
}

/// Original interface for display capture. Implementations own any
/// platform resources and must document their thread-affinity rules.
pub trait VideoCapture: Send {
    fn enumerate_sources(&self) -> Result<Vec<VideoSourceInfo>>;
    fn start(&mut self, config: VideoCaptureConfig) -> Result<()>;
    fn next_event(&mut self) -> Result<VideoCaptureEvent>;
    fn stop(&mut self) -> Result<()>;

    /// Optional session-scoped GPU bridge for encoders. CPU/mock capture
    /// implementations return `None`; native GPU capture exposes a context
    /// only while its session is running.
    fn gpu_frame_context(&self) -> Option<GpuFrameContext> {
        None
    }
}
