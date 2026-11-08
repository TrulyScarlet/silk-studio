//! Duration-bounded replay buffer storing encoded packets (spec §15).
//!
//! Contract highlights:
//! - Insertion validates monotonic DTS per stream and assigns the global
//!   ingestion sequence (§15.1).
//! - Eviction target is `T_b = newest_video_dts − retention`; the newest
//!   video keyframe at or before `T_b` is always retained, so every clip
//!   starts at a decodable point (§15.2, BUF-006).
//! - Snapshots take shared references only; live eviction continues while
//!   snapshots are in flight because payloads are refcounted (§15.3,
//!   BUF-007/009).
//! - Normalization rebases PTS/DTS so clips start at zero while preserving
//!   decode/presentation order (§15.4).
//!
//! Design decisions are recorded in ADR 0004.

mod buffer;
mod error;

pub use buffer::{BufferConfig, BufferMetrics, ReplayBuffer, SnapshotOutcome};
pub use error::BufferError;

use media_types::MediaSnapshot;

/// Rebase every timestamp in `snapshot` so the clip starts at zero while
/// preserving relative order and distinct PTS/DTS values (spec §15.4). Idempotent.
pub fn normalize_timestamps(snapshot: &mut MediaSnapshot) {
    snapshot.normalize_timestamps();
}
