//! Direct MP4 muxer adapter using the pure-Rust `mp4` crate (version 0.14.0).
//!
//! # Box Ordering and FastStart Note
//!
//! The `mp4` crate generates files with standard top-level box ordering:
//! `ftyp -> mdat -> moov`.
//!
//! Unlike "FastStart" / `qt-faststart` MP4 files (which place the `moov` header
//! before the `mdat` media data for progressive HTTP streaming), `ftyp -> mdat -> moov`
//! places the movie metadata box (`moov`) at the end of the file.
//!
//! For Silk's instant-replay clips:
//! 1. All clips are authored directly to the local filesystem for local playback,
//!    editing, and archiving.
//! 2. Random-access media readers seek directly to the `moov` box at the end
//!    of local files via standard file seeking without downloading over a network.
//! 3. Writing `moov` at the end enables single-pass sequential recording without
//!    requiring speculative box reservation or a secondary disk-rewrite pass,
//!    maximizing I/O throughput and responsiveness during instant-replay clipping.

use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use media_types::{MediaSnapshot, MediaType, StreamId, StreamPackets};
use mp4::{
    AacConfig, AudioObjectType, Av1Config, AvcConfig, ChannelConfig, FourCC, HevcConfig,
    MediaConfig, Mp4Config, Mp4Reader, Mp4Sample, Mp4Writer, SampleFreqIndex, TrackConfig,
    TrackType,
};

use crate::{validate_snapshot, ClipMetadata, Muxer, MuxerError, Result, SaveOptions};

/// Conservative maximum payload size to guarantee 32-bit container header safety.
pub const MAX_SAFE_MP4_PAYLOAD_BYTES: u64 = 0xFFFF_0000;

/// Number of PCM samples represented by one standard AAC-LC access unit.
pub const AAC_LC_SAMPLES_PER_FRAME: u32 = 1024;

/// Direct MP4 muxer implementation wrapping pure-Rust `mp4` crate.
#[derive(Debug, Default)]
pub struct Mp4Muxer;

struct StagedFileGuard<'a> {
    path: &'a Path,
    published: bool,
}

impl<'a> StagedFileGuard<'a> {
    fn new(path: &'a Path) -> Self {
        Self {
            path,
            published: false,
        }
    }

    fn mark_published(&mut self) {
        self.published = true;
    }
}

impl Drop for StagedFileGuard<'_> {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(self.path);
        }
    }
}

struct PreparedTrack {
    stream_index: usize,
    stream_id: StreamId,
    track_id: u32,
    media_type: MediaType,
    codec: String,
    timescale: u32,
    config: TrackConfig,
}

struct PreparedSample {
    track_id: u32,
    dts_ms: i64,
    sequence: u64,
    stream_index: usize,
    sample: Mp4Sample,
}

impl Muxer for Mp4Muxer {
    fn write_snapshot(
        &mut self,
        snapshot: &MediaSnapshot,
        final_path: &Path,
        _options: &SaveOptions,
    ) -> Result<ClipMetadata> {
        let temp_path = staged_path(final_path);
        let mut guard = StagedFileGuard::new(&temp_path);

        validate_snapshot(snapshot)?;
        check_total_payload_size(snapshot)?;
        validate_codecs(snapshot)?;

        let normalized_snapshot = normalize_snapshot(snapshot)?;
        let prepared_tracks = prepare_mp4_tracks(&normalized_snapshot)?;

        if let Some(parent) = final_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
        }

        write_staged_mp4(&temp_path, &normalized_snapshot, &prepared_tracks)?;
        validate_staged_mp4_file(&temp_path, &prepared_tracks)?;

        let size_bytes = fs::metadata(&temp_path)
            .map(|m| m.len())
            .map_err(|source| io_error(&temp_path, source))?;

        fs::rename(&temp_path, final_path).map_err(|source| io_error(final_path, source))?;
        guard.mark_published();

        let video = prepared_tracks
            .iter()
            .find(|t| t.media_type == MediaType::Video);
        let audio_codecs: Vec<String> = prepared_tracks
            .iter()
            .filter(|t| t.media_type == MediaType::Audio)
            .map(|t| t.codec.clone())
            .collect();

        Ok(ClipMetadata {
            path: final_path.to_path_buf(),
            duration_ms: snapshot_duration_ms(&normalized_snapshot),
            size_bytes,
            created_at_unix_ms: snapshot.captured_at_unix_ms,
            video_codec: video.map(|t| t.codec.clone()),
            audio_codecs,
        })
    }
}

pub fn check_total_payload_size(snapshot: &MediaSnapshot) -> Result<u64> {
    let mut total_bytes = 0_u64;
    for stream in &snapshot.streams {
        for packet in &stream.packets {
            let packet_len = u64::try_from(packet.payload.len())
                .map_err(|_| validation_error("packet payload length overflow"))?;
            total_bytes = total_bytes
                .checked_add(packet_len)
                .ok_or_else(|| validation_error("total snapshot payload size overflowed u64"))?;
            if total_bytes > MAX_SAFE_MP4_PAYLOAD_BYTES {
                return Err(validation_error(format!(
                    "snapshot total payload size ({total_bytes} bytes) exceeds the maximum safe 32-bit MP4 container limit ({MAX_SAFE_MP4_PAYLOAD_BYTES} bytes)"
                )));
            }
        }
    }
    Ok(total_bytes)
}

fn validation_error(reason: impl Into<String>) -> MuxerError {
    MuxerError::ValidationFailed {
        reason: reason.into(),
    }
}

fn write_error(details: impl Into<String>) -> MuxerError {
    MuxerError::WriteFailed {
        details: details.into(),
    }
}

fn io_error(path: &Path, source: io::Error) -> MuxerError {
    MuxerError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn staged_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VideoCodec {
    Avc,
    Hevc,
    Av1,
}

impl VideoCodec {
    fn from_str(codec: &str) -> Option<Self> {
        match codec.to_ascii_lowercase().as_str() {
            "h264" | "avc" | "avc1" => Some(Self::Avc),
            "h265" | "hevc" | "hvc1" => Some(Self::Hevc),
            "av1" | "av01" => Some(Self::Av1),
            _ => None,
        }
    }
}

fn is_aac_codec(codec: &str) -> bool {
    matches!(codec.to_ascii_lowercase().as_str(), "aac" | "mp4a")
}

fn validate_codecs(snapshot: &MediaSnapshot) -> Result<()> {
    for stream in &snapshot.streams {
        if stream.packets.is_empty() {
            continue;
        }
        match stream.descriptor.media_type {
            MediaType::Video => match VideoCodec::from_str(&stream.descriptor.codec) {
                Some(VideoCodec::Avc) | Some(VideoCodec::Hevc) | Some(VideoCodec::Av1) => {}
                None => {
                    return Err(validation_error(format!(
                        "video stream {} uses unsupported codec '{}'; MP4 muxer requires H.264/AVC, H.265/HEVC, or AV1",
                        stream.descriptor.stream_id, stream.descriptor.codec
                    )));
                }
            },
            MediaType::Audio => {
                if !is_aac_codec(&stream.descriptor.codec) {
                    return Err(validation_error(format!(
                        "audio stream {} uses unsupported codec '{}'; MP4 muxer requires AAC",
                        stream.descriptor.stream_id, stream.descriptor.codec
                    )));
                }
            }
        }
    }
    Ok(())
}

fn normalize_snapshot(snapshot: &MediaSnapshot) -> Result<MediaSnapshot> {
    let mut shift = snapshot.origin_pts;
    for stream in &snapshot.streams {
        if let Some(first) = stream.packets.first() {
            shift = shift.min(first.dts).min(first.pts);
        }
    }

    let mut streams = Vec::with_capacity(snapshot.streams.len());
    for stream in &snapshot.streams {
        let mut packets = Vec::with_capacity(stream.packets.len());
        for packet in &stream.packets {
            let normalized_dts = packet
                .dts
                .checked_sub(shift)
                .ok_or_else(|| validation_error("DTS underflow during normalization"))?;
            if normalized_dts < 0 {
                return Err(validation_error(format!(
                    "post-normalization DTS is negative: {}",
                    normalized_dts
                )));
            }

            let normalized_pts = packet
                .pts
                .checked_sub(shift)
                .ok_or_else(|| validation_error("PTS underflow during normalization"))?;
            if normalized_pts < 0 {
                return Err(validation_error(format!(
                    "post-normalization PTS is negative: {}",
                    normalized_pts
                )));
            }

            let mut copy = (**packet).clone();
            copy.dts = normalized_dts;
            copy.pts = normalized_pts;
            packets.push(std::sync::Arc::new(copy));
        }
        streams.push(StreamPackets {
            descriptor: stream.descriptor.clone(),
            packets,
        });
    }

    let normalized = MediaSnapshot {
        origin_pts: 0,
        time_base: snapshot.time_base,
        streams,
        captured_at_unix_ms: snapshot.captured_at_unix_ms,
    };
    validate_snapshot(&normalized)?;
    Ok(normalized)
}

fn prepare_mp4_tracks(snapshot: &MediaSnapshot) -> Result<Vec<PreparedTrack>> {
    let mut video_indices: Vec<usize> = Vec::new();
    let mut audio_indices: Vec<usize> = Vec::new();

    for (index, stream) in snapshot.streams.iter().enumerate() {
        if stream.packets.is_empty() {
            continue;
        }
        match stream.descriptor.media_type {
            MediaType::Video => video_indices.push(index),
            MediaType::Audio => audio_indices.push(index),
        }
    }

    if video_indices.is_empty() {
        return Err(validation_error(
            "snapshot contains no writable video stream",
        ));
    }
    if video_indices.len() > 1 {
        return Err(validation_error(format!(
            "snapshot contains {} video streams; MP4 muxer supports exactly 1 video stream",
            video_indices.len()
        )));
    }

    let mut tracks = Vec::with_capacity(video_indices.len() + audio_indices.len());

    // 1. Add video track (timescale = 1000 for 1 tick = 1 ms)
    let video_stream_idx = video_indices[0];
    let video_stream = &snapshot.streams[video_stream_idx];
    let video_desc = &video_stream.descriptor;

    let width = video_desc.width.unwrap_or(0);
    let height = video_desc.height.unwrap_or(0);
    if width == 0 || height == 0 {
        return Err(validation_error(format!(
            "video stream {} is missing positive dimensions",
            video_desc.stream_id
        )));
    }
    let width_u16 = u16::try_from(width).map_err(|_| {
        validation_error(format!(
            "video stream {} width {} exceeds u16 range",
            video_desc.stream_id, width
        ))
    })?;
    let height_u16 = u16::try_from(height).map_err(|_| {
        validation_error(format!(
            "video stream {} height {} exceeds u16 range",
            video_desc.stream_id, height
        ))
    })?;

    let video_codec = VideoCodec::from_str(&video_desc.codec).ok_or_else(|| {
        validation_error(format!("unsupported video codec '{}'", video_desc.codec))
    })?;

    let media_conf = match video_codec {
        VideoCodec::Avc => {
            let (sps, pps) = extract_sps_pps(video_stream)?;
            if sps.len() < 4 {
                return Err(validation_error(format!(
                    "video stream {} SPS length is {} bytes, which is shorter than required 4 bytes",
                    video_desc.stream_id,
                    sps.len()
                )));
            }
            if pps.is_empty() {
                return Err(validation_error(format!(
                    "video stream {} PPS is empty",
                    video_desc.stream_id
                )));
            }

            let avc_config = AvcConfig {
                width: width_u16,
                height: height_u16,
                seq_param_set: sps,
                pic_param_set: pps,
            };
            MediaConfig::AvcConfig(avc_config)
        }
        VideoCodec::Hevc => {
            let hevc_config = extract_hevc_config(video_stream, width_u16, height_u16)?;
            MediaConfig::HevcConfig(hevc_config)
        }
        VideoCodec::Av1 => {
            let av1_config = extract_av1_config(video_stream, width_u16, height_u16)?;
            MediaConfig::Av1Config(av1_config)
        }
    };

    tracks.push(PreparedTrack {
        stream_index: video_stream_idx,
        stream_id: video_desc.stream_id,
        track_id: 1,
        media_type: MediaType::Video,
        codec: video_desc.codec.clone(),
        timescale: 1000,
        config: TrackConfig {
            track_type: TrackType::Video,
            timescale: 1000,
            language: "und".to_string(),
            media_conf,
            track_name: None,
        },
    });

    // 2. Add nonempty audio tracks (timescale = sample_rate)
    for audio_stream_idx in audio_indices {
        let audio_stream = &snapshot.streams[audio_stream_idx];
        let audio_desc = &audio_stream.descriptor;

        let sample_rate = audio_desc.sample_rate.unwrap_or(0);
        if sample_rate == 0 {
            return Err(validation_error(format!(
                "audio stream {} is missing positive sample rate",
                audio_desc.stream_id
            )));
        }
        let freq_index = sample_freq_index(sample_rate)?;

        let channels = audio_desc.channels.unwrap_or(0);
        if channels == 0 {
            return Err(validation_error(format!(
                "audio stream {} is missing positive channel count",
                audio_desc.stream_id
            )));
        }
        let chan_conf = channel_config(channels)?;

        let profile = audio_desc
            .extradata
            .as_ref()
            .and_then(|data| {
                if data.len() >= 2 {
                    let aot_val = (data.as_slice()[0] >> 3) & 0x1F;
                    AudioObjectType::try_from(aot_val).ok()
                } else {
                    None
                }
            })
            .unwrap_or(AudioObjectType::AacLowComplexity);

        if profile != AudioObjectType::AacLowComplexity {
            return Err(validation_error(format!(
                "audio stream {} uses unsupported audio profile {:?}; only AAC-LC is supported",
                audio_desc.stream_id, profile
            )));
        }

        let aac_config = AacConfig {
            bitrate: 0,
            profile,
            freq_index,
            chan_conf,
        };

        let track_id = u32::try_from(tracks.len() + 1)
            .map_err(|_| validation_error("too many tracks for MP4 container"))?;

        tracks.push(PreparedTrack {
            stream_index: audio_stream_idx,
            stream_id: audio_desc.stream_id,
            track_id,
            media_type: MediaType::Audio,
            codec: audio_desc.codec.clone(),
            timescale: sample_rate,
            config: TrackConfig {
                track_type: TrackType::Audio,
                timescale: sample_rate,
                language: "und".to_string(),
                media_conf: MediaConfig::AacConfig(aac_config),
                track_name: audio_desc.name.clone(),
            },
        });
    }

    Ok(tracks)
}

fn sample_freq_index(sample_rate: u32) -> Result<SampleFreqIndex> {
    match sample_rate {
        96_000 => Ok(SampleFreqIndex::Freq96000),
        88_200 => Ok(SampleFreqIndex::Freq88200),
        64_000 => Ok(SampleFreqIndex::Freq64000),
        48_000 => Ok(SampleFreqIndex::Freq48000),
        44_100 => Ok(SampleFreqIndex::Freq44100),
        32_000 => Ok(SampleFreqIndex::Freq32000),
        24_000 => Ok(SampleFreqIndex::Freq24000),
        22_050 => Ok(SampleFreqIndex::Freq22050),
        16_000 => Ok(SampleFreqIndex::Freq16000),
        12_000 => Ok(SampleFreqIndex::Freq12000),
        11_025 => Ok(SampleFreqIndex::Freq11025),
        8_000 => Ok(SampleFreqIndex::Freq8000),
        7_350 => Ok(SampleFreqIndex::Freq7350),
        rate => Err(validation_error(format!(
            "unsupported audio sample rate: {} Hz",
            rate
        ))),
    }
}

fn channel_config(channels: u16) -> Result<ChannelConfig> {
    match channels {
        1 => Ok(ChannelConfig::Mono),
        2 => Ok(ChannelConfig::Stereo),
        3 => Ok(ChannelConfig::Three),
        4 => Ok(ChannelConfig::Four),
        5 => Ok(ChannelConfig::Five),
        6 => Ok(ChannelConfig::FiveOne),
        7 => Ok(ChannelConfig::SevenOne),
        c => Err(validation_error(format!(
            "unsupported audio channel count: {}",
            c
        ))),
    }
}

fn extract_sps_pps(stream: &StreamPackets) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;

    if let Some(extradata) = stream
        .descriptor
        .extradata
        .as_ref()
        .filter(|d| !d.is_empty())
    {
        let data = extradata.as_slice();
        if let Some((parsed_sps, parsed_pps)) = parse_sps_pps_from_avcc(data) {
            sps = Some(parsed_sps);
            pps = Some(parsed_pps);
        } else if let Some(nals) = nal_units(data) {
            for nal in nals {
                if nal.is_empty() {
                    continue;
                }
                let nal_type = nal[0] & 0x1F;
                if nal_type == 7 && sps.is_none() {
                    sps = Some(nal.to_vec());
                } else if nal_type == 8 && pps.is_none() {
                    pps = Some(nal.to_vec());
                }
            }
        }
    }

    if sps.is_none() || pps.is_none() {
        for packet in &stream.packets {
            if let Some(nals) = nal_units(packet.payload.as_slice()) {
                for nal in nals {
                    if nal.is_empty() {
                        continue;
                    }
                    let nal_type = nal[0] & 0x1F;
                    if nal_type == 7 && sps.is_none() {
                        sps = Some(nal.to_vec());
                    } else if nal_type == 8 && pps.is_none() {
                        pps = Some(nal.to_vec());
                    }
                }
            }
            if sps.is_some() && pps.is_some() {
                break;
            }
        }
    }

    let sps = sps.ok_or_else(|| {
        validation_error(format!(
            "video stream {} is missing H.264 SPS initialization data",
            stream.descriptor.stream_id
        ))
    })?;
    let pps = pps.ok_or_else(|| {
        validation_error(format!(
            "video stream {} is missing H.264 PPS initialization data",
            stream.descriptor.stream_id
        ))
    })?;

    Ok((sps, pps))
}

fn parse_sps_pps_from_avcc(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if data.len() < 7 || data[0] != 1 {
        return None;
    }
    let sps_count = usize::from(data[5] & 0x1F);
    if sps_count == 0 {
        return None;
    }
    let mut offset = 6_usize;
    let mut sps = None;
    for i in 0..sps_count {
        let end_of_length = offset.checked_add(2)?;
        if end_of_length > data.len() {
            return None;
        }
        let length = usize::from(u16::from_be_bytes(
            data[offset..end_of_length].try_into().ok()?,
        ));
        offset = end_of_length;
        let end_of_nal = offset.checked_add(length)?;
        if end_of_nal > data.len() {
            return None;
        }
        if i == 0 {
            sps = Some(data[offset..end_of_nal].to_vec());
        }
        offset = end_of_nal;
    }
    let sps = sps?;
    if offset >= data.len() {
        return None;
    }
    let pps_count = usize::from(data[offset]);
    if pps_count == 0 {
        return None;
    }
    offset += 1;
    let mut pps = None;
    for i in 0..pps_count {
        let end_of_length = offset.checked_add(2)?;
        if end_of_length > data.len() {
            return None;
        }
        let length = usize::from(u16::from_be_bytes(
            data[offset..end_of_length].try_into().ok()?,
        ));
        offset = end_of_length;
        let end_of_nal = offset.checked_add(length)?;
        if end_of_nal > data.len() {
            return None;
        }
        if i == 0 {
            pps = Some(data[offset..end_of_nal].to_vec());
        }
        offset = end_of_nal;
    }
    let pps = pps?;
    Some((sps, pps))
}

