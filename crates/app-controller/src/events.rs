//! Events forwarded from the controller to the desktop UI bridge. All
//! payloads are plain serializable data; no media content crosses this
//! boundary (spec §24).

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControllerEvent {
    StatusChanged {
        state: String,
    },

    ClipSaved {
        path: String,
        duration_ms: u64,
        size_bytes: u64,
    },

    SaveQueued {
        path: String,
    },

    SaveFailed {
        code: String,
        message: String,
    },

    CommandRejected {
        command: String,
        reason: String,
    },

    Notification {
        level: String,
        title: String,
        body: String,
    },

    Warning {
        code: String,
        message: String,
    },
}

impl ControllerEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::StatusChanged { .. } => "status_changed",
            Self::ClipSaved { .. } => "clip_saved",
            Self::SaveQueued { .. } => "save_queued",
            Self::SaveFailed { .. } => "save_failed",
            Self::CommandRejected { .. } => "command_rejected",
            Self::Notification { .. } => "notification",
            Self::Warning { .. } => "warning",
        }
    }
}
