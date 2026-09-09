use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use media_types::{
    EncodedPacket, MediaSnapshot, MediaType, PacketPayload, StreamDescriptor, StreamId,
    StreamPackets, TimeBase,
};

use crate::error::BufferError;

/// Buffer tuning. `retention_ms` is the duration bound (BUF-002/003);
/// `max_packets_per_stream` is a hard safety valve against pathological
/// input regardless of timestamps.
#[derive(Debug, Clone)]
pub struct BufferConfig {
    pub retention_ms: i64,
    pub max_packets_per_stream: usize,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            retention_ms: 60_000,
            max_packets_per_stream: 20_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BufferMetrics {
    pub packet_count: usize,
    /// Sum of payload bytes across all retained packets (excludes packet
    /// struct overhead; see ADR 0004 for the overhead factor).
    pub memory_bytes: u64,
    /// Newest minus oldest video DTS, in ticks.
    pub duration_ticks: i64,
    pub newest_video_dts: Option<i64>,
}

/// What a successful snapshot request reports (BUF-013): the requested
/// start, where the clip really starts, and the keyframe pre-roll.
#[derive(Debug, Clone)]
pub struct SnapshotOutcome {
    pub media: MediaSnapshot,
    pub requested_start_pts: i64,
    pub actual_start_pts: i64,
    /// `requested − actual ≥ 0`; how far before the requested point the
    /// keyframe forced us to start.
    pub keyframe_preroll_ticks: i64,
}

#[derive(Debug)]
struct StreamTrack {
    descriptor: StreamDescriptor,
    packets: VecDeque<Arc<EncodedPacket>>,
    last_dts: Option<i64>,
}

/// The replay buffer itself. Not internally locked: the engine owns one
/// instance behind a short mutex; sections never span I/O (ADR 0002).
#[derive(Debug)]
pub struct ReplayBuffer {
    config: BufferConfig,
    streams: BTreeMap<StreamId, StreamTrack>,
    video_stream: Option<StreamId>,
    next_sequence: u64,
    epoch: u64,
    memory_bytes: u64,
}

impl ReplayBuffer {
    pub fn new(config: BufferConfig) -> Self {
        Self {
            config,
            streams: BTreeMap::new(),
            video_stream: None,
            next_sequence: 0,
            epoch: 0,
            memory_bytes: 0,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Drop all buffered data and start a new epoch — used when codec
    /// parameters change so incompatible packets never share a clip
    /// (spec §16.3/§21.1).
    pub fn start_new_epoch(&mut self) -> u64 {
        self.streams.clear();
        self.video_stream = None;
        self.memory_bytes = 0;
        self.epoch += 1;
        self.epoch
    }

    /// Clear packet data for a discontinuity while retaining the registered
    /// stream descriptors. Recovery can therefore resume without allowing
    /// audio workers to race ahead into an unregistered stream.
    pub fn start_new_epoch_preserving_streams(&mut self) -> Result<u64, BufferError> {
        let descriptors: Vec<_> = self
            .streams
            .values()
            .map(|track| track.descriptor.clone())
            .collect();
        let epoch = self.start_new_epoch();
        for descriptor in descriptors {
            self.register_stream(descriptor)?;
        }
        Ok(epoch)
    }

    /// Register a stream and its codec descriptor before inserting into
    /// it. All streams must use the shared millisecond time base.
    pub fn register_stream(&mut self, descriptor: StreamDescriptor) -> Result<(), BufferError> {
        if descriptor.time_base != TimeBase::MILLISECOND {
            return Err(BufferError::UnsupportedTimeBase);
        }
        if descriptor.media_type == MediaType::Video {
            self.video_stream = Some(descriptor.stream_id);
        }
        self.streams.insert(
            descriptor.stream_id,
            StreamTrack {
                descriptor,
                packets: VecDeque::new(),
                last_dts: None,
            },
        );
        Ok(())
    }

    pub fn registered_streams(&self) -> usize {
        self.streams.len()
    }

    pub fn has_stream(&self, stream_id: StreamId) -> bool {
        self.streams.contains_key(&stream_id)
    }

    /// Update codec initialization bytes after a worker has configured its
    /// backend, before the first packet is inserted for that stream.
    pub fn set_stream_extradata(
        &mut self,
        stream_id: StreamId,
        extradata: Option<PacketPayload>,
    ) -> Result<(), BufferError> {
        let Some(track) = self.streams.get_mut(&stream_id) else {
            return Err(BufferError::UnregisteredStream { stream_id });
        };
        track.descriptor.extradata = extradata;
        Ok(())
    }

    /// Insert one encoded packet: validate → sequence → store → evict →
    /// account (spec §15.1 steps 1–7).
    pub fn insert(&mut self, mut packet: EncodedPacket) -> Result<(), BufferError> {
        let stream_id = packet.stream_id;
        let Some(track) = self.streams.get_mut(&stream_id) else {
            return Err(BufferError::UnregisteredStream { stream_id });
        };

        if let Some(prev) = track.last_dts {
            if packet.dts < prev {
                return Err(BufferError::NonMonotonicTimestamp {
                    stream_id,
                    previous_dts: prev,
                    new_dts: packet.dts,
                });
            }
        }

        packet.sequence = self.next_sequence;
        self.next_sequence += 1;

        let payload_bytes = packet.payload.len() as u64;
        track.last_dts = Some(packet.dts);
        track.packets.push_back(Arc::new(packet));
        self.memory_bytes += payload_bytes;

        // Absolute safety cap per stream (timestamps remain authoritative
        // for the retention window itself).
        let cap = self.config.max_packets_per_stream;
        if let Some(track) = self.streams.get_mut(&stream_id) {
            while track.packets.len() > cap {
                if let Some(dropped) = track.packets.pop_front() {
                    self.memory_bytes = self
                        .memory_bytes
                        .saturating_sub(dropped.payload.len() as u64);
                }
            }
        }

        self.evict();
        Ok(())
    }

    /// Enforce the retention window: keep everything from the newest video
    /// keyframe at or before `T_b = newest − retention`, plus overlapping
    /// audio (spec §15.2).
    fn evict(&mut self) {
        let Some(video_id) = self.video_stream else {
            return;
        };
        let Some(newest) = self.newest_video_dts() else {
            return;
        };
        let boundary = newest - self.config.retention_ms;

        // Phase 1 (immutable): locate the newest keyframe at/before T_b
        // and the current front DTS.
        let mut keep_from = None;
        let front_dts;
        {
            let Some(video_track) = self.streams.get(&video_id) else {
                return;
            };
            for (index, packet) in video_track.packets.iter().enumerate() {
                if packet.is_keyframe && packet.dts <= boundary {
                    keep_from = Some(index);
                }
            }
            front_dts = video_track.packets.front().map(|p| p.dts);
        }
        // No eligible keyframe yet: retain everything rather than produce
        // an undecodable prefix later.
        let Some(keep_from) = keep_from else { return };

        // Phase 2 (mutable): drop video prefixes, then align other streams.
        let earliest_kept_dts = if keep_from == 0 {
            // Front IS the retained keyframe.
            match front_dts {
                Some(dts) => dts,
                None => return,
            }
        } else {
            let dropped_count = keep_from;
            match self.drop_video_fronts_helper(video_id, dropped_count) {
                Some(dts) => dts,
                None => return,
            }
        };

        // Align other streams: drop audio fully before the earliest kept
        // video; partial overlap is retained for BUF-011 trimming at mux.
        for (id, track) in self.streams.iter_mut() {
            if *id == video_id {
                continue;
            }
            while let Some(front) = track.packets.front() {
                if front.dts + front.duration <= earliest_kept_dts {
                    if let Some(dropped) = track.packets.pop_front() {
                        self.memory_bytes = self
                            .memory_bytes
                            .saturating_sub(dropped.payload.len() as u64);
                    }
                } else {
                    break;
                }
            }
        }
    }

    fn drop_video_fronts_helper(&mut self, video_id: StreamId, count: usize) -> Option<i64> {
        let track = self.streams.get_mut(&video_id)?;
        for _ in 0..count {
            if let Some(dropped) = track.packets.pop_front() {
                self.memory_bytes = self
                    .memory_bytes
                    .saturating_sub(dropped.payload.len() as u64);
            }
        }
        track.packets.front().map(|front| front.dts)
    }

    /// Live retention change (BUF-012): applies immediately via re-evict.
    pub fn set_retention(&mut self, retention_ms: i64) -> Result<(), BufferError> {
        if retention_ms <= 0 {
            return Err(BufferError::RetentionTooShort {
                value_ms: retention_ms,
            });
        }
        self.config.retention_ms = retention_ms;
        self.evict();
        Ok(())
    }

    pub fn retention_ms(&self) -> i64 {
        self.config.retention_ms
    }

    /// Payload bytes currently retained (test/metrics surface).
    pub fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }

    /// Next sequence number that would be assigned (tests/telemetry).
    pub fn next_sequence_hint(&self) -> u64 {
        self.next_sequence
    }

    pub fn newest_video_dts(&self) -> Option<i64> {
        let video_id = self.video_stream?;
        let track = self.streams.get(&video_id)?;
        track.packets.back().map(|p| p.dts)
    }

    /// Presentation timestamp of the earliest retained video keyframe.
    pub fn earliest_video_keyframe_pts(&self) -> Option<i64> {
        let video_id = self.video_stream?;
        let track = self.streams.get(&video_id)?;
        track.packets.iter().find(|p| p.is_keyframe).map(|p| p.pts)
    }

    /// Buffered video span in ticks: newest DTS minus oldest retained DTS.
    pub fn duration_ticks(&self) -> i64 {
        let Some(video_id) = self.video_stream else {
            return 0;
        };
        let Some(track) = self.streams.get(&video_id) else {
            return 0;
        };
        match (track.packets.front(), track.packets.back()) {
            (Some(front), Some(back)) => (back.dts - front.dts).max(0),
            _ => 0,
        }
    }

    /// True when a full retention window of video history is buffered.
    /// Keyframe pre-roll means real availability is `retention + slack`;
    /// priming uses the plain span (documented in ADR 0004).
    pub fn is_primed(&self) -> bool {
        self.video_stream.is_some() && self.duration_ticks() >= self.config.retention_ms
    }

    pub fn metrics(&self) -> BufferMetrics {
        let video_id = self.video_stream;
        let packet_count = self.streams.values().map(|t| t.packets.len()).sum();
        BufferMetrics {
            packet_count,
            memory_bytes: self.memory_bytes,
            duration_ticks: self.duration_ticks(),
            newest_video_dts: video_id.and_then(|id| {
                self.streams
                    .get(&id)
                    .and_then(|t| t.packets.back().map(|p| p.dts))
            }),
        }
    }

    /// Build an immutable snapshot starting at the newest video keyframe
    /// at or before `requested_start_pts`, running through the newest
    /// packet (spec §15.3). Audio streams contribute every packet that
    /// overlaps the selected video interval.
    pub fn snapshot(&self, requested_start_pts: i64) -> Result<SnapshotOutcome, BufferError> {
        let Some(video_id) = self.video_stream else {
            return Err(BufferError::NothingBuffered);
        };
        let Some(video_track) = self.streams.get(&video_id) else {
            return Err(BufferError::NothingBuffered);
        };
        if video_track.packets.is_empty() {
            return Err(BufferError::NothingBuffered);
        }

        let mut kf_index = None;
        for (index, packet) in video_track.packets.iter().enumerate() {
            if packet.is_keyframe && packet.pts <= requested_start_pts {
                kf_index = Some(index);
            }
        }
        let Some(kf_index) = kf_index else {
            return Err(BufferError::NoKeyframeAtOrBefore {
                requested_start_pts,
            });
        };

        let first_keyframe = video_track.packets[kf_index].clone();
        let actual_start_pts = first_keyframe.pts.min(first_keyframe.dts);
        let preroll = requested_start_pts - actual_start_pts;

        let video_packets: Vec<Arc<EncodedPacket>> =
            video_track.packets.iter().skip(kf_index).cloned().collect();

        let interval_end = video_packets
            .last()
            .map(|p| p.dts + p.duration)
            .unwrap_or(actual_start_pts);

        let mut streams = vec![StreamPackets {
            descriptor: video_track.descriptor.clone(),
            packets: video_packets,
        }];

        for track in self.streams.values() {
            if track.descriptor.stream_id == video_id {
                continue;
            }
            let overlapping: Vec<Arc<EncodedPacket>> = track
                .packets
                .iter()
                .filter(|p| p.dts + p.duration > actual_start_pts && p.dts < interval_end)
                .cloned()
                .collect();
            if !overlapping.is_empty() {
                streams.push(StreamPackets {
                    descriptor: track.descriptor.clone(),
                    packets: overlapping,
                });
            }
        }

        Ok(SnapshotOutcome {
            requested_start_pts,
            actual_start_pts,
            keyframe_preroll_ticks: preroll,
            media: MediaSnapshot {
                origin_pts: actual_start_pts,
                time_base: TimeBase::MILLISECOND,
                streams,
                captured_at_unix_ms: 0, // set by the caller at save time
            },
        })
    }
}