fn extract_hevc_config(stream: &StreamPackets, width: u16, height: u16) -> Result<HevcConfig> {
    if let Some(extradata) = stream
        .descriptor
        .extradata
        .as_ref()
        .filter(|d| !d.is_empty())
    {
        let data = extradata.as_slice();
        if is_likely_hvcc_record(data) {
            return parse_hevc_config_from_hvcc(data, width, height)
                .map_err(|e| validation_error(format!("failed parsing hvcC extradata: {e}")));
        }
    }

    let mut vps_list: Vec<Vec<u8>> = Vec::new();
    let mut sps_list: Vec<Vec<u8>> = Vec::new();
    let mut pps_list: Vec<Vec<u8>> = Vec::new();

    if let Some(extradata) = stream
        .descriptor
        .extradata
        .as_ref()
        .filter(|d| !d.is_empty())
    {
        let data = extradata.as_slice();
        if let Some(nals) = nal_units(data) {
            for nal in nals {
                collect_hevc_nal(nal, &mut vps_list, &mut sps_list, &mut pps_list);
            }
        }
    }

    if vps_list.is_empty() || sps_list.is_empty() || pps_list.is_empty() {
        for packet in &stream.packets {
            if let Some(nals) = nal_units(packet.payload.as_slice()) {
                for nal in nals {
                    collect_hevc_nal(nal, &mut vps_list, &mut sps_list, &mut pps_list);
                }
            }
            if !vps_list.is_empty() && !sps_list.is_empty() && !pps_list.is_empty() {
                break;
            }
        }
    }

    if vps_list.is_empty() {
        return Err(validation_error(format!(
            "video stream {} is missing HEVC VPS initialization data",
            stream.descriptor.stream_id
        )));
    }
    if sps_list.is_empty() {
        return Err(validation_error(format!(
            "video stream {} is missing HEVC SPS initialization data",
            stream.descriptor.stream_id
        )));
    }
    if pps_list.is_empty() {
        return Err(validation_error(format!(
            "video stream {} is missing HEVC PPS initialization data",
            stream.descriptor.stream_id
        )));
    }

    let sps_info = parse_hevc_sps(&sps_list[0])
        .map_err(|e| validation_error(format!("failed parsing HEVC SPS: {e}")))?;

    let config = HevcConfig {
        width,
        height,
        general_profile_space: sps_info.general_profile_space,
        general_tier_flag: sps_info.general_tier_flag,
        general_profile_idc: sps_info.general_profile_idc,
        general_profile_compatibility_flags: sps_info.general_profile_compatibility_flags,
        general_constraint_indicator_flags: sps_info.general_constraint_indicator_flags,
        general_level_idc: sps_info.general_level_idc,
        min_spatial_segmentation_idc: 0,
        parallelism_type: 0,
        chroma_format_idc: sps_info.chroma_format_idc,
        bit_depth_luma_minus8: sps_info.bit_depth_luma_minus8,
        bit_depth_chroma_minus8: sps_info.bit_depth_chroma_minus8,
        avg_frame_rate: 0,
        constant_frame_rate: 0,
        num_temporal_layers: sps_info.num_temporal_layers,
        temporal_id_nested: sps_info.temporal_id_nested,
        length_size_minus_one: 3,
        vps: vps_list,
        sps: sps_list,
        pps: pps_list,
    };
    config
        .validate()
        .map_err(|e| validation_error(format!("HEVC config validation failed: {e}")))?;
    Ok(config)
}

fn is_likely_hvcc_record(data: &[u8]) -> bool {
    if data.len() < 23 || data[0] != 1 {
        return false;
    }
    if data.len() >= 4 {
        let len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if len + 4 == data.len() && data.len() < 16_777_216 {
            return false;
        }
    }
    true
}

fn collect_hevc_nal(
    nal: &[u8],
    vps_list: &mut Vec<Vec<u8>>,
    sps_list: &mut Vec<Vec<u8>>,
    pps_list: &mut Vec<Vec<u8>>,
) {
    if nal.len() < 2 {
        return;
    }
    let nal_type = (nal[0] >> 1) & 0x3F;
    match nal_type {
        32 if !vps_list.iter().any(|existing| existing.as_slice() == nal) => {
            vps_list.push(nal.to_vec());
        }
        33 if !sps_list.iter().any(|existing| existing.as_slice() == nal) => {
            sps_list.push(nal.to_vec());
        }
        34 if !pps_list.iter().any(|existing| existing.as_slice() == nal) => {
            pps_list.push(nal.to_vec());
        }
        _ => {}
    }
}

fn parse_hevc_config_from_hvcc(
    data: &[u8],
    width: u16,
    height: u16,
) -> std::result::Result<HevcConfig, String> {
    if data.len() < 23 {
        return Err("hvcC record length is less than 23 bytes".to_string());
    }
    if data[0] != 1 {
        return Err(format!("unsupported hvcC configurationVersion {}", data[0]));
    }
    let general_profile_space = (data[1] >> 6) & 0x03;
    let general_tier_flag = (data[1] & 0x20) != 0;
    let general_profile_idc = data[1] & 0x1F;

    let general_profile_compatibility_flags = u32::from_be_bytes(
        data[2..6]
            .try_into()
            .map_err(|_| "failed reading compatibility flags".to_string())?,
    );

    let general_constraint_indicator_flags =
        u64::from_be_bytes([0, 0, data[6], data[7], data[8], data[9], data[10], data[11]]);

    let general_level_idc = data[12];
    let min_spatial_segmentation_raw = u16::from_be_bytes(
        data[13..15]
            .try_into()
            .map_err(|_| "failed reading min spatial segmentation".to_string())?,
    );
    let min_spatial_segmentation_idc = min_spatial_segmentation_raw & 0x0FFF;

    let parallelism_type = data[15] & 0x03;
    let chroma_format_idc = data[16] & 0x03;
    let bit_depth_luma_minus8 = data[17] & 0x07;
    let bit_depth_chroma_minus8 = data[18] & 0x07;
    let avg_frame_rate = u16::from_be_bytes(
        data[19..21]
            .try_into()
            .map_err(|_| "failed reading avg frame rate".to_string())?,
    );

    let byte21 = data[21];
    let constant_frame_rate = (byte21 >> 6) & 0x03;
    let num_temporal_layers = (byte21 >> 3) & 0x07;
    let temporal_id_nested = (byte21 & 0x04) != 0;
    let length_size_minus_one = byte21 & 0x03;

    let num_of_arrays = data[22] as usize;
    let mut offset = 23_usize;
    let mut vps = Vec::new();
    let mut sps = Vec::new();
    let mut pps = Vec::new();

    for _ in 0..num_of_arrays {
        if offset + 3 > data.len() {
            return Err("truncated array header in hvcC".to_string());
        }
        let array_header = data[offset];
        let nal_type = array_header & 0x3F;
        let num_nalus = u16::from_be_bytes(
            data[offset + 1..offset + 3]
                .try_into()
                .map_err(|_| "failed reading numNalus".to_string())?,
        ) as usize;
        offset += 3;

        for _ in 0..num_nalus {
            if offset + 2 > data.len() {
                return Err("truncated NAL unit length in hvcC".to_string());
            }
            let nal_len = u16::from_be_bytes(
                data[offset..offset + 2]
                    .try_into()
                    .map_err(|_| "failed reading NAL length".to_string())?,
            ) as usize;
            offset += 2;
            if offset + nal_len > data.len() {
                return Err("truncated NAL unit data in hvcC".to_string());
            }
            let nal_bytes = data[offset..offset + nal_len].to_vec();
            offset += nal_len;

            match nal_type {
                32 => vps.push(nal_bytes),
                33 => sps.push(nal_bytes),
                34 => pps.push(nal_bytes),
                _ => {}
            }
        }
    }

    if offset != data.len() {
        return Err("trailing unparsed bytes in hvcC record".to_string());
    }
    if vps.is_empty() {
        return Err("hvcC record is missing VPS array".to_string());
    }
    if sps.is_empty() {
        return Err("hvcC record is missing SPS array".to_string());
    }
    if pps.is_empty() {
        return Err("hvcC record is missing PPS array".to_string());
    }

    let config = HevcConfig {
        width,
        height,
        general_profile_space,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility_flags,
        general_constraint_indicator_flags,
        general_level_idc,
        min_spatial_segmentation_idc,
        parallelism_type,
        chroma_format_idc,
        bit_depth_luma_minus8,
        bit_depth_chroma_minus8,
        avg_frame_rate,
        constant_frame_rate,
        num_temporal_layers,
        temporal_id_nested,
        length_size_minus_one,
        vps,
        sps,
        pps,
    };
    config
        .validate()
        .map_err(|e| format!("hvcC config validation failed: {e}"))?;
    Ok(config)
}

struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn remaining_bits(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.bit_pos)
    }

    fn read_bits(&mut self, n: usize) -> Option<u64> {
        if n == 0 {
            return Some(0);
        }
        if n > 64 || self.remaining_bits() < n {
            return None;
        }
        let mut result = 0_u64;
        for _ in 0..n {
            let byte_idx = self.bit_pos / 8;
            let bit_idx = 7 - (self.bit_pos % 8);
            let bit = ((self.data[byte_idx] >> bit_idx) & 1) as u64;
            result = (result << 1) | bit;
            self.bit_pos += 1;
        }
        Some(result)
    }

    fn read_bit(&mut self) -> Option<bool> {
        self.read_bits(1).map(|b| b == 1)
    }

    fn read_u8(&mut self, n: usize) -> Option<u8> {
        if n > 8 {
            return None;
        }
        self.read_bits(n).map(|b| b as u8)
    }

    fn read_u16(&mut self, n: usize) -> Option<u16> {
        if n > 16 {
            return None;
        }
        self.read_bits(n).map(|b| b as u16)
    }

    fn read_u32(&mut self, n: usize) -> Option<u32> {
        if n > 32 {
            return None;
        }
        self.read_bits(n).map(|b| b as u32)
    }

    fn read_ue(&mut self) -> Option<u64> {
        let mut leading_zeros = 0_usize;
        while !self.read_bit()? {
            leading_zeros += 1;
            if leading_zeros > 32 {
                return None;
            }
        }
        if leading_zeros == 0 {
            return Some(0);
        }
        let suffix = self.read_bits(leading_zeros)?;
        let value = (1_u64 << leading_zeros)
            .checked_sub(1)?
            .checked_add(suffix)?;
        Some(value)
    }
}

fn remove_emulation_prevention(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len());
    let mut zeros = 0_usize;
    for &byte in data {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        if byte == 0 {
            zeros += 1;
        } else {
            zeros = 0;
        }
        result.push(byte);
    }
    result
}

struct HevcSpsInfo {
    general_profile_space: u8,
    general_tier_flag: bool,
    general_profile_idc: u8,
    general_profile_compatibility_flags: u32,
    general_constraint_indicator_flags: u64,
    general_level_idc: u8,
    chroma_format_idc: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    num_temporal_layers: u8,
    temporal_id_nested: bool,
}

fn parse_hevc_sps(nal: &[u8]) -> std::result::Result<HevcSpsInfo, String> {
    if nal.len() < 2 {
        return Err("SPS NAL is too short".to_string());
    }
    let nal_type = (nal[0] >> 1) & 0x3F;
    if nal_type != 33 {
        return Err(format!("expected SPS NAL type 33, got {nal_type}"));
    }

    let rbsp = remove_emulation_prevention(&nal[2..]);
    let mut reader = BitReader::new(&rbsp);

    let _sps_video_parameter_set_id = reader
        .read_u8(4)
        .ok_or_else(|| "truncated sps_video_parameter_set_id".to_string())?;
    let sps_max_sub_layers_minus1 = reader
        .read_u8(3)
        .ok_or_else(|| "truncated sps_max_sub_layers_minus1".to_string())?;
    let num_temporal_layers = sps_max_sub_layers_minus1
        .checked_add(1)
        .ok_or_else(|| "num_temporal_layers overflow".to_string())?;
    if num_temporal_layers > 7 {
        return Err(format!("invalid num_temporal_layers {num_temporal_layers}"));
    }
    let temporal_id_nested = reader
        .read_bit()
        .ok_or_else(|| "truncated sps_temporal_id_nesting_flag".to_string())?;

    let general_profile_space = reader
        .read_u8(2)
        .ok_or_else(|| "truncated general_profile_space".to_string())?;
    let general_tier_flag = reader
        .read_bit()
        .ok_or_else(|| "truncated general_tier_flag".to_string())?;
    let general_profile_idc = reader
        .read_u8(5)
        .ok_or_else(|| "truncated general_profile_idc".to_string())?;
    let general_profile_compatibility_flags = reader
        .read_u32(32)
        .ok_or_else(|| "truncated general_profile_compatibility_flags".to_string())?;
    let general_constraint_indicator_flags = reader
        .read_bits(48)
        .ok_or_else(|| "truncated general_constraint_indicator_flags".to_string())?;
    let general_level_idc = reader
        .read_u8(8)
        .ok_or_else(|| "truncated general_level_idc".to_string())?;

    let mut sub_layer_profile_present = [false; 8];
    let mut sub_layer_level_present = [false; 8];
    for i in 0..sps_max_sub_layers_minus1 as usize {
        sub_layer_profile_present[i] = reader
            .read_bit()
            .ok_or_else(|| "truncated sub_layer_profile_present_flag".to_string())?;
        sub_layer_level_present[i] = reader
            .read_bit()
            .ok_or_else(|| "truncated sub_layer_level_present_flag".to_string())?;
    }
    if sps_max_sub_layers_minus1 > 0 {
        let reserved_bits = (8 - sps_max_sub_layers_minus1 as usize) * 2;
        reader
            .read_bits(reserved_bits)
            .ok_or_else(|| "truncated reserved_zero_2bits".to_string())?;
    }
    for i in 0..sps_max_sub_layers_minus1 as usize {
        if sub_layer_profile_present[i] {
            reader
                .read_bits(88)
                .ok_or_else(|| "truncated sub-layer profile data".to_string())?;
        }
        if sub_layer_level_present[i] {
            reader
                .read_bits(8)
                .ok_or_else(|| "truncated sub-layer level data".to_string())?;
        }
    }

    let _sps_seq_parameter_set_id = reader
        .read_ue()
        .ok_or_else(|| "truncated sps_seq_parameter_set_id".to_string())?;
    let chroma_format_idc_u64 = reader
        .read_ue()
        .ok_or_else(|| "truncated chroma_format_idc".to_string())?;
    let chroma_format_idc = u8::try_from(chroma_format_idc_u64)
        .map_err(|_| "chroma_format_idc exceeds u8".to_string())?;
    if chroma_format_idc > 3 {
        return Err(format!("invalid chroma_format_idc {chroma_format_idc}"));
    }
    if chroma_format_idc == 3 {
        let _separate_colour_plane_flag = reader
            .read_bit()
            .ok_or_else(|| "truncated separate_colour_plane_flag".to_string())?;
    }
    let _pic_width_in_luma_samples = reader
        .read_ue()
        .ok_or_else(|| "truncated pic_width_in_luma_samples".to_string())?;
    let _pic_height_in_luma_samples = reader
        .read_ue()
        .ok_or_else(|| "truncated pic_height_in_luma_samples".to_string())?;
    let conformance_window_flag = reader
        .read_bit()
        .ok_or_else(|| "truncated conformance_window_flag".to_string())?;
    if conformance_window_flag {
        reader
            .read_ue()
            .ok_or_else(|| "truncated conf_win_left_offset".to_string())?;
        reader
            .read_ue()
            .ok_or_else(|| "truncated conf_win_right_offset".to_string())?;
        reader
            .read_ue()
            .ok_or_else(|| "truncated conf_win_top_offset".to_string())?;
        reader
            .read_ue()
            .ok_or_else(|| "truncated conf_win_bottom_offset".to_string())?;
    }
    let bit_depth_luma_minus8_u64 = reader
        .read_ue()
        .ok_or_else(|| "truncated bit_depth_luma_minus8".to_string())?;
    let bit_depth_luma_minus8 = u8::try_from(bit_depth_luma_minus8_u64)
        .map_err(|_| "bit_depth_luma_minus8 exceeds u8".to_string())?;
    if bit_depth_luma_minus8 > 7 {
        return Err(format!(
            "invalid bit_depth_luma_minus8 {bit_depth_luma_minus8}"
        ));
    }
    let bit_depth_chroma_minus8_u64 = reader
        .read_ue()
        .ok_or_else(|| "truncated bit_depth_chroma_minus8".to_string())?;
    let bit_depth_chroma_minus8 = u8::try_from(bit_depth_chroma_minus8_u64)
        .map_err(|_| "bit_depth_chroma_minus8 exceeds u8".to_string())?;
    if bit_depth_chroma_minus8 > 7 {
        return Err(format!(
            "invalid bit_depth_chroma_minus8 {bit_depth_chroma_minus8}"
        ));
    }

    Ok(HevcSpsInfo {
        general_profile_space,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility_flags,
        general_constraint_indicator_flags,
        general_level_idc,
        chroma_format_idc,
        bit_depth_luma_minus8,
        bit_depth_chroma_minus8,
        num_temporal_layers,
        temporal_id_nested,
    })
}

fn nal_access_unit(data: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let nals = nal_units(data)
        .ok_or_else(|| "packet is not Annex B or length-prefixed NALs".to_string())?;
    let mut output = Vec::with_capacity(data.len() + 16);
    for nal in nals {
        if nal.is_empty() {
            continue;
        }
        let length = u32::try_from(nal.len()).map_err(|_| "NAL unit is too large".to_string())?;
        output.extend_from_slice(&length.to_be_bytes());
        output.extend_from_slice(nal);
    }
    if output.is_empty() {
        return Err("packet contains no NAL units".to_string());
    }
    Ok(output)
}

fn nal_units(data: &[u8]) -> Option<Vec<&[u8]>> {
    // Validate AVCC tiling first, then fall back to Annex-B
    avcc_nals(data).or_else(|| annex_b_nals(data))
}

fn annex_b_nals(data: &[u8]) -> Option<Vec<&[u8]>> {
    let first = find_start_code(data, 0)?;
    let mut result = Vec::new();
    let mut cursor = first;
    while let Some(code_len) = start_code_len(data, cursor) {
        let nal_start = cursor + code_len;
        let next = find_start_code(data, nal_start);
        let nal_end = next.unwrap_or(data.len());
        if nal_start < nal_end {
            result.push(&data[nal_start..nal_end]);
        }
        let Some(next) = next else { break };
        cursor = next;
    }
    (!result.is_empty()).then_some(result)
}

fn avcc_nals(data: &[u8]) -> Option<Vec<&[u8]>> {
    if data.len() < 4 {
        return None;
    }
    let mut offset = 0_usize;
    let mut result = Vec::new();
    while offset < data.len() {
        let end_of_length = offset.checked_add(4)?;
        if end_of_length > data.len() {
            return None;
        }
        let length = u32::from_be_bytes(data[offset..end_of_length].try_into().ok()?) as usize;
        if length == 0 {
            return None;
        }
        let nal_start = end_of_length;
        let nal_end = nal_start.checked_add(length)?;
        if nal_end > data.len() {
            return None;
        }
        result.push(&data[nal_start..nal_end]);
        offset = nal_end;
    }
    if offset == data.len() && !result.is_empty() {
        Some(result)
    } else {
        None
    }
}

fn find_start_code(data: &[u8], from: usize) -> Option<usize> {
    (from..data.len()).find(|index| start_code_len(data, *index).is_some())
}

fn start_code_len(data: &[u8], index: usize) -> Option<usize> {
    if index + 4 <= data.len() && data[index..index + 4] == [0, 0, 0, 1] {
        Some(4)
    } else if index + 3 <= data.len() && data[index..index + 3] == [0, 0, 1] {
        Some(3)
    } else {
        None
    }
}

pub fn strip_adts_if_present(data: &[u8]) -> &[u8] {
    if data.len() < 7 {
        return data;
    }
    // Check syncword (12 bits: 0xFFF) and layer == '00' (bits 1..2 of byte 1 must be 0)
    if data[0] != 0xFF || (data[1] & 0xF6) != 0xF0 {
        return data;
    }

    let protection_absent = data[1] & 0x01;
    let header_len = if protection_absent == 1 { 7 } else { 9 };
    if data.len() < header_len {
        return data;
    }

    // 13-bit frame length across byte 3 (bits 0..1), byte 4 (bits 7..0), byte 5 (bits 7..5)
    let frame_len = ((data[3] as usize & 0x03) << 11)
        | ((data[4] as usize) << 3)
        | ((data[5] as usize & 0xE0) >> 5);

    if frame_len == data.len() && data.len() > header_len {
        &data[header_len..]
    } else {
        data
    }
}

