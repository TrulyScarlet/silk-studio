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

use media_types::{EncodedPacket, MediaSnapshot};

/// Rebase every timestamp in `snapshot` so the clip starts at zero while
/// preserving relative order and distinct PTS/DTS values (spec §15.4).
///
/// The shift is the smallest PTS/DTS/origin in the snapshot (which may be
/// negative when display timestamps jitter before stream start); after
/// shifting, every timestamp is ≥ 0. Idempotent.
pub fn normalize_timestamps(snapshot: &mut MediaSnapshot) {
    let mut shift = snapshot.origin_pts;
    for stream in &snapshot.streams {
        if let Some(first) = stream.packets.first() {
            shift = shift.min(first.dts).min(first.pts);
        }
    }
    if shift == 0 {
        return;
    }
    for stream in &mut snapshot.streams {
        for packet in &mut stream.packets {
            if let Some(packet) = std::sync::Arc::get_mut(packet) {
                packet.pts -= shift;
                packet.dts -= shift;
            } else {
                // Shared elsewhere; replace with a shifted copy instead of
                // mutating in place (never legal under our refcounts).
                let mut copy = (**packet).clone();
                copy.pts -= shift;
                copy.dts -= shift;
                *packet = std::sync::Arc::new(copy);
            }
        }
    }
    snapshot.origin_pts -= shift;
}

/// Convenience: normalized deep view of one packet for validation/tests.
pub fn normalized_pts(packet: &EncodedPacket, origin: i64) -> i64 {
    packet.pts - origin
}
