//! Recorder engine: owns the recorder state machine (spec §19), drives
//! capture → encode workers, and hands replay snapshots to a muxer.
//!
//! Threading baseline (ADR 0002): the control methods run on the caller's
//! thread; one dedicated video worker exists per running session; all
//! cross-thread handoffs use bounded channels or short critical sections.
//! The full worker set from spec §14 lands incrementally through S4.

mod engine;
mod error;
mod state;

pub use engine::{
    AudioInput, EngineConfig, EngineEvent, EngineScalingPlan, RecorderEngine,
    ScalingFallbackReason, VideoScalingState, VideoScalingStatus, CLARITY_DIMENSIONS_OVERFLOW_CODE,
    CLARITY_DIMENSIONS_OVERFLOW_MESSAGE, CLARITY_ENCODER_NEGOTIATION_FAILED_CODE,
    CLARITY_ENCODER_NEGOTIATION_FAILED_MESSAGE, CLARITY_GPU_CONTEXT_UNAVAILABLE_CODE,
    CLARITY_GPU_CONTEXT_UNAVAILABLE_MESSAGE, CLARITY_STARTUP_PROBE_FAILED_CODE,
    CLARITY_STARTUP_PROBE_FAILED_MESSAGE, ENGINE_EVENT_QUEUE_BOUND, VIDEO_RESTART_LIMIT,
};
pub use error::EngineError;
pub use state::{allowed_transitions, validate_transition, InvalidTransition, RecorderState};