fn encode_leb128(mut value: usize, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn normalize_av1_sample_payload(data: &[u8]) -> std::result::Result<Vec<u8>, String> {
    if data.is_empty() {
        return Err("empty AV1 sample payload".to_string());
    }

    let mut offset = 0_usize;
    let mut normalized = Vec::with_capacity(data.len() + 8);

    while offset < data.len() {
        let header_byte = data[offset];
        if (header_byte & 0x80) != 0 {
            return Err("AV1 OBU forbidden bit is set".to_string());
        }
        if (header_byte & 0x01) != 0 {
            return Err("AV1 OBU reserved bit is set".to_string());
        }

        let obu_type = (header_byte >> 3) & 0x0F;
        if !((1..=7).contains(&obu_type) || obu_type == 15) {
            return Err(format!(
                "unsupported or reserved AV1 sample OBU type {obu_type}"
            ));
        }

        let obu_extension_flag = (header_byte & 0x04) != 0;
        let obu_has_size_field = (header_byte & 0x02) != 0;

        let header_len = if obu_extension_flag { 2 } else { 1 };
        if offset + header_len > data.len() {
            return Err("truncated AV1 OBU header".to_string());
        }

        if obu_extension_flag {
            let ext_byte = data[offset + 1];
            if (ext_byte & 0x07) != 0 {
                return Err("AV1 extension header reserved bits are set".to_string());
            }
        }

        if obu_has_size_field {
            let mut leb_cursor = offset + header_len;
            let mut obu_size: u64 = 0;
            let mut leb_bytes = 0_usize;
            loop {
                if leb_cursor >= data.len() {
                    return Err("truncated AV1 LEB128 size field".to_string());
                }
                if leb_bytes >= 8 {
                    return Err("AV1 LEB128 size field exceeds 8 bytes".to_string());
                }
                let byte = data[leb_cursor];
                leb_cursor += 1;
                let val = (byte & 0x7F) as u64;
                let shift = (leb_bytes * 7) as u32;
                let shifted = val
                    .checked_shl(shift)
                    .ok_or_else(|| "AV1 LEB128 size overflow".to_string())?;
                obu_size = obu_size
                    .checked_add(shifted)
                    .ok_or_else(|| "AV1 LEB128 size overflow".to_string())?;
                leb_bytes += 1;
                if (byte & 0x80) == 0 {
                    break;
                }
            }

            if obu_size > u32::MAX as u64 {
                return Err("AV1 OBU size exceeds 32-bit limit".to_string());
            }
            let obu_size = obu_size as usize;

            let obu_total_len = (leb_cursor - offset)
                .checked_add(obu_size)
                .ok_or_else(|| "AV1 OBU length overflow".to_string())?;
            if offset + obu_total_len > data.len() {
                return Err("truncated AV1 OBU payload".to_string());
            }

            normalized.push(header_byte | 0x02);
            if obu_extension_flag {
                normalized.push(data[offset + 1]);
            }
            encode_leb128(obu_size, &mut normalized);
            normalized.extend_from_slice(&data[leb_cursor..leb_cursor + obu_size]);

            offset += obu_total_len;
        } else {
            // Unsized OBU taking remainder of data
            let payload_size = data.len() - (offset + header_len);
            if payload_size > u32::MAX as usize {
                return Err("AV1 OBU size exceeds 32-bit limit".to_string());
            }
            let new_header_byte = header_byte | 0x02;
            normalized.push(new_header_byte);
            if obu_extension_flag {
                normalized.push(data[offset + 1]);
            }
            encode_leb128(payload_size, &mut normalized);
            normalized.extend_from_slice(&data[offset + header_len..]);

            offset = data.len();
        }
    }

    if normalized.is_empty() {
        return Err("no valid OBUs in AV1 sample".to_string());
    }

    Ok(normalized)
}

fn extract_av1_config(stream: &StreamPackets, width: u16, height: u16) -> Result<Av1Config> {
    if let Some(extradata) = stream
        .descriptor
        .extradata
        .as_ref()
        .filter(|d| !d.is_empty())
    {
        let data = extradata.as_slice();
        if data.len() >= 4 && data[0] == 0x81 {
            let seq_profile = (data[1] >> 5) & 0x07;
            let seq_level_idx_0 = data[1] & 0x1F;

            let seq_tier_0 = (data[2] & 0x80) != 0;
            let high_bitdepth = (data[2] & 0x40) != 0;
            let twelve_bit = (data[2] & 0x20) != 0;
            let monochrome = (data[2] & 0x10) != 0;
            let chroma_subsampling_x = (data[2] & 0x08) != 0;
            let chroma_subsampling_y = (data[2] & 0x04) != 0;
            let chroma_sample_position = data[2] & 0x03;

            let initial_presentation_delay_present = (data[3] & 0x10) != 0;
            let initial_presentation_delay_minus_one = if initial_presentation_delay_present {
                Some(data[3] & 0x0F)
            } else {
                None
            };

            let config_obus = data[4..].to_vec();
            if config_obus.is_empty() {
                return Err(validation_error("av1C extradata is missing configOBUs"));
            }

            let config = Av1Config {
                width,
                height,
                seq_profile,
                seq_level_idx_0,
                seq_tier_0,
                high_bitdepth,
                twelve_bit,
                monochrome,
                chroma_subsampling_x,
                chroma_subsampling_y,
                chroma_sample_position,
                initial_presentation_delay_minus_one,
                config_obus: config_obus.clone(),
            };
            config
                .validate()
                .map_err(|e| validation_error(format!("av1C extradata validation failed: {e}")))?;

            let inner_info = parse_av1_sequence_header_obu(&config_obus).map_err(|e| {
                validation_error(format!("failed parsing Sequence Header OBU in av1C: {e}"))
            })?;

            if seq_profile != inner_info.seq_profile
                || seq_level_idx_0 != inner_info.seq_level_idx_0
                || seq_tier_0 != inner_info.seq_tier_0
                || high_bitdepth != inner_info.high_bitdepth
                || twelve_bit != inner_info.twelve_bit
                || monochrome != inner_info.monochrome
                || chroma_subsampling_x != inner_info.chroma_subsampling_x
                || chroma_subsampling_y != inner_info.chroma_subsampling_y
                || chroma_sample_position != inner_info.chroma_sample_position
                || initial_presentation_delay_minus_one
                    != inner_info.initial_presentation_delay_minus_one
            {
                return Err(validation_error(
                    "av1C header fields contradict Sequence Header OBU bitstream",
                ));
            }

            return Ok(config);
        }
    }

    let mut seq_header_obu: Option<Vec<u8>> = None;

    if let Some(extradata) = stream
        .descriptor
        .extradata
        .as_ref()
        .filter(|d| !d.is_empty())
    {
        seq_header_obu = extract_sequence_header_obu(extradata.as_slice())?;
    }

    if seq_header_obu.is_none() {
        for packet in &stream.packets {
            if let Some(obu) = extract_sequence_header_obu(packet.payload.as_slice())? {
                seq_header_obu = Some(obu);
                break;
            }
        }
    }

    let config_obus = seq_header_obu.ok_or_else(|| {
        validation_error(format!(
            "video stream {} is missing AV1 Sequence Header OBU",
            stream.descriptor.stream_id
        ))
    })?;

    let av1_info = parse_av1_sequence_header_obu(&config_obus)
        .map_err(|e| validation_error(format!("failed parsing AV1 Sequence Header: {e}")))?;

    let config = Av1Config {
        width,
        height,
        seq_profile: av1_info.seq_profile,
        seq_level_idx_0: av1_info.seq_level_idx_0,
        seq_tier_0: av1_info.seq_tier_0,
        high_bitdepth: av1_info.high_bitdepth,
        twelve_bit: av1_info.twelve_bit,
        monochrome: av1_info.monochrome,
        chroma_subsampling_x: av1_info.chroma_subsampling_x,
        chroma_subsampling_y: av1_info.chroma_subsampling_y,
        chroma_sample_position: av1_info.chroma_sample_position,
        initial_presentation_delay_minus_one: av1_info.initial_presentation_delay_minus_one,
        config_obus,
    };
    config
        .validate()
        .map_err(|e| validation_error(format!("AV1 config validation failed: {e}")))?;
    Ok(config)
}

fn extract_sequence_header_obu(data: &[u8]) -> Result<Option<Vec<u8>>> {
    if data.is_empty() {
        return Ok(None);
    }
    let mut offset = 0_usize;
    while offset < data.len() {
        let start_offset = offset;
        let header_byte = data[offset];
        if (header_byte & 0x80) != 0 {
            return Err(validation_error("AV1 OBU forbidden bit is set"));
        }
        if (header_byte & 0x01) != 0 {
            return Err(validation_error("AV1 OBU reserved bit is set"));
        }
        let obu_type = (header_byte >> 3) & 0x0F;
        if !((1..=7).contains(&obu_type) || obu_type == 15) {
            return Err(validation_error(format!(
                "unsupported or reserved AV1 OBU type {obu_type}"
            )));
        }
        let obu_extension_flag = (header_byte & 0x04) != 0;
        let obu_has_size_field = (header_byte & 0x02) != 0;

        let header_len = if obu_extension_flag { 2 } else { 1 };
        if offset + header_len > data.len() {
            return Err(validation_error("truncated AV1 OBU header"));
        }
        if obu_extension_flag {
            let ext_byte = data[offset + 1];
            if (ext_byte & 0x07) != 0 {
                return Err(validation_error(
                    "AV1 extension header reserved bits are set",
                ));
            }
        }

        if obu_has_size_field {
            let mut leb_cursor = offset + header_len;
            let mut obu_size: u64 = 0;
            let mut leb_bytes = 0_usize;
            loop {
                if leb_cursor >= data.len() {
                    return Err(validation_error("truncated AV1 LEB128 size field"));
                }
                if leb_bytes >= 8 {
                    return Err(validation_error("AV1 LEB128 size field exceeds 8 bytes"));
                }
                let byte = data[leb_cursor];
                leb_cursor += 1;
                let val = (byte & 0x7F) as u64;
                let shift = (leb_bytes * 7) as u32;
                let shifted = val
                    .checked_shl(shift)
                    .ok_or_else(|| validation_error("AV1 LEB128 size overflow"))?;
                obu_size = obu_size
                    .checked_add(shifted)
                    .ok_or_else(|| validation_error("AV1 LEB128 size overflow"))?;
                leb_bytes += 1;
                if (byte & 0x80) == 0 {
                    break;
                }
            }
            if obu_size > u32::MAX as u64 {
                return Err(validation_error("AV1 OBU size exceeds 32-bit limit"));
            }
            let obu_size = obu_size as usize;
            let obu_total_len = (leb_cursor - start_offset)
                .checked_add(obu_size)
                .ok_or_else(|| validation_error("AV1 OBU length overflow"))?;
            if start_offset + obu_total_len > data.len() {
                return Err(validation_error("truncated AV1 OBU payload"));
            }

            if obu_type == 1 {
                let mut canonical_obu = Vec::with_capacity(header_len + 8 + obu_size);
                canonical_obu.push(header_byte | 0x02);
                if obu_extension_flag {
                    canonical_obu.push(data[start_offset + 1]);
                }
                encode_leb128(obu_size, &mut canonical_obu);
                canonical_obu.extend_from_slice(&data[leb_cursor..leb_cursor + obu_size]);
                return Ok(Some(canonical_obu));
            }

            offset = start_offset + obu_total_len;
        } else {
            let payload_size = data.len() - (start_offset + header_len);
            if payload_size > u32::MAX as usize {
                return Err(validation_error("AV1 OBU size exceeds 32-bit limit"));
            }
            if obu_type == 1 {
                let mut full_obu = Vec::with_capacity(header_len + 8 + payload_size);
                full_obu.push(header_byte | 0x02);
                if obu_extension_flag {
                    full_obu.push(data[start_offset + 1]);
                }
                encode_leb128(payload_size, &mut full_obu);
                full_obu.extend_from_slice(&data[start_offset + header_len..]);
                return Ok(Some(full_obu));
            }
            break;
        }
    }
    Ok(None)
}

#[derive(Debug)]
struct Av1SequenceHeaderInfo {
    seq_profile: u8,
    seq_level_idx_0: u8,
    seq_tier_0: bool,
    high_bitdepth: bool,
    twelve_bit: bool,
    monochrome: bool,
    chroma_subsampling_x: bool,
    chroma_subsampling_y: bool,
    chroma_sample_position: u8,
    initial_presentation_delay_minus_one: Option<u8>,
}

fn parse_av1_sequence_header_obu(obu: &[u8]) -> std::result::Result<Av1SequenceHeaderInfo, String> {
    if obu.is_empty() {
        return Err("empty Sequence Header OBU".to_string());
    }
    let header_byte = obu[0];
    if (header_byte & 0x80) != 0 {
        return Err("AV1 OBU forbidden bit is set".to_string());
    }
    if (header_byte & 0x01) != 0 {
        return Err("AV1 OBU reserved bit is set".to_string());
    }
    let obu_type = (header_byte >> 3) & 0x0F;
    if obu_type != 1 {
        return Err(format!(
            "expected Sequence Header OBU type 1, got {obu_type}"
        ));
    }
    let obu_extension_flag = (header_byte & 0x04) != 0;
    let obu_has_size_field = (header_byte & 0x02) != 0;

    let header_len = if obu_extension_flag { 2 } else { 1 };
    if header_len > obu.len() {
        return Err("truncated Sequence Header OBU header".to_string());
    }
    if obu_extension_flag {
        let ext_byte = obu[1];
        if (ext_byte & 0x07) != 0 {
            return Err("AV1 extension header reserved bits are set".to_string());
        }
    }

    let payload = if obu_has_size_field {
        let mut leb_cursor = header_len;
        let mut obu_size: u64 = 0;
        let mut leb_bytes = 0_usize;
        loop {
            if leb_cursor >= obu.len() {
                return Err("truncated LEB128 size field in Sequence Header OBU".to_string());
            }
            if leb_bytes >= 8 {
                return Err("LEB128 size field exceeds 8 bytes in Sequence Header OBU".to_string());
            }
            let byte = obu[leb_cursor];
            leb_cursor += 1;
            let val = (byte & 0x7F) as u64;
            let shift = (leb_bytes * 7) as u32;
            let shifted = val
                .checked_shl(shift)
                .ok_or_else(|| "LEB128 size overflow in Sequence Header OBU".to_string())?;
            obu_size = obu_size
                .checked_add(shifted)
                .ok_or_else(|| "LEB128 size overflow in Sequence Header OBU".to_string())?;
            leb_bytes += 1;
            if (byte & 0x80) == 0 {
                break;
            }
        }
        if obu_size > u32::MAX as u64 {
            return Err("AV1 OBU size exceeds 32-bit limit".to_string());
        }
        let obu_size = obu_size as usize;
        if leb_cursor + obu_size > obu.len() {
            return Err("truncated Sequence Header OBU payload".to_string());
        }
        &obu[leb_cursor..leb_cursor + obu_size]
    } else {
        &obu[header_len..]
    };

    let mut reader = BitReader::new(payload);
    let seq_profile = reader
        .read_u8(3)
        .ok_or_else(|| "truncated seq_profile".to_string())?;
    if seq_profile > 2 {
        return Err(format!("unsupported seq_profile {seq_profile}"));
    }
    let still_picture = reader
        .read_bit()
        .ok_or_else(|| "truncated still_picture".to_string())?;
    let reduced_still_picture_header = reader
        .read_bit()
        .ok_or_else(|| "truncated reduced_still_picture_header".to_string())?;
    if reduced_still_picture_header && !still_picture {
        return Err("reduced_still_picture_header requires still_picture".to_string());
    }

    let mut seq_level_idx_0 = 0_u8;
    let mut seq_tier_0 = false;
    let mut initial_presentation_delay_minus_one = None;

    if reduced_still_picture_header {
        seq_level_idx_0 = reader
            .read_u8(5)
            .ok_or_else(|| "truncated seq_level_idx_0".to_string())?;
        seq_tier_0 = false;
    } else {
        let timing_info_present_flag = reader
            .read_bit()
            .ok_or_else(|| "truncated timing_info_present_flag".to_string())?;
        let mut decoder_model_info_present_flag = false;
        let mut buffer_delay_length_minus_1 = 0_usize;
        if timing_info_present_flag {
            let _num_units_in_display_tick = reader
                .read_u32(32)
                .ok_or_else(|| "truncated num_units_in_display_tick".to_string())?;
            let _time_scale = reader
                .read_u32(32)
                .ok_or_else(|| "truncated time_scale".to_string())?;
            let equal_picture_interval = reader
                .read_bit()
                .ok_or_else(|| "truncated equal_picture_interval".to_string())?;
            if equal_picture_interval {
                let _num_ticks_per_picture_minus_1 = reader
                    .read_ue()
                    .ok_or_else(|| "truncated num_ticks_per_picture_minus_1".to_string())?;
            }
            decoder_model_info_present_flag = reader
                .read_bit()
                .ok_or_else(|| "truncated decoder_model_info_present_flag".to_string())?;
            if decoder_model_info_present_flag {
                buffer_delay_length_minus_1 = reader
                    .read_u8(5)
                    .ok_or_else(|| "truncated buffer_delay_length_minus_1".to_string())?
                    as usize;
                let _num_units_in_decoding_tick = reader
                    .read_u32(32)
                    .ok_or_else(|| "truncated num_units_in_decoding_tick".to_string())?;
                let _buffer_removal_time_length_minus_1 = reader
                    .read_u8(5)
                    .ok_or_else(|| "truncated buffer_removal_time_length_minus_1".to_string())?;
                let _frame_presentation_time_length_minus_1 =
                    reader.read_u8(5).ok_or_else(|| {
                        "truncated frame_presentation_time_length_minus_1".to_string()
                    })?;
            }
        }

        let initial_display_delay_present_flag = reader
            .read_bit()
            .ok_or_else(|| "truncated initial_display_delay_present_flag".to_string())?;
        let operating_points_cnt_minus_1 = reader
            .read_u8(5)
            .ok_or_else(|| "truncated operating_points_cnt_minus_1".to_string())?;

        for i in 0..=operating_points_cnt_minus_1 as usize {
            let _operating_point_idc = reader
                .read_u16(12)
                .ok_or_else(|| "truncated operating_point_idc".to_string())?;
            let seq_level_idx = reader
                .read_u8(5)
                .ok_or_else(|| "truncated seq_level_idx".to_string())?;
            let seq_tier = if seq_level_idx > 7 {
                reader
                    .read_bit()
                    .ok_or_else(|| "truncated seq_tier".to_string())?
            } else {
                false
            };

            if decoder_model_info_present_flag {
                let decoder_model_present = reader
                    .read_bit()
                    .ok_or_else(|| "truncated decoder_model_present_for_this_op".to_string())?;
                if decoder_model_present {
                    let n = buffer_delay_length_minus_1 + 1;
                    reader
                        .read_bits(n)
                        .ok_or_else(|| "truncated decoder_buffer_delay".to_string())?;
                    reader
                        .read_bits(n)
                        .ok_or_else(|| "truncated encoder_buffer_delay".to_string())?;
                    reader
                        .read_bit()
                        .ok_or_else(|| "truncated low_delay_mode_flag".to_string())?;
                }
            }

            let mut initial_display_delay = None;
            if initial_display_delay_present_flag {
                let initial_display_delay_present = reader.read_bit().ok_or_else(|| {
                    "truncated initial_display_delay_present_for_this_op".to_string()
                })?;
                if initial_display_delay_present {
                    initial_display_delay =
                        Some(reader.read_u8(4).ok_or_else(|| {
                            "truncated initial_display_delay_minus_1".to_string()
                        })?);
                }
            }

            if i == 0 {
                seq_level_idx_0 = seq_level_idx;
                seq_tier_0 = seq_tier;
                initial_presentation_delay_minus_one = initial_display_delay;
            }
        }
    }

    let frame_width_bits_minus_1 = reader
        .read_u8(4)
        .ok_or_else(|| "truncated frame_width_bits_minus_1".to_string())?
        as usize;
    let frame_height_bits_minus_1 = reader
        .read_u8(4)
        .ok_or_else(|| "truncated frame_height_bits_minus_1".to_string())?
        as usize;
    let _max_frame_width_minus_1 = reader
        .read_bits(frame_width_bits_minus_1 + 1)
        .ok_or_else(|| "truncated max_frame_width_minus_1".to_string())?;
    let _max_frame_height_minus_1 = reader
        .read_bits(frame_height_bits_minus_1 + 1)
        .ok_or_else(|| "truncated max_frame_height_minus_1".to_string())?;

    if !reduced_still_picture_header {
        let frame_id_numbers_present_flag = reader
            .read_bit()
            .ok_or_else(|| "truncated frame_id_numbers_present_flag".to_string())?;
        if frame_id_numbers_present_flag {
            let _delta_frame_id_length_minus_2 = reader
                .read_u8(4)
                .ok_or_else(|| "truncated delta_frame_id_length_minus_2".to_string())?;
            let _additional_frame_id_length_minus_1 = reader
                .read_u8(3)
                .ok_or_else(|| "truncated additional_frame_id_length_minus_1".to_string())?;
        }
    }

    let _use_128x128_superblock = reader
        .read_bit()
        .ok_or_else(|| "truncated use_128x128_superblock".to_string())?;
    let _enable_filter_intra = reader
        .read_bit()
        .ok_or_else(|| "truncated enable_filter_intra".to_string())?;
    let _enable_intra_edge_filter = reader
        .read_bit()
        .ok_or_else(|| "truncated enable_intra_edge_filter".to_string())?;

    if !reduced_still_picture_header {
        let _enable_interintra_compound = reader
            .read_bit()
            .ok_or_else(|| "truncated enable_interintra_compound".to_string())?;
        let _enable_masked_compound = reader
            .read_bit()
            .ok_or_else(|| "truncated enable_masked_compound".to_string())?;
        let _enable_warped_motion = reader
            .read_bit()
            .ok_or_else(|| "truncated enable_warped_motion".to_string())?;
        let _enable_dual_filter = reader
            .read_bit()
            .ok_or_else(|| "truncated enable_dual_filter".to_string())?;
        let enable_order_hint = reader
            .read_bit()
            .ok_or_else(|| "truncated enable_order_hint".to_string())?;
        if enable_order_hint {
            let _enable_jnt_comp = reader
                .read_bit()
                .ok_or_else(|| "truncated enable_jnt_comp".to_string())?;
            let _enable_ref_frame_mvs = reader
                .read_bit()
                .ok_or_else(|| "truncated enable_ref_frame_mvs".to_string())?;
        }
        let seq_choose_screen_content_tools = reader
            .read_bit()
            .ok_or_else(|| "truncated seq_choose_screen_content_tools".to_string())?;
        let mut seq_force_screen_content_tools = 2;
        if !seq_choose_screen_content_tools {
            seq_force_screen_content_tools = if reader
                .read_bit()
                .ok_or_else(|| "truncated seq_force_screen_content_tools".to_string())?
            {
                1
            } else {
                0
            };
        }
        if seq_force_screen_content_tools > 0 {
            let seq_choose_integer_mv = reader
                .read_bit()
                .ok_or_else(|| "truncated seq_choose_integer_mv".to_string())?;
            if !seq_choose_integer_mv {
                let _seq_force_integer_mv = reader
                    .read_bit()
                    .ok_or_else(|| "truncated seq_force_integer_mv".to_string())?;
            }
        }
        if enable_order_hint {
            let _order_hint_bits_minus_1 = reader
                .read_u8(3)
                .ok_or_else(|| "truncated order_hint_bits_minus_1".to_string())?;
        }
    }

    let _enable_superres = reader
        .read_bit()
        .ok_or_else(|| "truncated enable_superres".to_string())?;
    let _enable_cdef = reader
        .read_bit()
        .ok_or_else(|| "truncated enable_cdef".to_string())?;
    let _enable_restoration = reader
        .read_bit()
        .ok_or_else(|| "truncated enable_restoration".to_string())?;

    // color_config
    let high_bitdepth = reader
        .read_bit()
        .ok_or_else(|| "truncated high_bitdepth".to_string())?;
    let twelve_bit = if seq_profile == 2 && high_bitdepth {
        reader
            .read_bit()
            .ok_or_else(|| "truncated twelve_bit".to_string())?
    } else {
        false
    };
    let bit_depth = if seq_profile == 2 && high_bitdepth {
        if twelve_bit {
            12
        } else {
            10
        }
    } else if high_bitdepth {
        10
    } else {
        8
    };

    let monochrome = if seq_profile == 1 {
        false
    } else {
        reader
            .read_bit()
            .ok_or_else(|| "truncated monochrome".to_string())?
    };

    let color_description_present_flag = reader
        .read_bit()
        .ok_or_else(|| "truncated color_description_present_flag".to_string())?;
    let (color_primaries, transfer_characteristics, matrix_coefficients) =
        if color_description_present_flag {
            (
                reader
                    .read_u8(8)
                    .ok_or_else(|| "truncated color_primaries".to_string())?,
                reader
                    .read_u8(8)
                    .ok_or_else(|| "truncated transfer_characteristics".to_string())?,
                reader
                    .read_u8(8)
                    .ok_or_else(|| "truncated matrix_coefficients".to_string())?,
            )
        } else {
            (2, 2, 2)
        };

    let (subsampling_x, subsampling_y, chroma_sample_position) = if monochrome {
        let _color_range = reader
            .read_bit()
            .ok_or_else(|| "truncated color_range".to_string())?;
        (true, true, 0)
    } else if color_primaries == 1 && transfer_characteristics == 13 && matrix_coefficients == 0 {
        let _color_range = reader
            .read_bit()
            .ok_or_else(|| "truncated color_range".to_string())?;
        if !(seq_profile == 1 || (seq_profile == 2 && bit_depth == 12)) {
            return Err(
                "identity matrix not allowed for profile 0 or 10-bit profile 2".to_string(),
            );
        }
        (false, false, 0)
    } else {
        let _color_range = reader
            .read_bit()
            .ok_or_else(|| "truncated color_range".to_string())?;
        let (sub_x, sub_y) = if seq_profile == 0 {
            (true, true)
        } else if seq_profile == 1 {
            (false, false)
        } else if bit_depth == 12 {
            let sub_x = reader
                .read_bit()
                .ok_or_else(|| "truncated subsampling_x".to_string())?;
            let sub_y = if sub_x {
                reader
                    .read_bit()
                    .ok_or_else(|| "truncated subsampling_y".to_string())?
            } else {
                false
            };
            (sub_x, sub_y)
        } else {
            (true, false)
        };
        let csp = if sub_x && sub_y {
            reader
                .read_u8(2)
                .ok_or_else(|| "truncated chroma_sample_position".to_string())?
        } else {
            0
        };
        (sub_x, sub_y, csp)
    };

    if !monochrome {
        let _separate_uv_delta_q = reader
            .read_bit()
            .ok_or_else(|| "truncated separate_uv_delta_q".to_string())?;
    }

    let _film_grain_params_present = reader
        .read_bit()
        .ok_or_else(|| "truncated film_grain_params_present".to_string())?;

    let trailing_one_bit = reader
        .read_bit()
        .ok_or_else(|| "missing trailing_one_bit in Sequence Header OBU".to_string())?;
    if !trailing_one_bit {
        return Err("invalid trailing_one_bit in Sequence Header OBU".to_string());
    }

    while reader.remaining_bits() > 0 {
        if reader.read_bit() != Some(false) {
            return Err("invalid trailing zero bit or padding in Sequence Header OBU".to_string());
        }
    }

    Ok(Av1SequenceHeaderInfo {
        seq_profile,
        seq_level_idx_0,
        seq_tier_0,
        high_bitdepth,
        twelve_bit,
        monochrome,
        chroma_subsampling_x: subsampling_x,
        chroma_subsampling_y: subsampling_y,
        chroma_sample_position,
        initial_presentation_delay_minus_one,
    })
}

fn sorted_mp4_packets(
    snapshot: &MediaSnapshot,
    tracks: &[PreparedTrack],
) -> Result<Vec<PreparedSample>> {
    let mut all_samples = Vec::new();

    for track in tracks {
        let stream = &snapshot.streams[track.stream_index];
        if track.media_type == MediaType::Video {
            let video_codec = VideoCodec::from_str(&track.codec).ok_or_else(|| {
                validation_error(format!("unsupported video codec '{}'", track.codec))
            })?;
            for (packet_idx, packet) in stream.packets.iter().enumerate() {
                let start_time = u64::try_from(packet.dts).map_err(|_| {
                    validation_error(format!("invalid negative DTS {}", packet.dts))
                })?;
                if packet.duration <= 0 {
                    return Err(validation_error(format!(
                        "packet duration must be positive, got {}",
                        packet.duration
                    )));
                }

                // In MP4 (ISO/IEC 14496-12 stts box), sample_delta defines how long
                // the frame is displayed before being replaced by the next frame.
                // For screen/game capture with variable frame rate (VFR), calculate
                // duration from elapsed time until the next packet's DTS.
                // Fall back to packet.duration for the final frame or if DTS does not advance.
                let raw_duration = if let Some(next_packet) = stream.packets.get(packet_idx + 1) {
                    let diff = next_packet.dts - packet.dts;
                    if diff > 0 {
                        diff
                    } else {
                        packet.duration
                    }
                } else {
                    packet.duration
                };

                let duration = u32::try_from(raw_duration).map_err(|_| {
                    validation_error(format!("packet duration {} exceeds u32", raw_duration))
                })?;

                let comp_offset = packet.pts.checked_sub(packet.dts).ok_or_else(|| {
                    validation_error(format!(
                        "composition offset calculation overflowed: {} - {}",
                        packet.pts, packet.dts
                    ))
                })?;
                let rendering_offset = i32::try_from(comp_offset).map_err(|_| {
                    validation_error(format!(
                        "composition offset {} does not fit into i32",
                        comp_offset
                    ))
                })?;

                let payload = match video_codec {
                    VideoCodec::Avc | VideoCodec::Hevc => {
                        nal_access_unit(packet.payload.as_slice()).map_err(validation_error)?
                    }
                    VideoCodec::Av1 => normalize_av1_sample_payload(packet.payload.as_slice())
                        .map_err(validation_error)?,
                };
                if payload.is_empty() {
                    return Err(validation_error(format!(
                        "video stream {} contains an empty packet payload",
                        track.stream_id
                    )));
                }

                let sample = Mp4Sample {
                    start_time,
                    duration,
                    rendering_offset,
                    is_sync: packet.is_keyframe,
                    bytes: bytes::Bytes::from(payload),
                };

                all_samples.push(PreparedSample {
                    track_id: track.track_id,
                    dts_ms: packet.dts,
                    sequence: packet.sequence,
                    stream_index: track.stream_index,
                    sample,
                });
            }
        } else {
            // Audio track: timescale = sample_rate, cumulative clock with 1024 samples per frame
            let sample_rate = track.timescale;
            let first_dts_ms = stream.packets.first().map(|p| p.dts).unwrap_or(0);
            let initial_ticks_u128 = (first_dts_ms.max(0) as u128)
                .checked_mul(sample_rate as u128)
                .ok_or_else(|| validation_error("audio start timestamp multiplication overflow"))?
                / 1000;
            let initial_ticks = u64::try_from(initial_ticks_u128)
                .map_err(|_| validation_error("initial audio ticks overflowed u64"))?;

            let mut expected_ticks = initial_ticks;
            // A threshold of ~40ms distinguishes normal packet pacing from real pauses in voice/chat streams
            let pause_threshold_ticks = (sample_rate as u64 * 40) / 1000;

            for packet in &stream.packets {
                let packet_dts_ms = packet.dts.max(0);
                let packet_ticks = u64::try_from(
                    (packet_dts_ms as u128)
                        .checked_mul(sample_rate as u128)
                        .ok_or_else(|| {
                            validation_error("audio timestamp multiplication overflow")
                        })?
                        / 1000,
                )
                .map_err(|_| validation_error("audio timestamp overflowed u64"))?;

                let start_time =
                    if packet_ticks > expected_ticks.saturating_add(pause_threshold_ticks) {
                        // Preserve real silence gaps between sentences in voice chat (Discord / Mic)
                        packet_ticks
                    } else {
                        expected_ticks
                    };
                expected_ticks = start_time.saturating_add(u64::from(AAC_LC_SAMPLES_PER_FRAME));

                let raw = strip_adts_if_present(packet.payload.as_slice());
                if raw.is_empty() {
                    return Err(validation_error(format!(
                        "audio stream {} contains an empty packet payload",
                        track.stream_id
                    )));
                }

                let sample = Mp4Sample {
                    start_time,
                    duration: AAC_LC_SAMPLES_PER_FRAME,
                    rendering_offset: 0,
                    is_sync: true,
                    bytes: bytes::Bytes::copy_from_slice(raw),
                };

                all_samples.push(PreparedSample {
                    track_id: track.track_id,
                    dts_ms: packet.dts,
                    sequence: packet.sequence,
                    stream_index: track.stream_index,
                    sample,
                });
            }
        }
    }

    all_samples.sort_by(|a, b| {
        a.dts_ms
            .cmp(&b.dts_ms)
            .then_with(|| a.sequence.cmp(&b.sequence))
            .then_with(|| a.stream_index.cmp(&b.stream_index))
    });

    Ok(all_samples)
}

fn write_staged_mp4(path: &Path, snapshot: &MediaSnapshot, tracks: &[PreparedTrack]) -> Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| io_error(path, source))?;
    let buf_writer = BufWriter::new(file);

    let video_track = tracks.iter().find(|t| t.media_type == MediaType::Video);
    let video_codec_brand = video_track
        .and_then(|t| VideoCodec::from_str(&t.codec))
        .map(|c| match c {
            VideoCodec::Avc => FourCC::from(*b"avc1"),
            VideoCodec::Hevc => FourCC::from(*b"hvc1"),
            VideoCodec::Av1 => FourCC::from(*b"av01"),
        })
        .unwrap_or_else(|| FourCC::from(*b"avc1"));

    let mp4_config = Mp4Config {
        major_brand: FourCC::from(*b"isom"),
        minor_version: 512,
        compatible_brands: vec![
            FourCC::from(*b"isom"),
            FourCC::from(*b"iso2"),
            video_codec_brand,
            FourCC::from(*b"mp41"),
        ],
        timescale: 1000,
    };

    let mut writer = Mp4Writer::write_start(buf_writer, &mp4_config)
        .map_err(|e| write_error(format!("failed starting MP4 writer: {e}")))?;

    for track in tracks {
        writer
            .add_track(&track.config)
            .map_err(|e| write_error(format!("failed adding MP4 track: {e}")))?;
    }

    let sorted_samples = sorted_mp4_packets(snapshot, tracks)?;
    for item in sorted_samples {
        writer
            .write_sample(item.track_id, &item.sample)
            .map_err(|e| {
                write_error(format!(
                    "failed writing MP4 sample to track {}: {e}",
                    item.track_id
                ))
            })?;
    }

    writer
        .write_end()
        .map_err(|e| write_error(format!("failed finalizing MP4 moov box: {e}")))?;

    let mut buf_writer = writer.into_writer();
    buf_writer
        .flush()
        .map_err(|source| io_error(path, source))?;

    let file = buf_writer
        .into_inner()
        .map_err(|e| io_error(path, e.into_error()))?;

    file.sync_all().map_err(|source| io_error(path, source))?;

    drop(file);
    Ok(())
}

