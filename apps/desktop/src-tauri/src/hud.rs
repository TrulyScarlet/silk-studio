//! Native HUD runtime integration for desktop capture feedback.
//!
//! Replaces the legacy WebView-based overlay with a lightweight, zero-window-mutation
//! Direct2D / DirectComposition HUD window managed on a dedicated worker thread.

use std::sync::{Mutex, RwLock};

use app_controller::ControllerEvent;
use configuration::{OverlayMode, OverlayPosition, OverlaySettings};
use native_hud::{HudAnchor, HudConfig, HudCue, HudMode, NativeHud};

/// Map configuration overlay mode 1:1 to native HUD mode.
pub fn map_mode(mode: OverlayMode) -> HudMode {
    match mode {
        OverlayMode::Full => HudMode::Full,
        OverlayMode::Compact => HudMode::Compact,
    }
}

/// Map configuration overlay position 1:1 to native HUD anchor.
pub fn map_anchor(position: OverlayPosition) -> HudAnchor {
    match position {
        OverlayPosition::TopLeft => HudAnchor::TopLeft,
        OverlayPosition::TopCenter => HudAnchor::TopCenter,
        OverlayPosition::TopRight => HudAnchor::TopRight,
        OverlayPosition::CenterLeft => HudAnchor::CenterLeft,
        OverlayPosition::CenterRight => HudAnchor::CenterRight,
        OverlayPosition::BottomLeft => HudAnchor::BottomLeft,
        OverlayPosition::BottomCenter => HudAnchor::BottomCenter,
        OverlayPosition::BottomRight => HudAnchor::BottomRight,
    }
}

/// Derive native HUD configuration from user settings. Capture exclusion is always requested.
pub fn hud_config_from_settings(settings: &OverlaySettings) -> HudConfig {
    HudConfig {
        mode: map_mode(settings.mode),
        anchor: map_anchor(settings.position),
        exclude_from_capture: true,
    }
}

/// Map controller events to corresponding HUD cues.
///
/// Only `SaveQueued`, `ClipSaved`, `SaveFailed`, and `save_replay` `CommandRejected`
/// produce visual cues. All other controller events return `None`.
pub fn cue_for_event(event: &ControllerEvent) -> Option<HudCue> {
    match event {
        ControllerEvent::SaveQueued { .. } => Some(HudCue::queued()),
        ControllerEvent::ClipSaved {
            duration_ms,
            size_bytes,
            ..
        } => Some(HudCue::saved(*duration_ms, *size_bytes)),
        ControllerEvent::SaveFailed { message, .. } => Some(HudCue::failed(message.clone())),
        ControllerEvent::CommandRejected { command, reason } if command == "save_replay" => {
            Some(HudCue::rejected(reason.clone()))
        }
        _ => None,
    }
}

struct HudInner {
    settings: OverlaySettings,
    hud: Option<NativeHud>,
}

/// Tauri-managed native HUD runtime.
///
/// Owns the active `NativeHud` instance behind an `RwLock` and serializes
/// background reconfigurations with a dedicated `Mutex<()>`.
pub struct HudRuntime {
    inner: RwLock<HudInner>,
    reconfig_lock: Mutex<()>,
}

impl HudRuntime {
    /// Initialize HUD runtime according to initial settings.
    ///
    /// When disabled, no HWND or background rendering thread is created.
    /// If initialization fails, logs a truthful warning and leaves HUD disabled.
    pub fn init(settings: OverlaySettings) -> Self {
        let hud = if settings.enabled {
            let config = hud_config_from_settings(&settings);
            match NativeHud::try_new(config) {
                Ok(hud) => Some(hud),
                Err(error) => {
                    diagnostics::warn(
                        "native-hud",
                        &format!("native HUD initialization failed: {error}"),
                    );
                    None
                }
            }
        } else {
            None
        };
        Self {
            inner: RwLock::new(HudInner { settings, hud }),
            reconfig_lock: Mutex::new(()),
        }
    }

    /// Construct a headless HUD runtime with no native window for testing.
    #[allow(dead_code)]
    pub fn headless(settings: OverlaySettings) -> Self {
        Self {
            inner: RwLock::new(HudInner {
                settings,
                hud: None,
            }),
            reconfig_lock: Mutex::new(()),
        }
    }

    /// Return the currently configured overlay settings snapshot.
    #[allow(dead_code)]
    pub fn settings(&self) -> OverlaySettings {
        self.inner
            .read()
            .map(|guard| guard.settings)
            .unwrap_or_default()
    }

