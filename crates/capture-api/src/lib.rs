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

    #[error("capture was denied by Windows or the selected application; this source may be unsupported. Check Windows capture permissions or select another supported window. Application-enforced capture exclusions cannot be overridden. Silk will not bypass capture restrictions or switch to desktop capture")]
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

#[cfg(test)]
mod error_tests {
    use super::*;
    #[test]
    fn permission_denial_preserves_code_and_explains_application_restrictions() {
        let error = VideoCaptureError::PermissionDenied;
        assert_eq!(error.code(), "CAPTURE_PERMISSION_DENIED");
        let message = error.to_string();
        assert!(message.contains("selected application"));
        assert!(message.contains("unsupported"));
        assert!(message.contains("will not bypass"));
        assert!(message.contains("or switch to desktop capture"));
        assert!(message.contains("Check Windows capture permissions"));
        assert!(message.contains("select another supported window"));
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
    /// No frame is currently available. This is not source loss: return control
    /// so the caller can service stop/reconfiguration without fabricating media.
    Idle,
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
    /// Identity of the attached capture session, not a separately detected app.
    fn active_source_id(&self) -> Option<String> {
        None
    }
    /// Epoch of frame timestamps, when correlated to the host monotonic clock.
    fn timestamp_origin(&self) -> Option<std::time::Instant> {
        None
    }
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
