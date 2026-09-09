use thiserror::Error;

use media_types::StreamId;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BufferError {
    #[error("packets arrived for unregistered stream {stream_id}")]
    UnregisteredStream { stream_id: StreamId },

    #[error("stream {stream_id} went backwards: previous dts {previous_dts} -> new dts {new_dts}")]
    NonMonotonicTimestamp {
        stream_id: StreamId,
        previous_dts: i64,
        new_dts: i64,
    },

    #[error("replay buffer holds no packets yet (BUF-001 not satisfied)")]
    NothingBuffered,

    #[error("no keyframe at or before requested start {requested_start_pts}; buffer not ready (BUF-006)")]
    NoKeyframeAtOrBefore { requested_start_pts: i64 },

    #[error("retention must be positive, got {value_ms} ms")]
    RetentionTooShort { value_ms: i64 },

    #[error("stream time bases must be the shared millisecond base (ADR 0004)")]
    UnsupportedTimeBase,
}

impl BufferError {
    /// Stable code from the spec §20 categories where one applies.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnregisteredStream { .. }
            | Self::NonMonotonicTimestamp { .. }
            | Self::UnsupportedTimeBase => "BUFFER_INVALID_PACKET",
            Self::NothingBuffered | Self::NoKeyframeAtOrBefore { .. } => "BUFFER_NOT_READY",
            Self::RetentionTooShort { .. } => "INVALID_STATE_TRANSITION",
        }
    }
}