    /// Returns `true` if a native HUD worker is currently active and healthy.
    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.inner
            .read()
            .map(|guard| guard.hud.as_ref().is_some_and(|h| h.is_healthy()))
            .unwrap_or(false)
    }

    /// Attempt non-blocking HUD dispatch on the controller event hot path.
    ///
    /// Uses `try_read` to avoid blocking during runtime reconfiguration; drops
    /// the cue immediately under lock contention or when disabled/unavailable
    /// without cloning failure strings, formatting, or logging.
    pub fn try_dispatch_event(&self, event: &ControllerEvent) {
        let Ok(guard) = self.inner.try_read() else {
            return;
        };
        if !guard.settings.enabled {
            return;
        }
        let Some(hud) = guard.hud.as_ref() else {
            return;
        };
        if !hud.is_healthy() {
            return;
        }
        let Some(cue) = cue_for_event(event) else {
            return;
        };
        let _ = hud.try_show(cue);
    }

    /// Reconfigure the native HUD to match updated settings.
    ///
    /// Serializes concurrent reconfigurations. Builds the replacement outside
    /// the inner lock, atomically swaps instances under a brief write lock, and
    /// joins the previous worker outside all locks on drop.
    pub fn reconfigure(&self, new_settings: OverlaySettings) {
        let reconfig_guard = match self.reconfig_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let needs_rebuild = {
            let guard = match self.inner.read() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let settings_changed = guard.settings != new_settings;
            let missing_or_unhealthy =
                new_settings.enabled && guard.hud.as_ref().is_none_or(|h| !h.is_healthy());
            settings_changed || missing_or_unhealthy
        };

        if !needs_rebuild {
            return;
        }

        let new_hud = if new_settings.enabled {
            let config = hud_config_from_settings(&new_settings);
            match NativeHud::try_new(config) {
                Ok(hud) => Some(hud),
                Err(error) => {
                    diagnostics::warn(
                        "native-hud",
                        &format!("native HUD reconfiguration failed: {error}"),
                    );
                    None
                }
            }
        } else {
            None
        };

        let old_hud = {
            let mut guard = match self.inner.write() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.settings = new_settings;
            std::mem::replace(&mut guard.hud, new_hud)
        };

        // Explicitly release the reconfiguration lock before dropping the old HUD instance,
        // so worker thread join occurs outside all locks.
        drop(reconfig_guard);
        drop(old_hud);
    }

    /// Show a test capture confirmation cue via the managed native HUD.
    ///
    /// Returns an informative error if overlay is disabled, uninitialized, or busy.
    pub fn show_test_cue(&self) -> Result<(), String> {
        let guard = match self.inner.try_read() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err("native HUD is busy (reconfiguring)".to_string());
            }
            Err(std::sync::TryLockError::Poisoned(error)) => {
                return Err(format!("native HUD lock poisoned: {error}"));
            }
        };
        if !guard.settings.enabled {
            return Err("native HUD is disabled in settings".to_string());
        }
        let Some(hud) = guard.hud.as_ref() else {
            return Err("native HUD is unavailable or failed to initialize".to_string());
        };
        let cue = HudCue::saved(2_000, 10_000_000);
        hud.try_show(cue)
            .map_err(|error| format!("failed to show native HUD test cue: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_controller::ControllerEvent;
    use configuration::{OverlayMode, OverlayPosition, OverlaySettings};
    use native_hud::{HudAnchor, HudCue, HudMode};

    #[test]
    fn test_mode_mapping() {
        assert_eq!(map_mode(OverlayMode::Full), HudMode::Full);
        assert_eq!(map_mode(OverlayMode::Compact), HudMode::Compact);
    }

    #[test]
    fn test_anchor_mapping_all_variants() {
        assert_eq!(map_anchor(OverlayPosition::TopLeft), HudAnchor::TopLeft);
        assert_eq!(map_anchor(OverlayPosition::TopCenter), HudAnchor::TopCenter);
        assert_eq!(map_anchor(OverlayPosition::TopRight), HudAnchor::TopRight);
        assert_eq!(
            map_anchor(OverlayPosition::CenterLeft),
            HudAnchor::CenterLeft
        );
        assert_eq!(
            map_anchor(OverlayPosition::CenterRight),
            HudAnchor::CenterRight
        );
        assert_eq!(
            map_anchor(OverlayPosition::BottomLeft),
            HudAnchor::BottomLeft
        );
        assert_eq!(
            map_anchor(OverlayPosition::BottomCenter),
            HudAnchor::BottomCenter
        );
        assert_eq!(
            map_anchor(OverlayPosition::BottomRight),
            HudAnchor::BottomRight
        );
    }

    #[test]
    fn test_hud_config_derivation_enables_capture_exclusion() {
        let settings = OverlaySettings {
            enabled: true,
            mode: OverlayMode::Compact,
            position: OverlayPosition::TopRight,
        };
        let config = hud_config_from_settings(&settings);
        assert_eq!(config.mode, HudMode::Compact);
        assert_eq!(config.anchor, HudAnchor::TopRight);
        assert!(config.exclude_from_capture);
    }

    #[test]
    fn test_cue_for_event_relevant_variants() {
        let queued_event = ControllerEvent::SaveQueued {
            path: "C:/clips/test.mp4".to_string(),
        };
        assert_eq!(cue_for_event(&queued_event), Some(HudCue::queued()));

        let saved_event = ControllerEvent::ClipSaved {
            path: "C:/clips/test.mp4".to_string(),
            duration_ms: 5_000,
            size_bytes: 20_000_000,
        };
        assert_eq!(
            cue_for_event(&saved_event),
            Some(HudCue::saved(5_000, 20_000_000))
        );

        let failed_event = ControllerEvent::SaveFailed {
            code: "DISK_FULL".to_string(),
            message: "Not enough space".to_string(),
        };
        assert_eq!(
            cue_for_event(&failed_event),
            Some(HudCue::failed("Not enough space"))
        );

        let rejected_save = ControllerEvent::CommandRejected {
            command: "save_replay".to_string(),
            reason: "Buffer empty".to_string(),
        };
        assert_eq!(
            cue_for_event(&rejected_save),
            Some(HudCue::rejected("Buffer empty"))
        );
    }

    #[test]
    fn test_cue_for_event_irrelevant_variants_return_none() {
        let rejected_other = ControllerEvent::CommandRejected {
            command: "start_capture".to_string(),
            reason: "Already active".to_string(),
        };
        assert_eq!(cue_for_event(&rejected_other), None);

        let status_event = ControllerEvent::StatusChanged {
            state: "Ready".to_string(),
        };
        assert_eq!(cue_for_event(&status_event), None);

        let warning_event = ControllerEvent::Warning {
            code: "HOTKEY_REGISTRATION_FAILED".to_string(),
            message: "Cannot register key".to_string(),
        };
        assert_eq!(cue_for_event(&warning_event), None);
    }

    #[test]
    fn test_disabled_runtime_initialization_creates_no_window() {
        let settings = OverlaySettings {
            enabled: false,
            mode: OverlayMode::Full,
            position: OverlayPosition::BottomCenter,
        };
        let runtime = HudRuntime::init(settings);
        assert!(!runtime.is_active());
        assert_eq!(runtime.settings(), settings);
    }

    #[test]
    fn test_headless_runtime_initialization() {
        let settings = OverlaySettings {
            enabled: true,
            mode: OverlayMode::Compact,
            position: OverlayPosition::TopLeft,
        };
        let runtime = HudRuntime::headless(settings);
        assert!(!runtime.is_active());
        assert_eq!(runtime.settings(), settings);
    }

    #[test]
    fn test_disabled_reconfiguration_does_not_spawn_hud() {
        let runtime = HudRuntime::init(OverlaySettings {
            enabled: false,
            mode: OverlayMode::Full,
            position: OverlayPosition::BottomCenter,
        });
        assert!(!runtime.is_active());

        runtime.reconfigure(OverlaySettings {
            enabled: false,
            mode: OverlayMode::Compact,
            position: OverlayPosition::TopRight,
        });
        assert!(!runtime.is_active());
        assert_eq!(runtime.settings().mode, OverlayMode::Compact);
        assert_eq!(runtime.settings().position, OverlayPosition::TopRight);
    }

    #[test]
    fn test_test_cue_fails_gracefully_when_disabled_or_headless() {
        let disabled_runtime = HudRuntime::init(OverlaySettings {
            enabled: false,
            ..OverlaySettings::default()
        });
        let err = disabled_runtime.show_test_cue().unwrap_err();
        assert!(err.contains("disabled"));

        let headless_runtime = HudRuntime::headless(OverlaySettings {
            enabled: true,
            ..OverlaySettings::default()
        });
        let err = headless_runtime.show_test_cue().unwrap_err();
        assert!(err.contains("unavailable") || err.contains("failed"));
    }

    #[test]
    fn test_event_dispatch_on_headless_runtime_is_safe_noop() {
        let runtime = HudRuntime::headless(OverlaySettings::default());
        let event = ControllerEvent::ClipSaved {
            path: "test.mp4".to_string(),
            duration_ms: 2_000,
            size_bytes: 1_000,
        };
        // Calling try_dispatch_event must not panic or block
        runtime.try_dispatch_event(&event);
    }

    #[test]
    fn test_event_dispatch_under_write_lock_contention_is_safe_noop() {
        let runtime = HudRuntime::headless(OverlaySettings::default());
        let _write_guard = runtime.inner.write().unwrap();
        let event = ControllerEvent::SaveFailed {
            code: "TEST_ERR".to_string(),
            message: "Should not be cloned under contention".to_string(),
        };
        // Under write lock contention, try_dispatch_event must immediately no-op without blocking or panicking
        runtime.try_dispatch_event(&event);
    }

    #[test]
    fn test_show_test_cue_under_write_lock_returns_busy_error() {
        let runtime = HudRuntime::headless(OverlaySettings::default());
        let _write_guard = runtime.inner.write().unwrap();
        let err = runtime.show_test_cue().unwrap_err();
        assert!(err.contains("busy"));
    }
}
