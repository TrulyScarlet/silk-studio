//! Clip muxing contract (spec §13.8).
//!
//! A muxer receives an immutable [`MediaSnapshot`], writes it to a
//! temporary output, validates basic structural properties, and publishes
//! the final file atomically. It never re-encodes media (MUX-004).
//!
//! Contract obligations:
//! 1. Write to a distinct temporary name first (MUX-006).
//! 2. Normalize timestamps to begin at zero (spec §15.4).
//! 3. Preserve decode/presentation ordering.
//! 4. Validate readability before publication (MUX-005).
//! 5. Publish with an atomic rename when feasible (MUX-007).

mod mp4_muxer;

pub use mp4_muxer::Mp4Muxer;

use media_types::{MediaSnapshot, MediaType, TimeBase};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub type Result<T, E = MuxerError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum MuxerError {
    #[error("muxer initialization failed: {details}")]
    InitializationFailed { details: String },

    #[error("failed writing clip data: {details}")]
    WriteFailed { details: String },

    #[error("output validation failed: {reason}")]
    ValidationFailed { reason: String },

    #[error("snapshot contains no writable streams")]
    NothingToWrite,

    #[error("output I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("output directory is unavailable for {path}: {source}")]
    OutputDirectoryUnavailable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "insufficient disk space for {path}: {available_bytes} bytes available, {required_bytes} required"
    )]
    InsufficientDiskSpace {
        path: PathBuf,
        available_bytes: u64,
        required_bytes: u64,
    },
}

impl MuxerError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InitializationFailed { .. } => "MUXER_INITIALIZATION_FAILED",
            Self::WriteFailed { .. } => "MUXER_WRITE_FAILED",
            Self::ValidationFailed { .. } => "OUTPUT_VALIDATION_FAILED",
            Self::NothingToWrite => "OUTPUT_VALIDATION_FAILED",
            Self::Io { .. } => "MUXER_IO_FAILED",
            Self::OutputDirectoryUnavailable { .. } => "OUTPUT_DIRECTORY_UNAVAILABLE",
            Self::InsufficientDiskSpace { .. } => "INSUFFICIENT_DISK_SPACE",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveOptions {}

/// Metadata describing a successfully published clip file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipMetadata {
    pub path: PathBuf,
    pub duration_ms: u64,
    pub size_bytes: u64,
    pub created_at_unix_ms: u64,
    pub video_codec: Option<String>,
    pub audio_codecs: Vec<String>,
}

/// Original muxing interface. Implementations run on save-worker threads
/// and must not block buffer ingestion (BUF-007).
pub trait Muxer: Send {
    /// Write `snapshot` to `final_path`, staging + validating internally.
    /// On success the final path exists and is complete; on failure no
    /// complete file appears at that path.
    fn write_snapshot(
        &mut self,
        snapshot: &MediaSnapshot,
        final_path: &Path,
        options: &SaveOptions,
    ) -> Result<ClipMetadata>;
}

