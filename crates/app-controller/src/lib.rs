//! Application controller: owns application state, validates commands,
//! drives the recorder engine, and forwards events/notifications to the UI
//! bridge. The controller never processes media frames directly (§13.1).

pub mod commands;
pub mod controller;
pub mod events;
pub mod jobs;
pub mod notification;
pub mod single_instance;

pub use commands::Command;
pub use configuration::{AudioSourceKind, AudioTrackSettings};
pub use controller::{
    clean_game_name, pipeline_fidelity_capabilities, AudioDepsFactory, AudioTrackPlan,
    ClipAttribution, Controller, ControllerSettings, DepsFactory, FidelityAvailability,
    FidelityCapability, FidelityIssue, FidelityIssueKind, FidelityStatus,
    ARCHIVAL_PIPELINE_UNAVAILABLE_CODE, ARCHIVAL_PIPELINE_UNAVAILABLE_MESSAGE,
    DEFAULT_MINIMUM_FREE_SPACE_BYTES,
};
pub use events::ControllerEvent;
pub use jobs::{
    JobQueueError, SaveJobManager, SaveMetrics, SaveResult, SaveWork, SaveWorker, SAVE_QUEUE_BOUND,
};
pub use notification::{
    CollectingNotificationSink, ConsoleNotificationSink, NotificationContent, NotificationSeverity,
    NotificationSink,
};
pub use single_instance::{SingleInstanceError, SingleInstanceGuard};
