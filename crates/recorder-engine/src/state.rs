//! Recorder lifecycle states and validated transitions (spec §19).
//!
//! Spec-listed edges are reproduced exactly. Documented additions
//! (ADR 0003): `Degraded` participates like `Ready` for save/recover/
//! stop purposes, and every active state may abort into `Stopping`,
//! including `Error`.

use std::fmt;

/// Explicit recorder states shown by the UI (spec §9.2).
///
/// `Buffering` means capture is live but a full replay-duration history
/// has not accumulated yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecorderState {
    Stopped,
    Starting,
    Buffering,
    Ready,
    Saving,
    Degraded,
    Recovering,
    Error,
    Stopping,
}

impl RecorderState {
    /// Every declared state; used by exhaustive tests.
    pub const ALL: [RecorderState; 9] = [
        Self::Stopped,
        Self::Starting,
        Self::Buffering,
        Self::Ready,
        Self::Saving,
        Self::Degraded,
        Self::Recovering,
        Self::Error,
        Self::Stopping,
    ];

    pub fn is_active(self) -> bool {
        !matches!(self, Self::Stopped)
    }

    /// Whether a replay save may be requested in this state (BUF-007:
    /// saving never stops capture).
    pub fn can_save(self) -> bool {
        matches!(self, Self::Ready | Self::Degraded)
    }
}

impl fmt::Display for RecorderState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Stopped => "Stopped",
            Self::Starting => "Starting",
            Self::Buffering => "Buffering",
            Self::Ready => "Ready",
            Self::Saving => "Saving",
            Self::Degraded => "Degraded",
            Self::Recovering => "Recovering",
            Self::Error => "Error",
            Self::Stopping => "Stopping",
        };
        f.write_str(name)
    }
}

/// Structured rejection of an illegal state change (spec §19: invalid
/// transitions must return structured errors).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid recorder state transition from '{from}' to '{to}'")]
pub struct InvalidTransition {
    pub from: RecorderState,
    pub to: RecorderState,
}

/// Static transition table — the single source of truth.
pub fn allowed_transitions(from: RecorderState) -> &'static [RecorderState] {
    match from {
        RecorderState::Stopped => &[RecorderState::Starting],
        RecorderState::Starting => &[
            RecorderState::Buffering,
            RecorderState::Error,
            RecorderState::Stopping,
        ],
        RecorderState::Buffering => &[
            RecorderState::Ready,
            RecorderState::Degraded,
            RecorderState::Recovering,
            RecorderState::Error,
            RecorderState::Stopping,
        ],
        RecorderState::Ready => &[
            RecorderState::Saving,
            RecorderState::Degraded,
            RecorderState::Recovering,
            RecorderState::Error,
            RecorderState::Stopping,
        ],
        RecorderState::Saving => &[
            RecorderState::Ready,
            RecorderState::Degraded,
            RecorderState::Error,
            RecorderState::Stopping,
        ],
        RecorderState::Degraded => &[
            RecorderState::Ready,
            RecorderState::Saving,
            RecorderState::Recovering,
            RecorderState::Error,
            RecorderState::Stopping,
        ],
        RecorderState::Recovering => &[
            RecorderState::Buffering,
            RecorderState::Ready,
            RecorderState::Degraded,
            RecorderState::Error,
            RecorderState::Stopping,
        ],
        RecorderState::Error => &[RecorderState::Starting, RecorderState::Stopping],
        RecorderState::Stopping => &[RecorderState::Stopped],
    }
}

/// Check whether `from -> to` is legal.
pub fn validate_transition(
    from: RecorderState,
    to: RecorderState,
) -> Result<(), InvalidTransition> {
    if allowed_transitions(from).contains(&to) {
        Ok(())
    } else {
        Err(InvalidTransition { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RecorderState as S;

    #[test]
    fn spec_listed_edges_are_valid() {
        let spec_edges = [
            (S::Stopped, S::Starting),
            (S::Starting, S::Buffering),
            (S::Buffering, S::Ready),
            (S::Buffering, S::Recovering),
            (S::Ready, S::Saving),
            (S::Saving, S::Ready),
            (S::Ready, S::Recovering),
            (S::Recovering, S::Ready),
            (S::Recovering, S::Buffering),
            (S::Starting, S::Error),
            (S::Buffering, S::Error),
            (S::Ready, S::Error),
            (S::Error, S::Starting),
            (S::Starting, S::Stopping),
        ];
        for (from, to) in spec_edges {
            assert!(
                validate_transition(from, to).is_ok(),
                "spec edge {from} -> {to} must be allowed"
            );
        }
    }

    #[test]
    fn every_active_state_aborts_into_stopping_then_stopped() {
        for state in S::ALL
            .iter()
            .copied()
            .filter(|s| s.is_active() && *s != S::Stopping)
        {
            assert!(
                validate_transition(state, S::Stopping).is_ok(),
                "{state} must be able to stop"
            );
        }
        assert!(validate_transition(S::Stopping, S::Stopping).is_err());
        assert!(validate_transition(S::Stopped, S::Stopping).is_err());
        assert!(validate_transition(S::Stopping, S::Stopped).is_ok());
        assert!(validate_transition(S::Stopping, S::Starting).is_err());
    }

    #[test]
    fn representative_invalid_edges_rejected() {
        let invalid = [
            (S::Stopped, S::Ready),
            (S::Stopped, S::Saving),
            (S::Ready, S::Buffering),
            (S::Ready, S::Starting),
            (S::Saving, S::Saving),
            (S::Error, S::Ready),
            (S::Error, S::Saving),
            (S::Buffering, S::Saving),
            (S::Recovering, S::Saving),
        ];
        for (from, to) in invalid {
            assert!(
                validate_transition(from, to).is_err(),
                "edge {from} -> {to} must be rejected"
            );
        }
    }

    #[test]
    fn degraded_behaves_like_ready_for_saves() {
        assert!(S::Ready.can_save());
        assert!(S::Degraded.can_save());
        assert!(!S::Buffering.can_save());
        assert!(!S::Saving.can_save());
        assert!(!S::Error.can_save());
    }

    #[test]
    fn table_is_total_over_all_states() {
        for from in S::ALL {
            let targets = allowed_transitions(from);
            assert!(!targets.is_empty(), "{from} has no outgoing edges");
            for target in targets {
                assert!(
                    S::ALL.contains(target),
                    "edge {from} -> {target} references undeclared state"
                );
            }
        }
    }

    #[test]
    fn display_names_match_ui_labels() {
        assert_eq!(S::Buffering.to_string(), "Buffering");
        assert_eq!(S::Degraded.to_string(), "Degraded");
    }
}
