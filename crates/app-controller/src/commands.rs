//! Commands accepted from the UI/tray/hotkey surfaces. Every entry is part
//! of the IPC allowlist (spec §24): anything not listed here cannot be
//! invoked from the frontend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    StartCapture,
    StopCapture,
    SaveReplay,
    Poll,
    Ping { message: String },
}

impl Command {
    /// Stable name used in logs and rejection events.
    pub fn name(&self) -> &'static str {
        match self {
            Self::StartCapture => "start_capture",
            Self::StopCapture => "stop_capture",
            Self::SaveReplay => "save_replay",
            Self::Poll => "poll",
            Self::Ping { .. } => "ping",
        }
    }
}
