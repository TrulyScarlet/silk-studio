//! Backend-neutral post-save clip export contracts.
//!
//! This crate only plans an export and tracks bounded job state. It does not
//! read media, invoke an encoder, spawn FFmpeg, or claim that compression has
//! been performed.

mod planner;
mod queue;

pub use planner::{
    plan, ExportPlan, PlannerError, PlannerInput, PlannerRequest, Ratio, SourceVideo,
    TargetBytePlan, TargetBytePlanner, TargetSize, VideoRung, VideoSource, DEFAULT_VIDEO_LADDER,
    LARGE_TARGET_BYTES, MEDIUM_TARGET_BYTES, SMALL_TARGET_BYTES, TARGET_BYTES_20M,
    TARGET_BYTES_20_MB, TARGET_BYTES_500M, TARGET_BYTES_500_MB, TARGET_BYTES_50M,
    TARGET_BYTES_50_MB,
};
pub use queue::{
    CancelOutcome, CancellationToken, ExportJob, ExportJobId, ExportJobState, ExportProgress,
    ExportQueue, ExportQueueError, ExportStage, ExportStatus, QueueError, ACTIVE_JOB_LIMIT,
    EXPORT_ACTIVE_JOB_LIMIT, EXPORT_PENDING_JOB_LIMIT, EXPORT_TERMINAL_HISTORY_LIMIT,
    PENDING_JOB_LIMIT, TERMINAL_HISTORY_LIMIT,
};
