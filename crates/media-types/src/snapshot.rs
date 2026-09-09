use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::frame::PixelFormat;
use crate::packet::{EncodedPacket, MediaType, PacketPayload, StreamId};
use crate::time_base::TimeBase;

/// Static description of one stream inside a snapshot handed to the muxer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamDescriptor {
    pub stream_id: StreamId,
    pub media_type: MediaType,
    pub time_base: TimeBase,
    /// Optional human-readable track name for container metadata.
    #[serde(default)]
    pub name: Option<String>,
    /// Codec identifier such as `h264` or `aac`.
    pub codec: String,
    /// Codec-specific initialization bytes when the codec requires them.
    pub extradata: Option<PacketPayload>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub pixel_format: Option<PixelFormat>,
}

/// Packets belonging to one stream of a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamPackets {
    pub descriptor: StreamDescriptor,
    pub packets: Vec<Arc<EncodedPacket>>,
}

/// An immutable replay snapshot (spec §15.3): the unit of work submitted to
/// mux workers. Payloads inside packets are shared references; creating a
/// snapshot does not deep-copy encoded data.
///
/// Wall-clock capture time is metadata only — ordering always follows the
/// monotonic timeline carried by packet timestamps (spec §13.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSnapshot {
    /// Timeline origin subtracted during muxing so clip timestamps start
    /// at zero (spec §15.4).
    pub origin_pts: i64,
    pub time_base: TimeBase,
    pub streams: Vec<StreamPackets>,
    /// Wall-clock creation time as Unix epoch milliseconds, metadata only.
    pub captured_at_unix_ms: u64,
}

impl MediaSnapshot {
    /// Total number of packets across streams.
    pub fn packet_count(&self) -> usize {
        self.streams.iter().map(|s| s.packets.len()).sum()
    }

    /// Duration in `time_base` units covered by the video stream, if any.
    pub fn video_duration_ticks(&self) -> Option<i64> {
        let video = self
            .streams
            .iter()
            .find(|s| s.descriptor.media_type == MediaType::Video)?;
        let last = video.packets.last()?;
        Some(last.pts.saturating_sub(self.origin_pts) + last.duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video_packet(pts: i64) -> Arc<EncodedPacket> {
        Arc::new(EncodedPacket {
            stream_id: StreamId(0),
            media_type: MediaType::Video,
            pts,
            dts: pts,
            duration: 33,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: pts == 0,
            sequence: 0,
            payload: PacketPayload::from(&b"x"[..]),
        })
    }

    #[test]
    fn counts_packets_and_duration() {
        let snap = MediaSnapshot {
            origin_pts: 100,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: StreamDescriptor {
                    stream_id: StreamId(0),
                    media_type: MediaType::Video,
                    time_base: TimeBase::MILLISECOND,
                    name: None,
                    codec: "h264".to_string(),
                    extradata: None,
                    width: None,
                    height: None,
                    sample_rate: None,
                    channels: None,
                    pixel_format: None,
                },
                packets: vec![video_packet(100), video_packet(133), video_packet(166)],
            }],
            captured_at_unix_ms: 0,
        };
        assert_eq!(snap.packet_count(), 3);
        assert_eq!(snap.video_duration_ticks(), Some(99));
    }
}
