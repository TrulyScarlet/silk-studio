use thiserror::Error;

use crate::state::{InvalidTransition, RecorderState};

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    InvalidTransition(#[from] InvalidTransition),

    #[error(transparent)]
    Muxer(#[from] muxer::MuxerError),

    #[error(transparent)]
    Encoder(#[from] encoder_api::EncoderError),

    #[error(transparent)]
    Capture(#[from] capture_api::VideoCaptureError),

    #[error(transparent)]
    Buffer(#[from] replay_buffer::BufferError),

    #[error(transparent)]
    AudioSync(#[from] audio_sync::AudioSyncError),

    #[error("recorder must be in '{expected}', but it is in '{current}'")]
    WrongState {
        current: RecorderState,
        expected: &'static str,
    },

    #[error("engine already started")]
    AlreadyStarted,

    #[error("engine is not started")]
    NotStarted,

    #[error("no packets are buffered yet; replay is unavailable (BUF-001 not satisfied)")]
    NothingBuffered,

    #[error("engine was constructed without required components")]
    MissingComponent,

    #[error("invalid engine configuration: {details}")]
    InvalidConfiguration { details: String },
}

impl EngineError {
    /// Stable code from the spec §20 categories where one applies.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidTransition(_) | Self::WrongState { .. } => "INVALID_STATE_TRANSITION",
            Self::AlreadyStarted | Self::NotStarted | Self::MissingComponent => "ENGINE_LIFECYCLE",
            Self::InvalidConfiguration { .. } => "ENGINE_INVALID_CONFIGURATION",
            Self::NothingBuffered => "BUFFER_NOT_READY",
            Self::Muxer(inner) => inner.code(),
            Self::Encoder(inner) => inner.code(),
            Self::Capture(inner) => inner.code(),
            Self::Buffer(inner) => inner.code(),
            Self::AudioSync(_) => "AUDIO_SYNC_FAILURE",
        }
    }
}