fn scan_top_level_boxes<R: Read + Seek>(reader: &mut R, file_size: u64) -> Result<Vec<[u8; 4]>> {
    let mut boxes = Vec::new();
    let mut offset = 0_u64;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|e| validation_error(format!("seek error: {e}")))?;

    while offset < file_size {
        if offset + 8 > file_size {
            return Err(validation_error(format!(
                "truncated box header at offset {offset}"
            )));
        }
        let mut header = [0_u8; 8];
        reader
            .read_exact(&mut header)
            .map_err(|e| validation_error(format!("read box header error: {e}")))?;

        let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
        let box_type = [header[4], header[5], header[6], header[7]];
        boxes.push(box_type);

        let actual_size = if size32 == 1 {
            if offset + 16 > file_size {
                return Err(validation_error(format!(
                    "truncated 64-bit box header at offset {offset}"
                )));
            }
            let mut size64_bytes = [0_u8; 8];
            reader
                .read_exact(&mut size64_bytes)
                .map_err(|e| validation_error(format!("read 64-bit size error: {e}")))?;
            u64::from_be_bytes(size64_bytes)
        } else if size32 == 0 {
            file_size - offset
        } else {
            size32
        };

        if actual_size < 8 {
            return Err(validation_error(format!(
                "invalid box size {actual_size} for box {:?}",
                std::str::from_utf8(&box_type).unwrap_or("????")
            )));
        }

        offset = offset
            .checked_add(actual_size)
            .ok_or_else(|| validation_error("box size overflow"))?;

        if offset < file_size {
            reader
                .seek(SeekFrom::Start(offset))
                .map_err(|e| validation_error(format!("seek to next box error: {e}")))?;
        }
    }
    Ok(boxes)
}

fn validate_staged_mp4_file(path: &Path, prepared_tracks: &[PreparedTrack]) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| io_error(path, source))?;
    file.sync_all().map_err(|source| io_error(path, source))?;

    let file_size = file
        .metadata()
        .map(|m| m.len())
        .map_err(|source| io_error(path, source))?;

    if file_size < 32 {
        return Err(validation_error("staged MP4 output is too small"));
    }

    // 1. Validate top-level boxes
    let boxes = scan_top_level_boxes(&mut file, file_size)?;
    if !boxes.iter().any(|b| b == b"ftyp") {
        return Err(validation_error("staged MP4 is missing ftyp box"));
    }
    if !boxes.iter().any(|b| b == b"mdat") {
        return Err(validation_error("staged MP4 is missing mdat box"));
    }
    if !boxes.iter().any(|b| b == b"moov") {
        return Err(validation_error("staged MP4 is missing moov box"));
    }

    // 2. Validate via Mp4Reader header parsing
    file.seek(SeekFrom::Start(0))
        .map_err(|source| io_error(path, source))?;
    let reader = Mp4Reader::read_header(&mut file, file_size)
        .map_err(|e| validation_error(format!("Mp4Reader failed to parse authored MP4: {e}")))?;

    if reader.tracks().len() != prepared_tracks.len() {
        return Err(validation_error(format!(
            "MP4 reader reported {} tracks, expected {}",
            reader.tracks().len(),
            prepared_tracks.len()
        )));
    }

    for prepared_track in prepared_tracks {
        let track = reader
            .tracks()
            .get(&prepared_track.track_id)
            .ok_or_else(|| {
                validation_error(format!(
                    "authored MP4 is missing expected track {}",
                    prepared_track.track_id
                ))
            })?;

        let actual_track_type = track.track_type().map_err(|e| {
            validation_error(format!(
                "track {} track_type check failed: {e}",
                prepared_track.track_id
            ))
        })?;
        if actual_track_type != prepared_track.config.track_type {
            return Err(validation_error(format!(
                "track {} track_type mismatch: expected {:?}, got {:?}",
                prepared_track.track_id, prepared_track.config.track_type, actual_track_type
            )));
        }

        if prepared_track.media_type == MediaType::Video {
            let video_codec = VideoCodec::from_str(&prepared_track.codec).ok_or_else(|| {
                validation_error(format!(
                    "unsupported video codec '{}'",
                    prepared_track.codec
                ))
            })?;
            match video_codec {
                VideoCodec::Avc => {
                    let media_type = track.media_type().map_err(|e| {
                        validation_error(format!("video track media_type check failed: {e}"))
                    })?;
                    if media_type != mp4::MediaType::H264 {
                        return Err(validation_error(format!(
                            "video track media_type mismatch: expected H264, got {media_type:?}"
                        )));
                    }
                    let box_type = track.box_type().map_err(|e| {
                        validation_error(format!("video track box_type check failed: {e}"))
                    })?;
                    if box_type != FourCC::from(mp4::BoxType::Avc1Box) {
                        return Err(validation_error(format!(
                            "video track box_type mismatch: expected avc1, got {box_type}"
                        )));
                    }
                }
                VideoCodec::Hevc => {
                    let media_type = track.media_type().map_err(|e| {
                        validation_error(format!("video track media_type check failed: {e}"))
                    })?;
                    if media_type != mp4::MediaType::H265 {
                        return Err(validation_error(format!(
                            "video track media_type mismatch: expected H265, got {media_type:?}"
                        )));
                    }
                    let box_type = track.box_type().map_err(|e| {
                        validation_error(format!("video track box_type check failed: {e}"))
                    })?;
                    if box_type != FourCC::from(mp4::BoxType::Hvc1Box) {
                        return Err(validation_error(format!(
                            "video track box_type mismatch: expected hvc1, got {box_type}"
                        )));
                    }
                }
                VideoCodec::Av1 => {
                    let media_type = track.media_type().map_err(|e| {
                        validation_error(format!("video track media_type check failed: {e}"))
                    })?;
                    if media_type != mp4::MediaType::AV1 {
                        return Err(validation_error(format!(
                            "video track media_type mismatch: expected AV1, got {media_type:?}"
                        )));
                    }
                    let box_type = track.box_type().map_err(|e| {
                        validation_error(format!("video track box_type check failed: {e}"))
                    })?;
                    if box_type != FourCC::from(mp4::BoxType::Av01Box) {
                        return Err(validation_error(format!(
                            "video track box_type mismatch: expected av01, got {box_type}"
                        )));
                    }
                }
            }
        } else if prepared_track.media_type == MediaType::Audio {
            let media_type = track.media_type().map_err(|e| {
                validation_error(format!("audio track media_type check failed: {e}"))
            })?;
            if media_type != mp4::MediaType::AAC {
                return Err(validation_error(format!(
                    "audio track media_type mismatch: expected AAC, got {media_type:?}"
                )));
            }
            let box_type = track
                .box_type()
                .map_err(|e| validation_error(format!("audio track box_type check failed: {e}")))?;
            if box_type != FourCC::from(mp4::BoxType::Mp4aBox) {
                return Err(validation_error(format!(
                    "audio track box_type mismatch: expected mp4a, got {box_type}"
                )));
            }
        }
    }

    Ok(())
}

