use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::time_base::TimeBase;

/// Identifies one media stream within a capture/encode session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StreamId(pub u32);

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "stream-{}", self.0)
    }
}

/// Broad media class of a stream or packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Video,
    Audio,
}

/// Immutable, reference-counted packet payload bytes.
///
/// Snapshots share payloads instead of deep-copying them (spec §13.7);
/// payloads are never mutated after construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketPayload(Arc<[u8]>);

impl PacketPayload {
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of live references; snapshots can use this in tests to prove
    /// no unbounded duplication occurs.
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

impl From<Vec<u8>> for PacketPayload {
    fn from(value: Vec<u8>) -> Self {
        Self(Arc::from(value.into_boxed_slice()))
    }
}

impl From<&[u8]> for PacketPayload {
    fn from(value: &[u8]) -> Self {
        Self(Arc::from(value))
    }
}

/// One encoded access unit produced by an encoder and stored by the replay
/// buffer. Field set mirrors spec §13.7.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodedPacket {
    pub stream_id: StreamId,
    pub media_type: MediaType,
    /// Presentation timestamp in `time_base` units.
    pub pts: i64,
    /// Decode timestamp in `time_base` units; may differ from PTS for
    /// codecs with frame reordering.
    pub dts: i64,
    pub duration: i64,
    pub time_base: TimeBase,
    pub is_keyframe: bool,
    /// Monotonically increasing ingestion sequence assigned by the buffer.
    pub sequence: u64,
    pub payload: PacketPayload,
}

impl EncodedPacket {
    /// Returns the packet with its ingestion sequence set. Used once at
    /// buffer insertion (spec §15.1 step 2).
    pub fn with_sequence(mut self, sequence: u64) -> Self {
        self.sequence = sequence;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_packet() -> EncodedPacket {
        EncodedPacket {
            stream_id: StreamId(1),
            media_type: MediaType::Video,
            pts: 100,
            dts: 100,
            duration: 33,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: true,
            sequence: 0,
            payload: Vec::from(&b"payload"[..]).into(),
        }
    }

    #[test]
    fn payload_shares_without_copy() {
        let p = PacketPayload::from(vec![1_u8, 2, 3]);
        let q = p.clone();
        assert_eq!(p.as_slice(), q.as_slice());
        assert_eq!(p.strong_count(), 2);
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn with_sequence_sets_field_once() {
        let pkt = sample_packet().with_sequence(42);
        assert_eq!(pkt.sequence, 42);
    }

    #[test]
    fn stream_id_display() {
        assert_eq!(StreamId(7).to_string(), "stream-7");
    }
}
