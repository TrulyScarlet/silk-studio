//! Fallback implementation for non-Windows platforms.

#[cfg(feature = "diagnostic-lifecycle")]
use crate::types::DiagnosticLifecyclePolicy;
use crate::types::{HudCapabilities, HudConfig, HudCue, HudError};

/// A stub HUD handle for non-Windows platforms.
pub struct NativeHud;

impl NativeHud {
    /// Always returns `HudError::UnsupportedPlatform` on non-Windows platforms.
    pub fn try_new(_config: HudConfig) -> Result<Self, HudError> {
        Err(HudError::UnsupportedPlatform)
    }

    /// Always returns `HudError::UnsupportedPlatform` on non-Windows platforms.
    #[cfg(feature = "diagnostic-lifecycle")]
    pub fn try_new_diagnostic(
        _config: HudConfig,
        _policy: DiagnosticLifecyclePolicy,
    ) -> Result<Self, HudError> {
        Err(HudError::UnsupportedPlatform)
    }

    /// Returns whether the HUD is operational on this platform (always false).
    pub fn is_healthy(&self) -> bool {
        false
    }

    /// Non-operational stub for non-Windows platforms.
    pub fn try_show(&self, _cue: HudCue) -> Result<(), HudError> {
        Err(HudError::UnsupportedPlatform)
    }

    /// Returns default capabilities reflecting no platform support.
    pub fn capabilities(&self) -> &HudCapabilities {
        const DEFAULT_CAPS: HudCapabilities = HudCapabilities {
            is_supported: false,
            capture_exclusion_supported: false,
            capture_exclusion_applied: false,
            reduced_motion: false,
        };
        &DEFAULT_CAPS
    }

    /// Non-operational stub shutdown.
    pub fn shutdown(&self) {}
}
