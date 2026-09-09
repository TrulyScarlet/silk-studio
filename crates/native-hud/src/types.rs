//! Deterministic pure types and definitions for the Native HUD.

use thiserror::Error;

/// Bounded queue capacity for asynchronous HUD commands.
pub const HUD_COMMAND_QUEUE_BOUND: usize = 16;

/// Layout mode for the native HUD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HudMode {
    /// Full confirmation capsule (320x56 DIP, title and subtitle).
    #[default]
    Full,
    /// Compact confirmation pill (180x38 DIP, title only, border radius 19 DIP).
    Compact,
}

/// 8-point anchoring positions for the HUD window on the primary display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HudAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    CenterRight,
    BottomLeft,
    #[default]
    BottomCenter,
    BottomRight,
}

impl HudAnchor {
    /// String identifier matching the configuration contract.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::TopLeft => "top_left",
            Self::TopCenter => "top_center",
            Self::TopRight => "top_right",
            Self::CenterLeft => "center_left",
            Self::CenterRight => "center_right",
            Self::BottomLeft => "bottom_left",
            Self::BottomCenter => "bottom_center",
            Self::BottomRight => "bottom_right",
        }
    }

    /// Whether this anchor is located on the top edge of the display.
    pub const fn is_top(&self) -> bool {
        matches!(self, Self::TopLeft | Self::TopCenter | Self::TopRight)
    }

    /// Whether this anchor is located on the bottom edge of the display.
    pub const fn is_bottom(&self) -> bool {
        matches!(
            self,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight
        )
    }

    /// Whether this anchor is vertically centered.
    pub const fn is_center_y(&self) -> bool {
        matches!(self, Self::CenterLeft | Self::CenterRight)
    }

    /// Whether this anchor belongs to the left family (flush left).
    pub const fn is_left(&self) -> bool {
        matches!(self, Self::TopLeft | Self::CenterLeft | Self::BottomLeft)
    }

    /// Whether this anchor belongs to the right family (flush right).
    pub const fn is_right(&self) -> bool {
        matches!(self, Self::TopRight | Self::CenterRight | Self::BottomRight)
    }

    /// Whether this anchor is horizontally centered.
    pub const fn is_center_x(&self) -> bool {
        matches!(self, Self::TopCenter | Self::BottomCenter)
    }

    /// In-canvas horizontal and vertical offset `(x, y)` in DIPs for aligning the
    /// compact 180x38 pill within the fixed 320x56 canvas per Section 2.3.
    pub const fn compact_offset_dip(&self) -> (f32, f32) {
        let x = match self {
            Self::TopLeft | Self::CenterLeft | Self::BottomLeft => 0.0,
            Self::TopCenter | Self::BottomCenter => 70.0,
            Self::TopRight | Self::CenterRight | Self::BottomRight => 140.0,
        };
        let y = match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => 0.0,
            Self::CenterLeft | Self::CenterRight => 9.0,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => 18.0,
        };
        (x, y)
    }

    /// Directional entrance translation in DIPs for standard motion.
    ///
    /// Bottom anchors slide up (+8.0 -> 0.0), top anchors slide down (-8.0 -> 0.0),
    /// and center anchors do not translate (0.0).
    pub const fn slide_offset_dip(&self) -> f32 {
        if self.is_bottom() {
            8.0
        } else if self.is_top() {
            -8.0
        } else {
            0.0
        }
    }
}

/// Visual state kinds supported by the HUD renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HudStateKind {
    Queued,
    Saved,
    Failed,
}

/// A capture confirmation cue payload submitted to the Native HUD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HudCue {
    /// Save replay in progress.
    Queued {
        title: Option<String>,
        subtitle: Option<String>,
    },
    /// Replay successfully captured and written to disk.
    Saved {
        duration_ms: Option<u64>,
        bytes: Option<u64>,
        custom_subtitle: Option<String>,
    },
    /// Save failed due to an error.
    Failed { reason: String },
    /// Save command was rejected (e.g. buffer not ready).
    Rejected { reason: String },
}

impl HudCue {
    /// Creates a default `Queued` cue ("Saving replay...", "Writing clip to disk").
    pub fn queued() -> Self {
        Self::Queued {
            title: None,
            subtitle: None,
        }
    }

    /// Creates a `Queued` cue with custom title and subtitle.
    pub fn queued_custom(title: impl Into<String>, subtitle: impl Into<String>) -> Self {
        Self::Queued {
            title: Some(title.into()),
            subtitle: Some(subtitle.into()),
        }
    }

    /// Creates a `Saved` cue with formatted duration and file size.
    pub fn saved(duration_ms: u64, bytes: u64) -> Self {
        Self::Saved {
            duration_ms: Some(duration_ms),
            bytes: Some(bytes),
            custom_subtitle: None,
        }
    }

    /// Creates a `Saved` cue with custom subtitle text.
    pub fn saved_custom(subtitle: impl Into<String>) -> Self {
        Self::Saved {
            duration_ms: None,
            bytes: None,
            custom_subtitle: Some(subtitle.into()),
        }
    }