fn snapshot_duration_ms(snapshot: &MediaSnapshot) -> u64 {
    let mut max_duration = 0_u64;
    for stream in &snapshot.streams {
        if stream.packets.is_empty() {
            continue;
        }
        match stream.descriptor.media_type {
            MediaType::Video => {
                let stream_dur = stream
                    .packets
                    .iter()
                    .map(|p| p.pts.saturating_add(p.duration))
                    .max()
                    .unwrap_or(0);
                max_duration = max_duration.max(u64::try_from(stream_dur.max(0)).unwrap_or(0));
            }
            MediaType::Audio => {
                let sample_rate = stream.descriptor.sample_rate.unwrap_or(48_000) as u64;
                let numerator = (stream.packets.len() as u64)
                    .saturating_mul(u64::from(AAC_LC_SAMPLES_PER_FRAME))
                    .saturating_mul(1000);
                if let Some(audio_dur_ms) = numerator.checked_div(sample_rate) {
                    let first_dts =
                        stream.packets.first().map(|p| p.dts).unwrap_or(0).max(0) as u64;
                    max_duration = max_duration.max(first_dts.saturating_add(audio_dur_ms));
                }
            }
        }
    }
    max_duration
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_types::{EncodedPacket, PacketPayload, StreamDescriptor, TimeBase};
    use std::fs::File;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    const SPS: &[u8] = &[0x67, 0x42, 0x00, 0x1E, 0xE9, 0x01, 0x40, 0x7B, 0x20];
    const PPS: &[u8] = &[0x68, 0xCE, 0x3C, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00, 0x10, 0x20, 0x30];
    const P_FRAME: &[u8] = &[0x41, 0x9A, 0x22, 0x11, 0x55];

    fn annex_b(parts: &[&[u8]]) -> Vec<u8> {
        let mut output = Vec::new();
        for part in parts {
            output.extend_from_slice(&[0, 0, 0, 1]);
            output.extend_from_slice(part);
        }
        output
    }

    fn avcc_payload(parts: &[&[u8]]) -> Vec<u8> {
        let mut output = Vec::new();
        for part in parts {
            let len = u32::try_from(part.len()).unwrap();
            output.extend_from_slice(&len.to_be_bytes());
            output.extend_from_slice(part);
        }
        output
    }

    fn avcc_extradata(sps: &[u8], pps: &[u8]) -> Vec<u8> {
        let mut output = vec![
            1, sps[1], sps[2], sps[3], 0xFF, 0xE1, // 1 SPS
        ];
        let sps_len = u16::try_from(sps.len()).unwrap();
        output.extend_from_slice(&sps_len.to_be_bytes());
        output.extend_from_slice(sps);

        output.push(1); // 1 PPS
        let pps_len = u16::try_from(pps.len()).unwrap();
        output.extend_from_slice(&pps_len.to_be_bytes());
        output.extend_from_slice(pps);

        output
    }

    fn video_descriptor(extradata: Option<Vec<u8>>) -> StreamDescriptor {
        StreamDescriptor {
            stream_id: StreamId(0),
            media_type: MediaType::Video,
            time_base: TimeBase::MILLISECOND,
            name: None,
            codec: "h264".to_string(),
            extradata: extradata.map(PacketPayload::from),
            width: Some(640),
            height: Some(480),
            sample_rate: None,
            channels: None,
            pixel_format: None,
        }
    }

    fn audio_descriptor(
        stream_id: StreamId,
        name: Option<&str>,
        sample_rate: u32,
        channels: u16,
        extradata: Option<Vec<u8>>,
    ) -> StreamDescriptor {
        StreamDescriptor {
            stream_id,
            media_type: MediaType::Audio,
            time_base: TimeBase::MILLISECOND,
            name: name.map(|s| s.to_string()),
            codec: "aac".to_string(),
            extradata: extradata.map(PacketPayload::from),
            width: None,
            height: None,
            sample_rate: Some(sample_rate),
            channels: Some(channels),
            pixel_format: None,
        }
    }

    fn video_packet(
        pts: i64,
        dts: i64,
        duration: i64,
        keyframe: bool,
        payload: Vec<u8>,
    ) -> Arc<EncodedPacket> {
        Arc::new(EncodedPacket {
            stream_id: StreamId(0),
            media_type: MediaType::Video,
            pts,
            dts,
            duration,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: keyframe,
            sequence: pts as u64,
            payload: PacketPayload::from(payload),
        })
    }

    fn audio_packet(
        stream_id: StreamId,
        pts: i64,
        dts: i64,
        duration: i64,
        payload: Vec<u8>,
    ) -> Arc<EncodedPacket> {
        Arc::new(EncodedPacket {
            stream_id,
            media_type: MediaType::Audio,
            pts,
            dts,
            duration,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: true,
            sequence: pts as u64,
            payload: PacketPayload::from(payload),
        })
    }

    fn temp_output_path() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "silk-mp4-test-{}-{}.mp4",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn writes_h264_only_mp4() {
        let path = temp_output_path();
        let snapshot = MediaSnapshot {
            origin_pts: 50,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(None),
                packets: vec![
                    video_packet(50, 50, 33, true, annex_b(&[SPS, PPS, IDR])),
                    video_packet(83, 83, 33, false, annex_b(&[P_FRAME])),
                    video_packet(116, 116, 33, false, annex_b(&[P_FRAME])),
                ],
            }],
            captured_at_unix_ms: 123456789,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux H.264 MP4");

        assert_eq!(metadata.path, path);
        assert_eq!(metadata.video_codec, Some("h264".to_string()));
        assert!(metadata.audio_codecs.is_empty());
        assert_eq!(metadata.created_at_unix_ms, 123456789);
        assert_eq!(metadata.duration_ms, 99); // (116 - 50) + 33 = 99
        assert!(metadata.size_bytes > 0);
        assert!(path.exists());
        assert!(!staged_path(&path).exists());

        // Validate using Mp4Reader
        let mut file = File::open(&path).expect("open published mp4");
        let mut reader =
            Mp4Reader::read_header(&mut file, metadata.size_bytes).expect("read header");
        assert_eq!(reader.tracks().len(), 1);
        let track = reader.tracks().get(&1).expect("track 1");
        assert_eq!(track.track_type().unwrap(), TrackType::Video);
        assert_eq!(track.sample_count(), 3);

        let sample1 = reader.read_sample(1, 1).unwrap().unwrap();
        assert_eq!(sample1.start_time, 0);
        assert_eq!(sample1.duration, 33);
        assert_eq!(sample1.rendering_offset, 0);
        assert!(sample1.is_sync);

        let sample2 = reader.read_sample(1, 2).unwrap().unwrap();
        assert_eq!(sample2.start_time, 33);
        assert_eq!(sample2.duration, 33);
        assert!(!sample2.is_sync);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn writes_h264_plus_two_aac_tracks_mp4() {
        let path = temp_output_path();
        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![
                StreamPackets {
                    descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                    packets: vec![
                        video_packet(0, 0, 33, true, avcc_payload(&[IDR])),
                        video_packet(33, 33, 33, false, avcc_payload(&[P_FRAME])),
                    ],
                },
                StreamPackets {
                    descriptor: audio_descriptor(
                        StreamId(1),
                        Some("Game Audio"),
                        48_000,
                        2,
                        Some(vec![0x11, 0x90]),
                    ),
                    packets: vec![
                        audio_packet(StreamId(1), 0, 0, 21, vec![0x21, 0x10, 0x01]),
                        audio_packet(StreamId(1), 21, 21, 21, vec![0x21, 0x10, 0x02]),
                    ],
                },
                StreamPackets {
                    descriptor: audio_descriptor(
                        StreamId(2),
                        Some("Microphone"),
                        44_100,
                        1,
                        Some(vec![0x12, 0x08]),
                    ),
                    packets: vec![
                        audio_packet(StreamId(2), 0, 0, 23, vec![0x22, 0x10, 0x01]),
                        audio_packet(StreamId(2), 23, 23, 23, vec![0x22, 0x10, 0x02]),
                    ],
                },
            ],
            captured_at_unix_ms: 555,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux multi-track MP4");

        assert_eq!(metadata.video_codec, Some("h264".to_string()));
        assert_eq!(metadata.audio_codecs, vec!["aac", "aac"]);

        // Validate using Mp4Reader
        let mut file = File::open(&path).expect("open published mp4");
        let mut reader =
            Mp4Reader::read_header(&mut file, metadata.size_bytes).expect("read header");
        assert_eq!(reader.tracks().len(), 3);

        let video_track = reader.tracks().get(&1).expect("video track");
        assert_eq!(video_track.track_type().unwrap(), TrackType::Video);
        assert_eq!(video_track.timescale(), 1000);
        assert_eq!(video_track.sample_count(), 2);

        let audio1_track = reader.tracks().get(&2).expect("audio track 1");
        assert_eq!(audio1_track.track_type().unwrap(), TrackType::Audio);
        assert_eq!(audio1_track.track_name(), "Game Audio");
        assert_eq!(audio1_track.timescale(), 48_000);
        assert_eq!(audio1_track.sample_count(), 2);

        let audio1_s1 = reader.read_sample(2, 1).unwrap().unwrap();
        assert_eq!(audio1_s1.start_time, 0);
        assert_eq!(audio1_s1.duration, 1024);

        let audio1_s2 = reader.read_sample(2, 2).unwrap().unwrap();
        assert_eq!(audio1_s2.start_time, 1024);
        assert_eq!(audio1_s2.duration, 1024);

        let audio2_track = reader.tracks().get(&3).expect("audio track 2");
        assert_eq!(audio2_track.track_type().unwrap(), TrackType::Audio);
        assert_eq!(audio2_track.track_name(), "Microphone");
        assert_eq!(audio2_track.timescale(), 44_100);
        assert_eq!(audio2_track.sample_count(), 2);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn eliminates_aac_clock_drift_over_long_run() {
        let path = temp_output_path();
        // 2813 packets at 48 kHz (representing ~60.01 seconds of audio: 2813 * 1024 / 48000 = 60.010666 s)
        let packet_count = 2813_usize;
        let mut audio_packets = Vec::with_capacity(packet_count);
        for i in 0..packet_count {
            let dts_ms = (i as i64 * 1024 * 1000) / 48000;
            audio_packets.push(audio_packet(
                StreamId(1),
                dts_ms,
                dts_ms,
                21,
                vec![0x21, 0x10, (i % 256) as u8],
            ));
        }

        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![
                StreamPackets {
                    descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                    packets: vec![
                        video_packet(0, 0, 30_000, true, avcc_payload(&[IDR])),
                        video_packet(30_000, 30_000, 30_000, false, avcc_payload(&[P_FRAME])),
                    ],
                },
                StreamPackets {
                    descriptor: audio_descriptor(
                        StreamId(1),
                        Some("Game Audio"),
                        48_000,
                        2,
                        Some(vec![0x11, 0x90]),
                    ),
                    packets: audio_packets,
                },
            ],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux long AAC stream");

        let mut file = File::open(&path).expect("open file");
        let mut reader =
            Mp4Reader::read_header(&mut file, metadata.size_bytes).expect("read header");
        let audio_track = reader.tracks().get(&2).expect("audio track");
        assert_eq!(audio_track.timescale(), 48_000);
        assert_eq!(audio_track.sample_count(), packet_count as u32);

        // Check sample 2813 (1-indexed)
        let last_sample = reader.read_sample(2, packet_count as u32).unwrap().unwrap();
        let expected_start_ticks = (packet_count as u64 - 1) * 1024;
        assert_eq!(last_sample.start_time, expected_start_ticks);
        assert_eq!(last_sample.duration, 1024);

        let total_audio_ticks = (packet_count as u64) * 1024;
        let total_audio_ms = (total_audio_ticks * 1000) / 48000;
        assert_eq!(total_audio_ms, 60010);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn vfr_sample_durations_prevent_fast_forward_speed() {
        let path = temp_output_path();
        // 4 frames arriving at 60 Hz capture cadence (16-17ms apart) despite 8ms nominal duration.
        // Without VFR delta calculation, total duration would be 4 * 8ms = 32ms (2x fast forward).
        // With VFR delta calculation, total duration is 16 + 17 + 17 + 8 = 58ms (accurate real time).
        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                packets: vec![
                    video_packet(0, 0, 8, true, avcc_payload(&[IDR])),
                    video_packet(16, 16, 8, false, avcc_payload(&[P_FRAME])),
                    video_packet(33, 33, 8, false, avcc_payload(&[P_FRAME])),
                    video_packet(50, 50, 8, false, avcc_payload(&[P_FRAME])),
                ],
            }],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux VFR video stream");

        let mut file = File::open(&path).expect("open file");
        let mut reader =
            Mp4Reader::read_header(&mut file, metadata.size_bytes).expect("read header");
        assert_eq!(
            reader.tracks().get(&1).expect("video track").sample_count(),
            4
        );
        let total_duration: u64 = reader
            .tracks()
            .get(&1)
            .expect("video track")
            .duration()
            .as_millis() as u64;

        let s1 = reader.read_sample(1, 1).unwrap().unwrap();
        assert_eq!(s1.duration, 16);

        let s2 = reader.read_sample(1, 2).unwrap().unwrap();
        assert_eq!(s2.duration, 17);

        let s3 = reader.read_sample(1, 3).unwrap().unwrap();
        assert_eq!(s3.duration, 17);

        let s4 = reader.read_sample(1, 4).unwrap().unwrap();
        assert_eq!(s4.duration, 8);

        assert_eq!(total_duration, 58);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn annex_b_preserves_valid_terminal_zero_bytes() {
        let path = temp_output_path();
        // IDR NAL unit with valid terminal zero bytes: 0x65, 0x88, 0x84, 0x00, 0x00
        let idr_with_zeroes = &[0x65, 0x88, 0x84, 0x12, 0x00, 0x00];
        let p_with_zeroes = &[0x41, 0x9A, 0x22, 0x00];

        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(None),
                packets: vec![
                    video_packet(0, 0, 33, true, annex_b(&[SPS, PPS, idr_with_zeroes])),
                    video_packet(33, 33, 33, false, annex_b(&[p_with_zeroes])),
                ],
            }],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux Annex-B with terminal zeroes");

        let mut file = File::open(&path).expect("open file");
        let mut reader =
            Mp4Reader::read_header(&mut file, metadata.size_bytes).expect("read header");
        let sample1 = reader.read_sample(1, 1).unwrap().unwrap();
        // Sample 1 payload should contain: [4-byte len(SPS), SPS, 4-byte len(PPS), PPS, 4-byte len(IDR), IDR]
        let expected_idr_len = idr_with_zeroes.len() as u32;
        let idr_offset = 4 + SPS.len() + 4 + PPS.len();
        assert_eq!(
            &sample1.bytes[idr_offset..idr_offset + 4],
            &expected_idr_len.to_be_bytes()
        );
        assert_eq!(
            &sample1.bytes[idr_offset + 4..idr_offset + 4 + idr_with_zeroes.len()],
            idr_with_zeroes
        );

        let sample2 = reader.read_sample(1, 2).unwrap().unwrap();
        let expected_p_len = p_with_zeroes.len() as u32;
        assert_eq!(&sample2.bytes[..4], &expected_p_len.to_be_bytes());
        assert_eq!(&sample2.bytes[4..4 + p_with_zeroes.len()], p_with_zeroes);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn avcc_tiling_first_does_not_split_on_embedded_start_codes() {
        let path = temp_output_path();
        // AVCC NAL unit whose payload contains byte patterns [0x00, 0x00, 0x01] and [0x00, 0x00, 0x00, 0x01]
        let tricky_nal = &[
            0x65, 0x88, 0x00, 0x00, 0x01, 0x55, 0x00, 0x00, 0x00, 0x01, 0x99,
        ];

        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                packets: vec![video_packet(0, 0, 33, true, avcc_payload(&[tricky_nal]))],
            }],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux AVCC with embedded start codes");

        let mut file = File::open(&path).expect("open file");
        let mut reader =
            Mp4Reader::read_header(&mut file, metadata.size_bytes).expect("read header");
        let sample = reader.read_sample(1, 1).unwrap().unwrap();
        // Must contain exactly 1 NAL of length tricky_nal.len()
        let expected_len = tricky_nal.len() as u32;
        assert_eq!(&sample.bytes[..4], &expected_len.to_be_bytes());
        assert_eq!(&sample.bytes[4..4 + tricky_nal.len()], tricky_nal);
        assert_eq!(sample.bytes.len(), 4 + tricky_nal.len());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn robust_adts_stripping_and_false_positive_retention() {
        let raw_payload = &[0x21, 0x10, 0x55, 0xaa];

        // 1. Valid 7-byte ADTS header for raw_payload (total length = 11 bytes)
        // syncword = 0xFFF, ID=0, layer=00, protection_absent=1 -> byte 0: 0xFF, byte 1: 0xF1
        // profile=1 (AAC-LC), freq_idx=3 (48000), private=0, chan=2 -> byte 2: 0x58 (01 0011 0 0)
        // chan_rest=0, orig=0, home=0, cpy=0, cpy_st=0, len_hi=00 -> byte 3: 0x80 (0 0 0 0 0 0 00)
        // len_mid=00000001 (1) -> byte 4: 0x01
        // len_lo=011 (3), fullness=0x7FF -> byte 5: 0x7F (011 11111) -> frame_len = 11
        // fullness_rest=0x3F, raw_blocks=0 -> byte 6: 0xFC
        let mut valid_adts = vec![0xFF, 0xF1, 0x58, 0x80, 0x01, 0x7F, 0xFC];
        valid_adts.extend_from_slice(raw_payload);
        assert_eq!(strip_adts_if_present(&valid_adts), raw_payload);

        // 2. False-positive: starts with 0xFF, 0xF1, but layer is 01 (not 00)
        let mut bad_layer = valid_adts.clone();
        bad_layer[1] = 0xF3; // layer bits 1..2 are 01
        assert_eq!(strip_adts_if_present(&bad_layer), &bad_layer[..]);

        // 3. False-positive: frame length in header (e.g. 20) does not match payload length (11)
        let mut bad_len = valid_adts.clone();
        bad_len[4] = 0x02; // changes frame length
        assert_eq!(strip_adts_if_present(&bad_len), &bad_len[..]);

        // 4. Raw AAC payload that does not start with 0xFF
        assert_eq!(strip_adts_if_present(raw_payload), raw_payload);
    }

    #[test]
    fn rejects_more_than_one_video_stream() {
        let path = temp_output_path();
        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![
                StreamPackets {
                    descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                    packets: vec![video_packet(0, 0, 33, true, avcc_payload(&[IDR]))],
                },
                StreamPackets {
                    descriptor: StreamDescriptor {
                        stream_id: StreamId(1),
                        media_type: MediaType::Video,
                        time_base: TimeBase::MILLISECOND,
                        name: Some("Secondary video".to_string()),
                        codec: "h264".to_string(),
                        extradata: Some(PacketPayload::from(avcc_extradata(SPS, PPS))),
                        width: Some(320),
                        height: Some(240),
                        sample_rate: None,
                        channels: None,
                        pixel_format: None,
                    },
                    packets: vec![Arc::new(EncodedPacket {
                        stream_id: StreamId(1),
                        media_type: MediaType::Video,
                        pts: 0,
                        dts: 0,
                        duration: 33,
                        time_base: TimeBase::MILLISECOND,
                        is_keyframe: true,
                        sequence: 0,
                        payload: PacketPayload::from(avcc_payload(&[IDR])),
                    })],
                },
            ],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        let err = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect_err("must reject multiple video streams");

        assert!(matches!(err, MuxerError::ValidationFailed { .. }));
        assert!(!path.exists());
        assert!(!staged_path(&path).exists());
    }

    #[test]
    fn rejects_total_payload_exceeding_safe_32bit_limit() {
        let path = temp_output_path();
        // Create 4097 packets referencing a 1 MB payload buffer using refcounts without allocating >4GB
        let one_mb_payload = PacketPayload::from(vec![0u8; 1024 * 1024]);
        let mut packets = Vec::with_capacity(4097);
        for i in 0..4097_i64 {
            packets.push(Arc::new(EncodedPacket {
                stream_id: StreamId(0),
                media_type: MediaType::Video,
                pts: i * 33,
                dts: i * 33,
                duration: 33,
                time_base: TimeBase::MILLISECOND,
                is_keyframe: i == 0,
                sequence: i as u64,
                payload: one_mb_payload.clone(),
            }));
        }

        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                packets,
            }],
            captured_at_unix_ms: 0,
        };

        let total_size = check_total_payload_size(&snapshot);
        assert!(total_size.is_err());

        let mut muxer = Mp4Muxer;
        let err = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect_err("must reject total payload exceeding safe 32-bit limit");

        assert!(matches!(err, MuxerError::ValidationFailed { .. }));
        assert!(!path.exists());
        assert!(!staged_path(&path).exists());
    }

    #[test]
    fn supports_avcc_and_annex_b_inputs() {
        // Test Annex-B conversion
        let path1 = temp_output_path();
        let annex_b_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(None),
                packets: vec![video_packet(0, 0, 40, true, annex_b(&[SPS, PPS, IDR]))],
            }],
            captured_at_unix_ms: 100,
        };
        let mut muxer = Mp4Muxer;
        let meta1 = muxer
            .write_snapshot(&annex_b_snap, &path1, &SaveOptions::default())
            .expect("Annex-B mux");
        assert!(meta1.size_bytes > 0);
        let _ = fs::remove_file(path1);

        // Test AVCC conversion
        let path2 = temp_output_path();
        let avcc_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                packets: vec![video_packet(0, 0, 40, true, avcc_payload(&[IDR]))],
            }],
            captured_at_unix_ms: 100,
        };
        let meta2 = muxer
            .write_snapshot(&avcc_snap, &path2, &SaveOptions::default())
            .expect("AVCC mux");
        assert!(meta2.size_bytes > 0);
        let _ = fs::remove_file(path2);
    }

    #[test]
    fn preserves_timestamp_and_rendering_offsets_with_b_frames() {
        let path = temp_output_path();
        // Simulation of I, P, B frame stream:
        // Frame 0 (I-frame): DTS=0, PTS=66, composition offset=66
        // Frame 1 (P-frame): DTS=33, PTS=132, composition offset=99
        // Frame 2 (B-frame): DTS=66, PTS=33, composition offset=-33
        // Frame 3 (B-frame): DTS=99, PTS=99, composition offset=0
        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                packets: vec![
                    video_packet(66, 0, 33, true, avcc_payload(&[IDR])),
                    video_packet(132, 33, 33, false, avcc_payload(&[P_FRAME])),
                    video_packet(33, 66, 33, false, avcc_payload(&[P_FRAME])),
                    video_packet(99, 99, 33, false, avcc_payload(&[P_FRAME])),
                ],
            }],
            captured_at_unix_ms: 1,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("write B-frame MP4");

        let mut file = File::open(&path).expect("open published mp4");
        let mut reader =
            Mp4Reader::read_header(&mut file, metadata.size_bytes).expect("read header");

        let s1 = reader.read_sample(1, 1).unwrap().unwrap();
        assert_eq!(s1.start_time, 0);
        assert_eq!(s1.rendering_offset, 66);
        assert!(s1.is_sync);

        let s2 = reader.read_sample(1, 2).unwrap().unwrap();
        assert_eq!(s2.start_time, 33);
        assert_eq!(s2.rendering_offset, 99);
        assert!(!s2.is_sync);

        let s3 = reader.read_sample(1, 3).unwrap().unwrap();
        assert_eq!(s3.start_time, 66);
        assert_eq!(s3.rendering_offset, -33);
        assert!(!s3.is_sync);

        let s4 = reader.read_sample(1, 4).unwrap().unwrap();
        assert_eq!(s4.start_time, 99);
        assert_eq!(s4.rendering_offset, 0);
        assert!(!s4.is_sync);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_unsupported_video_and_audio_codecs() {
        let path = temp_output_path();
        let mut muxer = Mp4Muxer;

        // HEVC rejection
        let mut hevc_desc = video_descriptor(None);
        hevc_desc.codec = "hevc".to_string();
        let hevc_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: hevc_desc,
                packets: vec![video_packet(0, 0, 33, true, vec![1, 2, 3])],
            }],
            captured_at_unix_ms: 0,
        };
        let err = muxer
            .write_snapshot(&hevc_snap, &path, &SaveOptions::default())
            .expect_err("must reject HEVC");
        assert!(matches!(err, MuxerError::ValidationFailed { .. }));
        assert!(!path.exists());
        assert!(!staged_path(&path).exists());

        // AV1 rejection
        let mut av1_desc = video_descriptor(None);
        av1_desc.codec = "av1".to_string();
        let av1_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_desc,
                packets: vec![video_packet(0, 0, 33, true, vec![1, 2, 3])],
            }],
            captured_at_unix_ms: 0,
        };
        let err = muxer
            .write_snapshot(&av1_snap, &path, &SaveOptions::default())
            .expect_err("must reject AV1");
        assert!(matches!(err, MuxerError::ValidationFailed { .. }));

        // Opus audio rejection
        let mut opus_desc =
            audio_descriptor(StreamId(1), Some("Audio"), 48000, 2, Some(vec![1, 2]));
        opus_desc.codec = "opus".to_string();
        let opus_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![
                StreamPackets {
                    descriptor: video_descriptor(None),
                    packets: vec![video_packet(0, 0, 33, true, annex_b(&[SPS, PPS, IDR]))],
                },
                StreamPackets {
                    descriptor: opus_desc,
                    packets: vec![audio_packet(StreamId(1), 0, 0, 20, vec![1, 2, 3])],
                },
            ],
            captured_at_unix_ms: 0,
        };
        let err = muxer
            .write_snapshot(&opus_snap, &path, &SaveOptions::default())
            .expect_err("must reject Opus audio");
        assert!(matches!(err, MuxerError::ValidationFailed { .. }));
        assert!(!path.exists());
        assert!(!staged_path(&path).exists());
    }

    #[test]
    fn rejects_malformed_and_missing_sps_pps() {
        let path = temp_output_path();
        let mut muxer = Mp4Muxer;

        // 1. Missing SPS and PPS completely
        let missing_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(None),
                packets: vec![video_packet(0, 0, 33, true, annex_b(&[IDR]))],
            }],
            captured_at_unix_ms: 0,
        };
        let err = muxer
            .write_snapshot(&missing_snap, &path, &SaveOptions::default())
            .expect_err("missing SPS/PPS");
        assert!(matches!(err, MuxerError::ValidationFailed { .. }));
        assert!(!path.exists());
        assert!(!staged_path(&path).exists());

        // 2. SPS too short (< 4 bytes)
        let short_sps = &[0x67, 0x42, 0x00];
        let short_sps_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(None),
                packets: vec![video_packet(
                    0,
                    0,
                    33,
                    true,
                    annex_b(&[short_sps, PPS, IDR]),
                )],
            }],
            captured_at_unix_ms: 0,
        };
        let err = muxer
            .write_snapshot(&short_sps_snap, &path, &SaveOptions::default())
            .expect_err("SPS too short");
        assert!(matches!(err, MuxerError::ValidationFailed { .. }));

        // 3. PPS missing (SPS present only)
        let missing_pps_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(None),
                packets: vec![video_packet(0, 0, 33, true, annex_b(&[SPS, IDR]))],
            }],
            captured_at_unix_ms: 0,
        };
        let err = muxer
            .write_snapshot(&missing_pps_snap, &path, &SaveOptions::default())
            .expect_err("PPS missing");
        assert!(matches!(err, MuxerError::ValidationFailed { .. }));
    }

    #[test]
    fn extracts_sps_pps_from_length_prefixed_extradata() {
        let path = temp_output_path();
        let mut muxer = Mp4Muxer;

        // Extradata formatted as length-prefixed NALs: [len(4), SPS, len(4), PPS]
        let mut lp_extradata = Vec::new();
        let sps_len = u32::try_from(SPS.len()).unwrap();
        lp_extradata.extend_from_slice(&sps_len.to_be_bytes());
        lp_extradata.extend_from_slice(SPS);
        let pps_len = u32::try_from(PPS.len()).unwrap();
        lp_extradata.extend_from_slice(&pps_len.to_be_bytes());
        lp_extradata.extend_from_slice(PPS);

        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(Some(lp_extradata)),
                packets: vec![video_packet(0, 0, 33, true, avcc_payload(&[IDR]))],
            }],
            captured_at_unix_ms: 10,
        };

        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux with length-prefixed extradata");
        assert!(metadata.size_bytes > 0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cleans_up_part_file_on_write_failure() {
        let path = temp_output_path();
        let mut muxer = Mp4Muxer;

        // Packet 2 has invalid NAL payload
        let corrupt_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                packets: vec![
                    video_packet(0, 0, 33, true, avcc_payload(&[IDR])),
                    video_packet(33, 33, 33, false, vec![0xDE, 0xAD, 0xBE, 0xEF]),
                ],
            }],
            captured_at_unix_ms: 0,
        };

        let err = muxer
            .write_snapshot(&corrupt_snap, &path, &SaveOptions::default())
            .expect_err("corrupt packet must fail");
        assert!(matches!(err, MuxerError::ValidationFailed { .. }));
        assert!(!path.exists());
        assert!(!staged_path(&path).exists());
    }

    #[test]
    fn validates_top_level_box_structure() {
        let path = temp_output_path();
        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                packets: vec![video_packet(0, 0, 33, true, avcc_payload(&[IDR]))],
            }],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux MP4");

        let mut file = File::open(&path).expect("open published mp4");
        let boxes = scan_top_level_boxes(&mut file, metadata.size_bytes).expect("scan boxes");

        assert_eq!(boxes[0], *b"ftyp");
        assert!(boxes.iter().any(|b| b == b"mdat"));
        assert!(boxes.iter().any(|b| b == b"moov"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_negative_or_underflowing_timestamps() {
        let path = temp_output_path();
        let mut muxer = Mp4Muxer;

        // Origin PTS is 100, but a packet has DTS = 50. Post-normalization DTS would be -50.
        let invalid_snap = MediaSnapshot {
            origin_pts: 100,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: video_descriptor(Some(avcc_extradata(SPS, PPS))),
                packets: vec![
                    video_packet(100, 100, 33, true, avcc_payload(&[IDR])),
                    video_packet(133, 50, 33, false, avcc_payload(&[P_FRAME])),
                ],
            }],
            captured_at_unix_ms: 0,
        };

        let err = normalize_snapshot(&invalid_snap).expect_err("must reject non-monotonic DTS");
        assert!(matches!(err, MuxerError::ValidationFailed { .. }));

        let err2 = muxer.write_snapshot(&invalid_snap, &path, &SaveOptions::default());
        assert!(err2.is_err());
        assert!(!path.exists());
        assert!(!staged_path(&path).exists());
    }

    #[test]
    fn snapshot_duration_handles_zero_sample_rate_and_empty_streams() {
        let mut audio_desc = audio_descriptor(StreamId(1), Some("Audio"), 48000, 2, None);
        audio_desc.sample_rate = Some(0);

        let snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![
                StreamPackets {
                    descriptor: video_descriptor(None),
                    packets: Vec::new(),
                },
                StreamPackets {
                    descriptor: audio_desc,
                    packets: vec![audio_packet(StreamId(1), 0, 0, 21, vec![1, 2, 3])],
                },
            ],
            captured_at_unix_ms: 0,
        };

        // Zero sample rate should safely produce 0 without dividing by zero
        assert_eq!(snapshot_duration_ms(&snap), 0);
    }

    const HEVC_VPS: &[u8] = &[
        0x40, 0x01, 0x0c, 0x01, 0xff, 0xff, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x80, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x03, 0x00, 0x78, 0xac, 0x09,
    ];
    const HEVC_SPS: &[u8] = &[
        0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x80, 0x00, 0x00, 0x03, 0x00, 0x00,
        0x03, 0x00, 0x78, 0xa0, 0x05, 0x02, 0x01, 0x48, 0x59, 0x5a, 0x6e, 0x4f, 0x01, 0x0e, 0x01,
        0x11, 0x72, 0x74, 0x08, 0x00, 0x00, 0x03, 0x00, 0x08, 0x00, 0x00, 0x03, 0x01, 0xe0, 0x40,
    ];
    const HEVC_PPS: &[u8] = &[0x44, 0x01, 0xc1, 0x72, 0xb4, 0x62, 0x40];
    const HEVC_IDR: &[u8] = &[0x26, 0x01, 0xaf, 0x01, 0x11, 0x22, 0x33];
    const HEVC_TRAIL: &[u8] = &[0x02, 0x01, 0x11, 0x22, 0x33];

    fn hvcc_extradata(vps: &[u8], sps: &[u8], pps: &[u8]) -> Vec<u8> {
        let mut output = vec![
            1, // configurationVersion
            1, // profile space(0), tier(0), profile_idc(1)
            0x60, 0x00, 0x00, 0x00, // compatibility flags
            0x90, 0x00, 0x00, 0x00, 0x00, 0x00, // constraint flags
            120,  // level idc
            0xF0, 0x00, // min spatial segmentation idc
            0xFC, // parallelism type
            0xFD, // chroma format 1 (4:2:0)
            0xF8, // bit depth luma minus 8 (0)
            0xF8, // bit depth chroma minus 8 (0)
            0x00, 0x3C, // avg frame rate (60)
            0x4F, // constant_frame_rate(1) | num_temporal_layers(1) | temporal_id_nested(1) | length_size_minus_one(3)
            3,    // numOfArrays
        ];
        // Array 0: VPS (type 32 = 0x20, completeness = 1 -> 0xA0)
        output.push(0xA0);
        output.extend_from_slice(&1_u16.to_be_bytes());
        output.extend_from_slice(&u16::try_from(vps.len()).unwrap().to_be_bytes());
        output.extend_from_slice(vps);

        // Array 1: SPS (type 33 = 0x21, completeness = 1 -> 0xA1)
        output.push(0xA1);
        output.extend_from_slice(&1_u16.to_be_bytes());
        output.extend_from_slice(&u16::try_from(sps.len()).unwrap().to_be_bytes());
        output.extend_from_slice(sps);

        // Array 2: PPS (type 34 = 0x22, completeness = 1 -> 0xA2)
        output.push(0xA2);
        output.extend_from_slice(&1_u16.to_be_bytes());
        output.extend_from_slice(&u16::try_from(pps.len()).unwrap().to_be_bytes());
        output.extend_from_slice(pps);

        output
    }

    fn hevc_video_descriptor(codec: &str, extradata: Option<Vec<u8>>) -> StreamDescriptor {
        StreamDescriptor {
            stream_id: StreamId(0),
            media_type: MediaType::Video,
            time_base: TimeBase::MILLISECOND,
            name: None,
            codec: codec.to_string(),
            extradata: extradata.map(PacketPayload::from),
            width: Some(1920),
            height: Some(1080),
            sample_rate: None,
            channels: None,
            pixel_format: None,
        }
    }

    #[test]
    fn hevc_with_complete_hvcc_extradata_writes_and_reads_hvc1() {
        let path = temp_output_path();
        let hvcc = hvcc_extradata(HEVC_VPS, HEVC_SPS, HEVC_PPS);
        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![
                StreamPackets {
                    descriptor: hevc_video_descriptor("hevc", Some(hvcc)),
                    packets: vec![
                        video_packet(0, 0, 33, true, avcc_payload(&[HEVC_IDR])),
                        video_packet(33, 33, 33, false, avcc_payload(&[HEVC_TRAIL])),
                    ],
                },
                StreamPackets {
                    descriptor: audio_descriptor(StreamId(1), Some("Audio"), 48000, 2, None),
                    packets: vec![
                        audio_packet(StreamId(1), 0, 0, 21, vec![0x11, 0x22, 0x33]),
                        audio_packet(StreamId(1), 21, 21, 21, vec![0x44, 0x55, 0x66]),
                    ],
                },
            ],
            captured_at_unix_ms: 1_700_000_000,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux HEVC MP4");

        assert_eq!(metadata.video_codec.as_deref(), Some("hevc"));
        assert_eq!(metadata.audio_codecs, vec!["aac".to_string()]);

        let file = File::open(&path).expect("open published file");
        let file_size = file.metadata().unwrap().len();
        let mut reader = mp4::Mp4Reader::read_header(file, file_size).expect("read mp4 header");

        assert_eq!(reader.tracks().len(), 2);
        let video_track = reader.tracks().get(&1).expect("video track 1");
        assert_eq!(video_track.track_type().unwrap(), TrackType::Video);
        assert_eq!(video_track.media_type().unwrap(), mp4::MediaType::H265);
        assert_eq!(
            video_track.box_type().unwrap(),
            FourCC::from(mp4::BoxType::Hvc1Box)
        );
        assert_eq!(video_track.box_type().unwrap().value, *b"hvc1");
        assert_eq!(video_track.width(), 1920);
        assert_eq!(video_track.height(), 1080);

        let hvcc_box = video_track.hvcc().expect("hvcc present on hvc1 track");
        assert_eq!(hvcc_box.configuration_version, 1);
        assert_eq!(hvcc_box.general_profile_idc, 1);
        assert_eq!(hvcc_box.general_level_idc, 120);
        assert_eq!(hvcc_box.length_size_minus_one, 3);
        assert_eq!(hvcc_box.arrays.len(), 3);
        assert_eq!(hvcc_box.arrays[0].nal_units[0], HEVC_VPS);
        assert_eq!(hvcc_box.arrays[1].nal_units[0], HEVC_SPS);
        assert_eq!(hvcc_box.arrays[2].nal_units[0], HEVC_PPS);

        // Check ftyp compatible brands contains hvc1
        assert!(reader
            .ftyp
            .compatible_brands
            .iter()
            .any(|b| b.value == *b"hvc1"));

        // Verify mdat samples are 4-byte length prefixed
        let sample = reader.read_sample(1, 1).unwrap().unwrap();
        assert!(sample.bytes.len() >= 4);
        let nal_len = u32::from_be_bytes(sample.bytes[0..4].try_into().unwrap()) as usize;
        assert_eq!(nal_len, HEVC_IDR.len());
        assert_eq!(&sample.bytes[4..4 + nal_len], HEVC_IDR);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn hevc_annex_b_parameter_sets_packets_follow_sps_parsing() {
        let path = temp_output_path();
        // Packets contain Annex-B VPS, SPS, PPS along with IDR
        let keyframe_payload = annex_b(&[HEVC_VPS, HEVC_SPS, HEVC_PPS, HEVC_IDR]);
        let p_payload = annex_b(&[HEVC_TRAIL]);

        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: hevc_video_descriptor("h265", None),
                packets: vec![
                    video_packet(0, 0, 33, true, keyframe_payload),
                    video_packet(33, 33, 33, false, p_payload),
                ],
            }],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux HEVC Annex-B MP4");

        assert_eq!(metadata.video_codec.as_deref(), Some("h265"));

        let file = File::open(&path).expect("open published file");
        let file_size = file.metadata().unwrap().len();
        let reader = mp4::Mp4Reader::read_header(file, file_size).expect("read mp4 header");

        let video_track = reader.tracks().get(&1).expect("video track 1");
        assert_eq!(video_track.media_type().unwrap(), mp4::MediaType::H265);
        assert_eq!(
            video_track.box_type().unwrap(),
            FourCC::from(mp4::BoxType::Hvc1Box)
        );
        assert_eq!(video_track.box_type().unwrap().value, *b"hvc1");

        let hvcc_box = video_track.hvcc().expect("hvcc present");
        assert_eq!(hvcc_box.general_profile_idc, 1);
        assert_eq!(hvcc_box.general_level_idc, 120);
        assert_eq!(hvcc_box.chroma_format_idc, 1);
        assert_eq!(hvcc_box.bit_depth_luma_minus8, 0);
        assert_eq!(hvcc_box.bit_depth_chroma_minus8, 2);
        assert_eq!(hvcc_box.length_size_minus_one, 3);
        assert_eq!(hvcc_box.arrays.len(), 3);
        assert_eq!(hvcc_box.arrays[0].nal_units[0], HEVC_VPS);
        assert_eq!(hvcc_box.arrays[1].nal_units[0], HEVC_SPS);
        assert_eq!(hvcc_box.arrays[2].nal_units[0], HEVC_PPS);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn hevc_codec_aliases_accepted() {
        for alias in &["hevc", "h265", "hvc1", "HEVC", "H265", "HVC1"] {
            let path = temp_output_path();
            let hvcc = hvcc_extradata(HEVC_VPS, HEVC_SPS, HEVC_PPS);
            let snapshot = MediaSnapshot {
                origin_pts: 0,
                time_base: TimeBase::MILLISECOND,
                streams: vec![StreamPackets {
                    descriptor: hevc_video_descriptor(alias, Some(hvcc)),
                    packets: vec![video_packet(0, 0, 33, true, avcc_payload(&[HEVC_IDR]))],
                }],
                captured_at_unix_ms: 0,
            };

            let mut muxer = Mp4Muxer;
            let metadata = muxer
                .write_snapshot(&snapshot, &path, &SaveOptions::default())
                .expect("mux HEVC alias");
            assert_eq!(metadata.video_codec.as_deref(), Some(*alias));

            let file = File::open(&path).expect("open published file");
            let file_size = file.metadata().unwrap().len();
            let reader = mp4::Mp4Reader::read_header(file, file_size).expect("read mp4 header");
            let video_track = reader.tracks().get(&1).unwrap();
            assert_eq!(video_track.media_type().unwrap(), mp4::MediaType::H265);
            assert_eq!(video_track.box_type().unwrap().value, *b"hvc1");

            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn hevc_malformed_hvcc_and_missing_parameter_sets_fail_without_final_file() {
        let mut muxer = Mp4Muxer;

        // 1. Missing VPS
        let path1 = temp_output_path();
        let no_vps_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: hevc_video_descriptor("hevc", None),
                packets: vec![video_packet(
                    0,
                    0,
                    33,
                    true,
                    annex_b(&[HEVC_SPS, HEVC_PPS, HEVC_IDR]),
                )],
            }],
            captured_at_unix_ms: 0,
        };
        let err1 = muxer
            .write_snapshot(&no_vps_snap, &path1, &SaveOptions::default())
            .expect_err("missing VPS must fail");
        assert!(matches!(err1, MuxerError::ValidationFailed { .. }));
        assert!(!path1.exists());
        assert!(!staged_path(&path1).exists());

        // 2. Missing SPS
        let path2 = temp_output_path();
        let no_sps_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: hevc_video_descriptor("hevc", None),
                packets: vec![video_packet(
                    0,
                    0,
                    33,
                    true,
                    annex_b(&[HEVC_VPS, HEVC_PPS, HEVC_IDR]),
                )],
            }],
            captured_at_unix_ms: 0,
        };
        let err2 = muxer
            .write_snapshot(&no_sps_snap, &path2, &SaveOptions::default())
            .expect_err("missing SPS must fail");
        assert!(matches!(err2, MuxerError::ValidationFailed { .. }));
        assert!(!path2.exists());
        assert!(!staged_path(&path2).exists());

        // 3. Missing PPS
        let path3 = temp_output_path();
        let no_pps_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: hevc_video_descriptor("hevc", None),
                packets: vec![video_packet(
                    0,
                    0,
                    33,
                    true,
                    annex_b(&[HEVC_VPS, HEVC_SPS, HEVC_IDR]),
                )],
            }],
            captured_at_unix_ms: 0,
        };
        let err3 = muxer
            .write_snapshot(&no_pps_snap, &path3, &SaveOptions::default())
            .expect_err("missing PPS must fail");
        assert!(matches!(err3, MuxerError::ValidationFailed { .. }));
        assert!(!path3.exists());
        assert!(!staged_path(&path3).exists());

        // 4. Truncated hvcC extradata
        let path4 = temp_output_path();
        let truncated_hvcc = vec![1, 1, 0x60, 0x00]; // less than 23 bytes
        let truncated_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: hevc_video_descriptor("hevc", Some(truncated_hvcc)),
                packets: vec![video_packet(0, 0, 33, true, avcc_payload(&[HEVC_IDR]))],
            }],
            captured_at_unix_ms: 0,
        };
        let err4 = muxer
            .write_snapshot(&truncated_snap, &path4, &SaveOptions::default())
            .expect_err("truncated hvcC must fail");
        assert!(matches!(err4, MuxerError::ValidationFailed { .. }));
        assert!(!path4.exists());
        assert!(!staged_path(&path4).exists());
    }

    #[test]
    fn hevc_truncated_sps_exp_golomb_fails_without_final_file() {
        let path = temp_output_path();
        let mut muxer = Mp4Muxer;

        // SPS truncated right after profile_tier_level
        let truncated_sps = vec![0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x00];
        let snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: hevc_video_descriptor("hevc", None),
                packets: vec![video_packet(
                    0,
                    0,
                    33,
                    true,
                    annex_b(&[HEVC_VPS, &truncated_sps, HEVC_PPS, HEVC_IDR]),
                )],
            }],
            captured_at_unix_ms: 0,
        };

        let err = muxer
            .write_snapshot(&snap, &path, &SaveOptions::default())
            .expect_err("truncated SPS must fail");
        assert!(matches!(err, MuxerError::ValidationFailed { .. }));
        assert!(!path.exists());
        assert!(!staged_path(&path).exists());
    }

    #[test]
    fn unsupported_codecs_fail_without_publishing_file() {
        let mut muxer = Mp4Muxer;
        for unsupported in &["vp8", "vp9", "prores", "dnxhd"] {
            let path = temp_output_path();
            let snap = MediaSnapshot {
                origin_pts: 0,
                time_base: TimeBase::MILLISECOND,
                streams: vec![StreamPackets {
                    descriptor: StreamDescriptor {
                        stream_id: StreamId(0),
                        media_type: MediaType::Video,
                        time_base: TimeBase::MILLISECOND,
                        name: None,
                        codec: unsupported.to_string(),
                        extradata: None,
                        width: Some(1920),
                        height: Some(1080),
                        sample_rate: None,
                        channels: None,
                        pixel_format: None,
                    },
                    packets: vec![video_packet(0, 0, 33, true, vec![1, 2, 3])],
                }],
                captured_at_unix_ms: 0,
            };

            let err = muxer
                .write_snapshot(&snap, &path, &SaveOptions::default())
                .expect_err("unsupported codec must fail");
            assert!(matches!(err, MuxerError::ValidationFailed { .. }));
            assert!(!path.exists());
            assert!(!staged_path(&path).exists());
        }
    }

    struct BitWriter {
        bytes: Vec<u8>,
        current_byte: u8,
        num_bits: u8,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                current_byte: 0,
                num_bits: 0,
            }
        }

        fn write_bits(&mut self, value: u64, n: usize) {
            for i in (0..n).rev() {
                let bit = ((value >> i) & 1) as u8;
                self.current_byte = (self.current_byte << 1) | bit;
                self.num_bits += 1;
                if self.num_bits == 8 {
                    self.bytes.push(self.current_byte);
                    self.current_byte = 0;
                    self.num_bits = 0;
                }
            }
        }

        fn write_bit(&mut self, bit: bool) {
            self.write_bits(if bit { 1 } else { 0 }, 1);
        }

        fn into_bytes(mut self) -> Vec<u8> {
            if self.num_bits > 0 {
                self.current_byte <<= 8 - self.num_bits;
                self.bytes.push(self.current_byte);
            }
            self.bytes
        }
    }

    fn build_test_av1_sequence_header(
        profile: u8,
        level: u8,
        tier: bool,
        high_bitdepth: bool,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let mut bw = BitWriter::new();
        bw.write_bits(profile as u64, 3); // seq_profile
        bw.write_bit(false); // still_picture
        bw.write_bit(false); // reduced_still_picture_header
        bw.write_bit(false); // timing_info_present_flag
        bw.write_bit(false); // initial_display_delay_present_flag
        bw.write_bits(0, 5); // operating_points_cnt_minus_1 = 0
        bw.write_bits(0, 12); // operating_point_idc[0] = 0
        bw.write_bits(level as u64, 5); // seq_level_idx[0]
        if level > 7 {
            bw.write_bit(tier); // seq_tier[0]
        }
        bw.write_bits(10, 4); // frame_width_bits_minus_1 = 10 (11 bits)
        bw.write_bits(10, 4); // frame_height_bits_minus_1 = 10 (11 bits)
        bw.write_bits((width - 1) as u64, 11); // max_frame_width_minus_1
        bw.write_bits((height - 1) as u64, 11); // max_frame_height_minus_1
        bw.write_bit(false); // frame_id_numbers_present_flag
        bw.write_bit(false); // use_128x128_superblock
        bw.write_bit(true); // enable_filter_intra
        bw.write_bit(true); // enable_intra_edge_filter
        bw.write_bit(true); // enable_interintra_compound
        bw.write_bit(true); // enable_masked_compound
        bw.write_bit(true); // enable_warped_motion
        bw.write_bit(true); // enable_dual_filter
        bw.write_bit(true); // enable_order_hint
        bw.write_bit(true); // enable_jnt_comp
        bw.write_bit(true); // enable_ref_frame_mvs
        bw.write_bit(true); // seq_choose_screen_content_tools
        bw.write_bit(true); // seq_choose_integer_mv
        bw.write_bits(3, 3); // order_hint_bits_minus_1
        bw.write_bit(false); // enable_superres
        bw.write_bit(true); // enable_cdef
        bw.write_bit(true); // enable_restoration
                            // color_config
        bw.write_bit(high_bitdepth); // high_bitdepth
        if profile == 2 && high_bitdepth {
            bw.write_bit(false); // twelve_bit
        }
        if profile != 1 {
            bw.write_bit(false); // monochrome
        }
        bw.write_bit(false); // color_description_present_flag
        bw.write_bit(false); // color_range
        if profile == 0 {
            bw.write_bits(0, 2); // chroma_sample_position = 0
        }
        bw.write_bit(false); // separate_uv_delta_q
        bw.write_bit(false); // film_grain_params_present
        bw.write_bit(true); // trailing_one_bit

        let payload = bw.into_bytes();
        let mut obu = Vec::new();
        obu.push(0x0A); // type 1, has_size_field=1
        encode_leb128(payload.len(), &mut obu);
        obu.extend_from_slice(&payload);
        obu
    }

    const AV1_FRAME_SIZED: &[u8] = &[0x32, 0x04, 0x11, 0x22, 0x33, 0x44];
    const AV1_FRAME_UNSIZED: &[u8] = &[0x30, 0x11, 0x22, 0x33, 0x44];

    fn av1_video_descriptor(codec: &str, extradata: Option<Vec<u8>>) -> StreamDescriptor {
        StreamDescriptor {
            stream_id: StreamId(0),
            media_type: MediaType::Video,
            time_base: TimeBase::MILLISECOND,
            name: None,
            codec: codec.to_string(),
            extradata: extradata.map(PacketPayload::from),
            width: Some(1920),
            height: Some(1080),
            sample_rate: None,
            channels: None,
            pixel_format: None,
        }
    }

    fn av1c_extradata(config_obus: &[u8], seq_level_idx: u8, high_bitdepth: bool) -> Vec<u8> {
        let mut output = vec![
            0x81,                                    // marker=1, version=1
            seq_level_idx & 0x1F,                    // seq_profile=0 (0 << 5), seq_level_idx_0
            if high_bitdepth { 0x4C } else { 0x0C }, // tier=0, high_bitdepth, twelve_bit=0, mono=0, x=1, y=1, pos=0
            0x00,                                    // delay not present
        ];
        output.extend_from_slice(config_obus);
        output
    }

    #[test]
    fn av1_with_complete_av1c_extradata_writes_and_reads_av01() {
        let path = temp_output_path();
        let seq_header_8bit = build_test_av1_sequence_header(0, 8, false, false, 1920, 1080);
        let av1c = av1c_extradata(&seq_header_8bit, 8, false);
        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![
                StreamPackets {
                    descriptor: av1_video_descriptor("av1", Some(av1c)),
                    packets: vec![
                        video_packet(0, 0, 33, true, AV1_FRAME_SIZED.to_vec()),
                        video_packet(33, 33, 33, false, AV1_FRAME_SIZED.to_vec()),
                    ],
                },
                StreamPackets {
                    descriptor: audio_descriptor(StreamId(1), Some("Audio"), 48000, 2, None),
                    packets: vec![audio_packet(StreamId(1), 0, 0, 21, vec![0x11, 0x22, 0x33])],
                },
            ],
            captured_at_unix_ms: 1_700_000_000,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux AV1 MP4");

        assert_eq!(metadata.video_codec.as_deref(), Some("av1"));
        assert_eq!(metadata.audio_codecs, vec!["aac".to_string()]);

        let file = File::open(&path).expect("open published file");
        let file_size = file.metadata().unwrap().len();
        let mut reader = mp4::Mp4Reader::read_header(file, file_size).expect("read mp4 header");

        assert_eq!(reader.tracks().len(), 2);
        let video_track = reader.tracks().get(&1).expect("video track 1");
        assert_eq!(video_track.track_type().unwrap(), TrackType::Video);
        assert_eq!(video_track.media_type().unwrap(), mp4::MediaType::AV1);
        assert_eq!(
            video_track.box_type().unwrap(),
            FourCC::from(mp4::BoxType::Av01Box)
        );
        assert_eq!(video_track.box_type().unwrap().value, *b"av01");
        assert_eq!(video_track.width(), 1920);
        assert_eq!(video_track.height(), 1080);

        let av1c_box = video_track.av1c().expect("av1c present on av01 track");
        assert_eq!(av1c_box.seq_profile, 0);
        assert_eq!(av1c_box.seq_level_idx_0, 8);
        assert!(!av1c_box.seq_tier_0);
        assert!(!av1c_box.high_bitdepth);
        assert!(!av1c_box.twelve_bit);
        assert!(!av1c_box.monochrome);
        assert!(av1c_box.chroma_subsampling_x);
        assert!(av1c_box.chroma_subsampling_y);
        assert_eq!(av1c_box.chroma_sample_position, 0);
        assert_eq!(av1c_box.config_obus, seq_header_8bit);

        // Check ftyp compatible brands contains av01
        assert!(reader
            .ftyp
            .compatible_brands
            .iter()
            .any(|b| b.value == *b"av01"));

        // Verify mdat samples are explicit size-prefixed OBUs
        let sample = reader.read_sample(1, 1).unwrap().unwrap();
        assert_eq!(sample.bytes.as_ref(), AV1_FRAME_SIZED);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn av1_sequence_header_obu_packet_fallback_8bit_and_10bit() {
        // Test 8-bit Sequence Header in first keyframe packet
        let path1 = temp_output_path();
        let seq_header_8bit = build_test_av1_sequence_header(0, 8, false, false, 1920, 1080);
        let mut keyframe_8bit = seq_header_8bit.clone();
        keyframe_8bit.extend_from_slice(AV1_FRAME_SIZED);

        let snap1 = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", None),
                packets: vec![
                    video_packet(0, 0, 33, true, keyframe_8bit),
                    video_packet(33, 33, 33, false, AV1_FRAME_SIZED.to_vec()),
                ],
            }],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        let meta1 = muxer
            .write_snapshot(&snap1, &path1, &SaveOptions::default())
            .expect("mux AV1 8-bit");
        assert_eq!(meta1.video_codec.as_deref(), Some("av1"));

        let file1 = File::open(&path1).expect("open published file");
        let file_size1 = file1.metadata().unwrap().len();
        let reader1 = mp4::Mp4Reader::read_header(file1, file_size1).expect("read header");
        let track1 = reader1.tracks().get(&1).unwrap();
        let av1c_1 = track1.av1c().unwrap();
        assert_eq!(av1c_1.seq_profile, 0);
        assert_eq!(av1c_1.seq_level_idx_0, 8);
        assert!(!av1c_1.seq_tier_0);
        assert!(!av1c_1.high_bitdepth);
        assert!(!av1c_1.twelve_bit);
        assert!(!av1c_1.monochrome);
        assert!(av1c_1.chroma_subsampling_x);
        assert!(av1c_1.chroma_subsampling_y);
        assert_eq!(av1c_1.config_obus, seq_header_8bit);
        let _ = fs::remove_file(path1);

        // Test 10-bit Sequence Header in first keyframe packet
        let path2 = temp_output_path();
        let seq_header_10bit = build_test_av1_sequence_header(0, 12, false, true, 1920, 1080);
        let mut keyframe_10bit = seq_header_10bit.clone();
        keyframe_10bit.extend_from_slice(AV1_FRAME_SIZED);

        let snap2 = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av01", None),
                packets: vec![
                    video_packet(0, 0, 33, true, keyframe_10bit),
                    video_packet(33, 33, 33, false, AV1_FRAME_SIZED.to_vec()),
                ],
            }],
            captured_at_unix_ms: 0,
        };

        let meta2 = muxer
            .write_snapshot(&snap2, &path2, &SaveOptions::default())
            .expect("mux AV1 10-bit");
        assert_eq!(meta2.video_codec.as_deref(), Some("av01"));

        let file2 = File::open(&path2).expect("open published file");
        let file_size2 = file2.metadata().unwrap().len();
        let reader2 = mp4::Mp4Reader::read_header(file2, file_size2).expect("read header");
        let track2 = reader2.tracks().get(&1).unwrap();
        let av1c_2 = track2.av1c().unwrap();
        assert_eq!(av1c_2.seq_profile, 0);
        assert_eq!(av1c_2.seq_level_idx_0, 12);
        assert!(!av1c_2.seq_tier_0);
        assert!(av1c_2.high_bitdepth);
        assert!(!av1c_2.twelve_bit);
        assert!(!av1c_2.monochrome);
        assert!(av1c_2.chroma_subsampling_x);
        assert!(av1c_2.chroma_subsampling_y);
        assert_eq!(av1c_2.config_obus, seq_header_10bit);
        let _ = fs::remove_file(path2);
    }

    #[test]
    fn av1_codec_aliases_accepted() {
        for alias in &["av1", "av01", "AV1", "AV01"] {
            let path = temp_output_path();
            let seq_header = build_test_av1_sequence_header(0, 8, false, false, 1920, 1080);
            let av1c = av1c_extradata(&seq_header, 8, false);
            let snapshot = MediaSnapshot {
                origin_pts: 0,
                time_base: TimeBase::MILLISECOND,
                streams: vec![StreamPackets {
                    descriptor: av1_video_descriptor(alias, Some(av1c)),
                    packets: vec![video_packet(0, 0, 33, true, AV1_FRAME_SIZED.to_vec())],
                }],
                captured_at_unix_ms: 0,
            };

            let mut muxer = Mp4Muxer;
            let metadata = muxer
                .write_snapshot(&snapshot, &path, &SaveOptions::default())
                .expect("mux AV1 alias");
            assert_eq!(metadata.video_codec.as_deref(), Some(*alias));

            let file = File::open(&path).expect("open published file");
            let file_size = file.metadata().unwrap().len();
            let reader = mp4::Mp4Reader::read_header(file, file_size).expect("read mp4 header");
            let video_track = reader.tracks().get(&1).unwrap();
            assert_eq!(video_track.media_type().unwrap(), mp4::MediaType::AV1);
            assert_eq!(video_track.box_type().unwrap().value, *b"av01");

            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn av1_sample_normalizer_multiobu_and_single_unsized() {
        let path = temp_output_path();
        let seq_header = build_test_av1_sequence_header(0, 8, false, false, 1920, 1080);
        let av1c = av1c_extradata(&seq_header, 8, false);

        // Packet 1: multi-OBU with explicit size fields (e.g. Temporal Delimiter + Frame)
        // Temporal delimiter: type 2 (0x10), has_size=1 (0x12), size=0 (0x00) -> [0x12, 0x00]
        let mut multi_obu = vec![0x12, 0x00];
        multi_obu.extend_from_slice(AV1_FRAME_SIZED);

        // Packet 2: single unsized OBU -> AV1_FRAME_UNSIZED ([0x30, 0x11, 0x22, 0x33, 0x44])
        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", Some(av1c)),
                packets: vec![
                    video_packet(0, 0, 33, true, multi_obu.clone()),
                    video_packet(33, 33, 33, false, AV1_FRAME_UNSIZED.to_vec()),
                ],
            }],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux AV1 samples");

        let file = File::open(&path).expect("open published file");
        let file_size = file.metadata().unwrap().len();
        let mut reader = mp4::Mp4Reader::read_header(file, file_size).expect("read mp4 header");

        // Verify sample 1 passed through multi-OBU
        let s1 = reader.read_sample(1, 1).unwrap().unwrap();
        assert_eq!(s1.bytes.as_ref(), multi_obu.as_slice());

        // Verify sample 2 normalized single unsized OBU to size-delimited
        let s2 = reader.read_sample(1, 2).unwrap().unwrap();
        assert_eq!(s2.bytes.as_ref(), AV1_FRAME_SIZED);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn av1_malformed_and_missing_sequence_header_rejections() {
        let mut muxer = Mp4Muxer;

        // 1. Missing Sequence Header (only Frame OBU in packet)
        let path1 = temp_output_path();
        let no_seq_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", None),
                packets: vec![video_packet(0, 0, 33, true, AV1_FRAME_SIZED.to_vec())],
            }],
            captured_at_unix_ms: 0,
        };
        let err1 = muxer
            .write_snapshot(&no_seq_snap, &path1, &SaveOptions::default())
            .expect_err("missing sequence header must fail");
        assert!(matches!(err1, MuxerError::ValidationFailed { .. }));
        assert!(!path1.exists());
        assert!(!staged_path(&path1).exists());

        // 2. Malformed LEB128 in Sequence Header
        let path2 = temp_output_path();
        let bad_leb_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", None),
                packets: vec![video_packet(
                    0,
                    0,
                    33,
                    true,
                    vec![0x0A, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00],
                )],
            }],
            captured_at_unix_ms: 0,
        };
        let err2 = muxer
            .write_snapshot(&bad_leb_snap, &path2, &SaveOptions::default())
            .expect_err("oversized LEB128 must fail");
        assert!(matches!(err2, MuxerError::ValidationFailed { .. }));
        assert!(!path2.exists());
        assert!(!staged_path(&path2).exists());

        // 3. Truncated Sequence Header
        let path3 = temp_output_path();
        let truncated_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", None),
                packets: vec![video_packet(0, 0, 33, true, vec![0x0A, 0x02, 0x00, 0x00])],
            }],
            captured_at_unix_ms: 0,
        };
        let err3 = muxer
            .write_snapshot(&truncated_snap, &path3, &SaveOptions::default())
            .expect_err("truncated Sequence Header must fail");
        assert!(matches!(err3, MuxerError::ValidationFailed { .. }));
        assert!(!path3.exists());
        assert!(!staged_path(&path3).exists());

        // 4. Invalid reduced_still_picture_header without still_picture
        // seq_profile=0 (3 bits: 0), still_picture=0 (1 bit: 0), reduced_still=1 (1 bit: 1) -> 0b00001 -> 0x08 in byte 0
        let path4 = temp_output_path();
        let invalid_reduced_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", None),
                packets: vec![video_packet(
                    0,
                    0,
                    33,
                    true,
                    vec![
                        0x0A, 0x0E, 0x08, 0x00, 0x00, 0x10, 0x3B, 0xFF, 0xC3, 0xA0, 0x19, 0xD8,
                        0x08, 0x08, 0x08, 0x00,
                    ],
                )],
            }],
            captured_at_unix_ms: 0,
        };
        let err4 = muxer
            .write_snapshot(&invalid_reduced_snap, &path4, &SaveOptions::default())
            .expect_err("invalid reduced still must fail");
        assert!(matches!(err4, MuxerError::ValidationFailed { .. }));
        assert!(!path4.exists());
        assert!(!staged_path(&path4).exists());

        // 5. Contradictory av1C extradata (outer profile 1 vs inner sequence header profile 0)
        let path5 = temp_output_path();
        let seq_header = build_test_av1_sequence_header(0, 8, false, false, 1920, 1080);
        let mut contradictory_av1c = av1c_extradata(&seq_header, 8, false);
        contradictory_av1c[1] = 0x28; // claims seq_profile = 1 (1 << 5)
        let contradictory_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", Some(contradictory_av1c)),
                packets: vec![video_packet(0, 0, 33, true, AV1_FRAME_SIZED.to_vec())],
            }],
            captured_at_unix_ms: 0,
        };
        let err5 = muxer
            .write_snapshot(&contradictory_snap, &path5, &SaveOptions::default())
            .expect_err("contradictory av1C must fail");
        assert!(matches!(err5, MuxerError::ValidationFailed { .. }));
        assert!(!path5.exists());
        assert!(!staged_path(&path5).exists());

        // 6. Sample OBU with reserved bit 0 set
        let path6 = temp_output_path();
        let av1c = av1c_extradata(&seq_header, 8, false);
        let bad_sample_bit_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", Some(av1c.clone())),
                packets: vec![video_packet(
                    0,
                    0,
                    33,
                    true,
                    vec![0x33, 0x04, 0x11, 0x22, 0x33, 0x44],
                )],
            }],
            captured_at_unix_ms: 0,
        };
        let err6 = muxer
            .write_snapshot(&bad_sample_bit_snap, &path6, &SaveOptions::default())
            .expect_err("sample OBU reserved bit must fail");
        assert!(matches!(err6, MuxerError::ValidationFailed { .. }));
        assert!(!path6.exists());
        assert!(!staged_path(&path6).exists());

        // 7. Sample OBU with reserved type 0
        let path7 = temp_output_path();
        let bad_type0_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", Some(av1c.clone())),
                packets: vec![video_packet(0, 0, 33, true, vec![0x02, 0x02, 0x11, 0x22])],
            }],
            captured_at_unix_ms: 0,
        };
        let err7 = muxer
            .write_snapshot(&bad_type0_snap, &path7, &SaveOptions::default())
            .expect_err("sample OBU type 0 must fail");
        assert!(matches!(err7, MuxerError::ValidationFailed { .. }));
        assert!(!path7.exists());
        assert!(!staged_path(&path7).exists());

        // 8. Sample OBU with unsupported Tile List type 8
        let path8 = temp_output_path();
        let bad_type8_snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", Some(av1c)),
                packets: vec![video_packet(0, 0, 33, true, vec![0x42, 0x02, 0x11, 0x22])],
            }],
            captured_at_unix_ms: 0,
        };
        let err8 = muxer
            .write_snapshot(&bad_type8_snap, &path8, &SaveOptions::default())
            .expect_err("sample OBU type 8 must fail");
        assert!(matches!(err8, MuxerError::ValidationFailed { .. }));
        assert!(!path8.exists());
        assert!(!staged_path(&path8).exists());
    }

    #[test]
    fn av1_sample_normalizer_canonicalizes_non_minimal_leb128_preserving_payload() {
        let path = temp_output_path();
        let seq_header = build_test_av1_sequence_header(0, 8, false, false, 1920, 1080);
        let av1c = av1c_extradata(&seq_header, 8, false);

        // Frame OBU with non-minimal 2-byte LEB128 [0x84, 0x00] encoding size 4
        let non_minimal_sample = vec![0x32, 0x84, 0x00, 0xAA, 0xBB, 0xCC, 0xDD];
        let expected_canonical_sample = vec![0x32, 0x04, 0xAA, 0xBB, 0xCC, 0xDD];

        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", Some(av1c)),
                packets: vec![video_packet(0, 0, 33, true, non_minimal_sample)],
            }],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux AV1 with non-minimal sample LEB128");

        let file = File::open(&path).expect("open published file");
        let file_size = file.metadata().unwrap().len();
        let mut reader = mp4::Mp4Reader::read_header(file, file_size).expect("read mp4 header");

        let s1 = reader.read_sample(1, 1).unwrap().unwrap();
        assert_eq!(s1.bytes.as_ref(), expected_canonical_sample.as_slice());
        // Verify payload bytes [0xAA, 0xBB, 0xCC, 0xDD] are byte-identical
        assert_eq!(&s1.bytes[2..], &[0xAA, 0xBB, 0xCC, 0xDD]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn av1_sequence_header_tail_and_trailing_bits_validation() {
        // 1. Valid sequence header parses successfully
        let valid_obu = build_test_av1_sequence_header(0, 8, false, false, 1920, 1080);
        let parsed =
            parse_av1_sequence_header_obu(&valid_obu).expect("valid sequence header must parse");
        assert_eq!(parsed.seq_profile, 0);
        assert_eq!(parsed.seq_level_idx_0, 8);
        assert!(!parsed.high_bitdepth);

        // Helper to generate customized sequence headers
        let build_custom_sh = |trailing_one: bool,
                               trailing_padding_non_zero: bool,
                               omit_film_grain: bool|
         -> Vec<u8> {
            let mut bw = BitWriter::new();
            bw.write_bits(0, 3); // seq_profile = 0
            bw.write_bit(false); // still_picture
            bw.write_bit(false); // reduced_still_picture_header
            bw.write_bit(false); // timing_info_present_flag
            bw.write_bit(false); // initial_display_delay_present_flag
            bw.write_bits(0, 5); // operating_points_cnt_minus_1 = 0
            bw.write_bits(0, 12); // operating_point_idc[0] = 0
            bw.write_bits(8, 5); // seq_level_idx[0] = 8
            bw.write_bit(false); // seq_tier[0]
            bw.write_bits(10, 4); // frame_width_bits_minus_1 = 10
            bw.write_bits(10, 4); // frame_height_bits_minus_1 = 10
            bw.write_bits(1919, 11); // max_frame_width_minus_1
            bw.write_bits(1079, 11); // max_frame_height_minus_1
            bw.write_bit(false); // frame_id_numbers_present_flag
            bw.write_bit(false); // use_128x128_superblock
            bw.write_bit(true); // enable_filter_intra
            bw.write_bit(true); // enable_intra_edge_filter
            bw.write_bit(true); // enable_interintra_compound
            bw.write_bit(true); // enable_masked_compound
            bw.write_bit(true); // enable_warped_motion
            bw.write_bit(true); // enable_dual_filter
            bw.write_bit(true); // enable_order_hint
            bw.write_bit(true); // enable_jnt_comp
            bw.write_bit(true); // enable_ref_frame_mvs
            bw.write_bit(true); // seq_choose_screen_content_tools
            bw.write_bit(true); // seq_choose_integer_mv
            bw.write_bits(3, 3); // order_hint_bits_minus_1
            bw.write_bit(false); // enable_superres
            bw.write_bit(true); // enable_cdef
            bw.write_bit(true); // enable_restoration
                                // color_config
            bw.write_bit(false); // high_bitdepth
            bw.write_bit(false); // monochrome
            bw.write_bit(false); // color_description_present_flag
            bw.write_bit(false); // color_range
            bw.write_bits(0, 2); // chroma_sample_position
            bw.write_bit(false); // separate_uv_delta_q
            if !omit_film_grain {
                bw.write_bit(false); // film_grain_params_present
                bw.write_bit(trailing_one); // trailing_one_bit
                if trailing_padding_non_zero {
                    bw.write_bit(true); // non-zero padding bit
                }
            }
            let payload = bw.into_bytes();
            let mut obu = Vec::new();
            obu.push(0x0A);
            encode_leb128(payload.len(), &mut obu);
            obu.extend_from_slice(&payload);
            obu
        };

        // 2. Truncation before required tail bits fails
        let mut truncated_obu = valid_obu.clone();
        truncated_obu.pop();
        truncated_obu[1] -= 1;
        let err_trunc =
            parse_av1_sequence_header_obu(&truncated_obu).expect_err("truncated tail must fail");
        assert!(err_trunc.contains("truncated") || err_trunc.contains("missing"));

        // 3. trailing_one_bit = 0 fails
        let zero_trailing_sh = build_custom_sh(false, false, false);
        let err_zero = parse_av1_sequence_header_obu(&zero_trailing_sh)
            .expect_err("trailing_one_bit=0 must fail");
        assert_eq!(err_zero, "invalid trailing_one_bit in Sequence Header OBU");

        // 4. Non-zero trailing padding bit fails
        let nonzero_padding_sh = build_custom_sh(true, true, false);
        let err_nonzero = parse_av1_sequence_header_obu(&nonzero_padding_sh)
            .expect_err("nonzero padding must fail");
        assert_eq!(
            err_nonzero,
            "invalid trailing zero bit or padding in Sequence Header OBU"
        );

        // 5. Extra non-zero trailing byte fails
        let mut extra_nonzero_byte_sh = valid_obu.clone();
        extra_nonzero_byte_sh.push(0x01);
        extra_nonzero_byte_sh[1] += 1; // update LEB128 size
        let err_extra = parse_av1_sequence_header_obu(&extra_nonzero_byte_sh)
            .expect_err("nonzero extra byte must fail");
        assert!(err_extra.contains("trailing zero bit") || err_extra.contains("padding"));

        // 6. Public mux path integration with valid Sequence Header succeeds
        let path = temp_output_path();
        let av1c = av1c_extradata(&valid_obu, 8, false);
        let snap = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", Some(av1c)),
                packets: vec![video_packet(0, 0, 33, true, AV1_FRAME_SIZED.to_vec())],
            }],
            captured_at_unix_ms: 0,
        };
        let mut muxer = Mp4Muxer;
        muxer
            .write_snapshot(&snap, &path, &SaveOptions::default())
            .expect("mux valid AV1 with trailing bits");
        let _ = fs::remove_file(path);

        // 7. Public mux path with malformed trailing bit fails
        let path_bad = temp_output_path();
        let bad_av1c = av1c_extradata(&zero_trailing_sh, 8, false);
        let snap_bad = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: av1_video_descriptor("av1", Some(bad_av1c)),
                packets: vec![video_packet(0, 0, 33, true, AV1_FRAME_SIZED.to_vec())],
            }],
            captured_at_unix_ms: 0,
        };
        let err_mux = muxer
            .write_snapshot(&snap_bad, &path_bad, &SaveOptions::default())
            .expect_err("mux with bad trailing bit must fail");
        assert!(matches!(err_mux, MuxerError::ValidationFailed { .. }));
        assert!(!path_bad.exists());
    }

    #[test]
    fn av1_extradata_scanning_rejects_reserved_and_tile_list_obu_types() {
        let seq_header = build_test_av1_sequence_header(0, 8, false, false, 1920, 1080);

        // Type 0 (reserved) in extradata -> rejected
        let mut bad_type0_extradata = vec![0x02, 0x01, 0x00];
        bad_type0_extradata.extend_from_slice(&seq_header);
        let err0 = extract_sequence_header_obu(&bad_type0_extradata).expect_err("type 0 must fail");
        assert!(matches!(err0, MuxerError::ValidationFailed { .. }));

        // Type 8 (Tile List) in extradata -> rejected
        let mut bad_type8_extradata = vec![0x42, 0x01, 0x00];
        bad_type8_extradata.extend_from_slice(&seq_header);
        let err8 = extract_sequence_header_obu(&bad_type8_extradata).expect_err("type 8 must fail");
        assert!(matches!(err8, MuxerError::ValidationFailed { .. }));

        // Type 9 (reserved) in extradata -> rejected
        let mut bad_type9_extradata = vec![0x4A, 0x01, 0x00];
        bad_type9_extradata.extend_from_slice(&seq_header);
        let err9 = extract_sequence_header_obu(&bad_type9_extradata).expect_err("type 9 must fail");
        assert!(matches!(err9, MuxerError::ValidationFailed { .. }));

        // Valid multi-OBU extradata (Type 2 Temporal Delimiter + Type 15 Padding + Type 1 Sequence Header) -> succeeds
        let mut multi_obu_extradata = vec![0x12, 0x00]; // type 2, size 0
        multi_obu_extradata.extend_from_slice(&[0x7A, 0x01, 0x00]); // type 15, size 1
        multi_obu_extradata.extend_from_slice(&seq_header);
        let extracted = extract_sequence_header_obu(&multi_obu_extradata)
            .expect("scanning valid multi-OBU extradata must succeed")
            .expect("should find sequence header");
        assert_eq!(extracted, seq_header);
    }

    #[test]
    fn hevc_with_auxiliary_nal_arrays_in_hvcc_succeeds() {
        let path = temp_output_path();
        let mut hvcc = hvcc_extradata(HEVC_VPS, HEVC_SPS, HEVC_PPS);

        // Modify numOfArrays from 3 to 4
        hvcc[22] = 4;
        // Append auxiliary SEI NAL array: type 39 (PREFIX_SEI_NUT), completeness=1 -> 0x80 | 39 = 0xA7
        hvcc.push(0xA7);
        hvcc.extend_from_slice(&1_u16.to_be_bytes()); // numNalus = 1
        let sei_payload = &[0x4E, 0x01, 0x01, 0x02]; // NAL type 39: (39 << 1) = 78 = 0x4E
        hvcc.extend_from_slice(&u16::try_from(sei_payload.len()).unwrap().to_be_bytes());
        hvcc.extend_from_slice(sei_payload);

        let snapshot = MediaSnapshot {
            origin_pts: 0,
            time_base: TimeBase::MILLISECOND,
            streams: vec![StreamPackets {
                descriptor: hevc_video_descriptor("hevc", Some(hvcc)),
                packets: vec![video_packet(0, 0, 33, true, avcc_payload(&[HEVC_IDR]))],
            }],
            captured_at_unix_ms: 0,
        };

        let mut muxer = Mp4Muxer;
        let metadata = muxer
            .write_snapshot(&snapshot, &path, &SaveOptions::default())
            .expect("mux HEVC with auxiliary SEI array");
        assert_eq!(metadata.video_codec.as_deref(), Some("hevc"));

        let file = File::open(&path).expect("open published file");
        let file_size = file.metadata().unwrap().len();
        let reader = mp4::Mp4Reader::read_header(file, file_size).expect("read mp4 header");
        let video_track = reader.tracks().get(&1).unwrap();
        assert_eq!(video_track.media_type().unwrap(), mp4::MediaType::H265);
        assert_eq!(video_track.box_type().unwrap().value, *b"hvc1");

        let _ = fs::remove_file(path);
    }
}