/// Validate the immutable snapshot before a backend performs container I/O.
///
/// The replay buffer normally establishes these invariants, but the muxer is
/// also an independent trust boundary: snapshots can be produced by recovery
/// or test implementations. This function deliberately does not require zero
/// timestamps because normalization belongs at the mux boundary.
pub fn validate_snapshot(snapshot: &MediaSnapshot) -> Result<()> {
    if snapshot.time_base != TimeBase::MILLISECOND {
        return Err(MuxerError::ValidationFailed {
            reason: "snapshot must use the shared millisecond time base".to_string(),
        });
    }

    let mut stream_ids = BTreeSet::new();
    let mut has_video = false;
    let mut writable_streams = 0_usize;

    for stream in &snapshot.streams {
        if !stream_ids.insert(stream.descriptor.stream_id) {
            return Err(MuxerError::ValidationFailed {
                reason: format!(
                    "snapshot contains duplicate stream {}",
                    stream.descriptor.stream_id
                ),
            });
        }
        if stream.descriptor.time_base != snapshot.time_base {
            return Err(MuxerError::ValidationFailed {
                reason: format!(
                    "stream {} does not use the snapshot time base",
                    stream.descriptor.stream_id
                ),
            });
        }
        if stream.packets.is_empty() {
            continue;
        }

        writable_streams += 1;
        if stream.descriptor.media_type == MediaType::Video {
            has_video = true;
            if !stream.packets[0].is_keyframe {
                return Err(MuxerError::ValidationFailed {
                    reason: "video stream must begin with a keyframe".to_string(),
                });
            }
        }
        if stream.descriptor.media_type == MediaType::Audio
            && (stream.descriptor.sample_rate.unwrap_or(0) == 0
                || stream.descriptor.channels.unwrap_or(0) == 0)
        {
            return Err(MuxerError::ValidationFailed {
                reason: format!(
                    "audio stream {} is missing a positive sample rate or channel count",
                    stream.descriptor.stream_id
                ),
            });
        }

        let mut previous_dts = None;
        for packet in &stream.packets {
            if packet.stream_id != stream.descriptor.stream_id {
                return Err(MuxerError::ValidationFailed {
                    reason: format!(
                        "packet stream {} does not match descriptor stream {}",
                        packet.stream_id, stream.descriptor.stream_id
                    ),
                });
            }
            if packet.media_type != stream.descriptor.media_type {
                return Err(MuxerError::ValidationFailed {
                    reason: format!(
                        "packet media type does not match stream {}",
                        stream.descriptor.stream_id
                    ),
                });
            }
            if packet.time_base != stream.descriptor.time_base {
                return Err(MuxerError::ValidationFailed {
                    reason: format!(
                        "packet time base does not match stream {}",
                        stream.descriptor.stream_id
                    ),
                });
            }
            if packet.duration <= 0 {
                return Err(MuxerError::ValidationFailed {
                    reason: format!(
                        "stream {} contains a non-positive packet duration",
                        stream.descriptor.stream_id
                    ),
                });
            }
            if let Some(previous_dts) = previous_dts {
                if packet.dts < previous_dts {
                    return Err(MuxerError::ValidationFailed {
                        reason: format!(
                            "stream {} decode timestamps are not monotonic",
                            stream.descriptor.stream_id
                        ),
                    });
                }
            }
            previous_dts = Some(packet.dts);
        }
    }

    if writable_streams == 0 {
        return Err(MuxerError::NothingToWrite);
    }
    if !has_video {
        return Err(MuxerError::ValidationFailed {
            reason: "snapshot has no writable video stream".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_types::{EncodedPacket, PacketPayload, StreamDescriptor, StreamId, StreamPackets};
    use std::sync::Arc;

    const VIDEO: StreamId = StreamId(0);
    const AUDIO: StreamId = StreamId(1);

    fn descriptor(stream_id: StreamId, media_type: MediaType) -> StreamDescriptor {
        StreamDescriptor {
            stream_id,
            media_type,
            time_base: TimeBase::MILLISECOND,
            name: None,
            codec: match media_type {
                MediaType::Video => "h264".to_string(),
                MediaType::Audio => "aac".to_string(),
            },
            extradata: None,
            width: None,
            height: None,
            sample_rate: (media_type == MediaType::Audio).then_some(48_000),
            channels: (media_type == MediaType::Audio).then_some(2),
            pixel_format: None,
        }
    }

    fn packet(
        stream_id: StreamId,
        media_type: MediaType,
        dts: i64,
        keyframe: bool,
    ) -> Arc<EncodedPacket> {
        Arc::new(EncodedPacket {
            stream_id,
            media_type,
            pts: dts,
            dts,
            duration: 20,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: keyframe,
            sequence: 0,
            payload: PacketPayload::from(&[1_u8][..]),
        })
    }

    fn snapshot(video_packets: Vec<Arc<EncodedPacket>>) -> MediaSnapshot {
        MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: descriptor(VIDEO, MediaType::Video),
                packets: video_packets,
            }],
            captured_at_unix_ms: 0,
        }
    }

    #[test]
    fn accepts_video_and_audio_with_matching_metadata() {
        let mut snapshot = snapshot(vec![packet(VIDEO, MediaType::Video, 0, true)]);
        snapshot.streams.push(StreamPackets {
            descriptor: descriptor(AUDIO, MediaType::Audio),
            packets: vec![packet(AUDIO, MediaType::Audio, 0, true)],
        });
        assert!(validate_snapshot(&snapshot).is_ok());
    }

    #[test]
    fn rejects_empty_or_video_less_snapshots() {
        assert!(matches!(
            validate_snapshot(&snapshot(Vec::new())),
            Err(MuxerError::NothingToWrite)
        ));

        let audio_only = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: descriptor(AUDIO, MediaType::Audio),
                packets: vec![packet(AUDIO, MediaType::Audio, 0, true)],
            }],
            captured_at_unix_ms: 0,
        };
        assert!(matches!(
            validate_snapshot(&audio_only),
            Err(MuxerError::ValidationFailed { .. })
        ));
    }

    #[test]
    fn rejects_mismatched_metadata_and_non_monotonic_decode_order() {
        let mut mismatch = snapshot(vec![packet(StreamId(9), MediaType::Video, 0, true)]);
        assert!(matches!(
            validate_snapshot(&mismatch),
            Err(MuxerError::ValidationFailed { .. })
        ));

        mismatch.streams[0].packets = vec![
            packet(VIDEO, MediaType::Video, 20, true),
            packet(VIDEO, MediaType::Video, 10, false),
        ];
        assert!(matches!(
            validate_snapshot(&mismatch),
            Err(MuxerError::ValidationFailed { .. })
        ));
    }

    #[test]
    fn requires_a_keyframe_and_positive_audio_parameters() {
        let non_keyframe = snapshot(vec![packet(VIDEO, MediaType::Video, 0, false)]);
        assert!(matches!(
            validate_snapshot(&non_keyframe),
            Err(MuxerError::ValidationFailed { .. })
        ));

        let mut invalid_audio = snapshot(vec![packet(VIDEO, MediaType::Video, 0, true)]);
        let mut audio_descriptor = descriptor(AUDIO, MediaType::Audio);
        audio_descriptor.sample_rate = Some(0);
        invalid_audio.streams.push(StreamPackets {
            descriptor: audio_descriptor,
            packets: vec![packet(AUDIO, MediaType::Audio, 0, true)],
        });
        assert!(matches!(
            validate_snapshot(&invalid_audio),
            Err(MuxerError::ValidationFailed { .. })
        ));
    }
}