    /// Creates a `Failed` cue with the given failure reason.
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::Failed {
            reason: reason.into(),
        }
    }

    /// Creates a `Rejected` cue with the given rejection reason.
    pub fn rejected(reason: impl Into<String>) -> Self {
        Self::Rejected {
            reason: reason.into(),
        }
    }

    /// The visual state kind corresponding to this cue.
    pub const fn state_kind(&self) -> HudStateKind {
        match self {
            Self::Queued { .. } => HudStateKind::Queued,
            Self::Saved { .. } => HudStateKind::Saved,
            Self::Failed { .. } | Self::Rejected { .. } => HudStateKind::Failed,
        }
    }

    /// The title text to render for this cue.
    pub fn title(&self) -> &str {
        match self {
            Self::Queued { title: Some(t), .. } => t.as_str(),
            Self::Queued { title: None, .. } => "Saving replay...",
            Self::Saved { .. } => "Silk Captured",
            Self::Failed { .. } => "Save failed",
            Self::Rejected { .. } => "Save rejected",
        }
    }

    /// Resolves the subtitle text for this cue, formatting duration/size if present.
    pub fn subtitle(&self) -> String {
        match self {
            Self::Queued {
                subtitle: Some(s), ..
            } => s.clone(),
            Self::Queued { subtitle: None, .. } => "Writing clip to disk".to_string(),
            Self::Saved {
                custom_subtitle: Some(s),
                ..
            } => s.clone(),
            Self::Saved {
                duration_ms, bytes, ..
            } => match (*duration_ms, *bytes) {
                (Some(dur), Some(b)) => crate::format::format_saved_subtitle(dur, b),
                (Some(dur), None) => crate::format::format_duration(dur),
                (None, Some(b)) => crate::format::format_file_size(b),
                (None, None) => "Clip saved".to_string(),
            },
            Self::Failed { reason } | Self::Rejected { reason } => reason.clone(),
        }
    }

    /// Hold duration in milliseconds for this cue state.
    pub const fn hold_duration_ms(&self) -> u64 {
        match self {
            Self::Queued { .. } => crate::geometry::MAX_QUEUED_HOLD_MS,
            Self::Saved { .. } => crate::geometry::HOLD_SAVED_DURATION_MS,
            Self::Failed { .. } | Self::Rejected { .. } => crate::geometry::HOLD_FAILED_DURATION_MS,
        }
    }
}

/// Configuration parameters for initializing the native HUD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudConfig {
    /// Visual layout mode (Full 320x56 capsule or Compact 180x38 pill).
    pub mode: HudMode,
    /// Screen anchor position.
    pub anchor: HudAnchor,
    /// Whether to apply window display capture exclusion (`WDA_EXCLUDEFROMCAPTURE`).
    pub exclude_from_capture: bool,
}

impl Default for HudConfig {
    fn default() -> Self {
        Self {
            mode: HudMode::Full,
            anchor: HudAnchor::BottomCenter,
            exclude_from_capture: true,
        }
    }
}

/// Runtime capabilities and status determined at initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HudCapabilities {
    /// Whether native HUD rendering is supported on this platform.
    pub is_supported: bool,
    /// Whether capture exclusion API is available on this system.
    pub capture_exclusion_supported: bool,
    /// Whether capture exclusion was successfully applied to the window.
    pub capture_exclusion_applied: bool,
    /// Whether system reduced-motion is active.
    pub reduced_motion: bool,
}

/// Diagnostic lifecycle policy for DirectComposition root attachment and HWND visibility.
#[cfg(feature = "diagnostic-lifecycle")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DiagnosticLifecyclePolicy {
    /// HWND is shown continuously; DirectComposition root visual is attached continuously.
    AttachedShown,
    /// HWND is shown continuously; DirectComposition root visual is detached (null) while idle.
    DetachedShown,
    /// DirectComposition root visual remains attached; HWND is hidden (`SW_HIDE`) while idle.
    #[default]
    AttachedHidden,
}

#[cfg(not(feature = "diagnostic-lifecycle"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum DiagnosticLifecyclePolicy {
    #[allow(dead_code)]
    AttachedShown,
    #[allow(dead_code)]
    DetachedShown,
    #[default]
    AttachedHidden,
}

#[cfg(feature = "diagnostic-lifecycle")]
impl DiagnosticLifecyclePolicy {
    /// Whether DirectComposition root visual is attached while idle.
    pub const fn is_idle_root_attached(&self) -> bool {
        match self {
            Self::AttachedShown | Self::AttachedHidden => true,
            Self::DetachedShown => false,
        }
    }

    /// Whether the native HWND is visible while idle.
    pub const fn is_idle_window_shown(&self) -> bool {
        match self {
            Self::AttachedShown | Self::DetachedShown => true,
            Self::AttachedHidden => false,
        }
    }
}

/// Errors produced during Native HUD initialization or operation.
#[derive(Debug, Error)]
pub enum HudError {
    #[error("native HUD is not supported on this platform")]
    UnsupportedPlatform,

    #[error("failed to spawn native HUD thread: {0}")]
    ThreadSpawnFailed(String),

    #[error("native HUD initialization timed out")]
    InitializationTimeout,

    #[error("native HUD initialization failed: {0}")]
    InitializationFailed(String),

    #[error("window creation failed: {0}")]
    WindowCreationFailed(String),

    #[error("Direct3D / DXGI device creation failed: {0}")]
    DeviceCreationFailed(String),

    #[error("DirectComposition initialization failed: {0}")]
    CompositionFailed(String),

    #[error("Direct2D initialization failed: {0}")]
    Direct2DFailed(String),

    #[error("DirectWrite initialization failed: {0}")]
    DirectWriteFailed(String),

    #[error("HUD command queue is full (capacity {HUD_COMMAND_QUEUE_BOUND})")]
    QueueFull,

    #[error("HUD command channel disconnected")]
    ChannelDisconnected,

    #[error("HUD is shutting down")]
    ShuttingDown,
}
