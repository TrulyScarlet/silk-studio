use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::path::Path;

use crate::{Error, Mp4Config, Mp4Reader, Mp4Track, Mp4Writer, Result, TrackType};

/// Losslessly trim an MP4 media file from `start_seconds` to `end_seconds` in pure Rust.
///
/// Features:
/// - Fast bitstream copying with zero video/audio re-encoding or compression loss.
/// - Automatically snaps start time to the nearest preceding IDR keyframe so video playback starts cleanly.
/// - Allows selective inclusion of audio tracks (e.g. mute microphone or desktop audio).
/// - 100% self-contained with no external tools or runtime dependencies.
pub fn trim_mp4<R: Read + Seek, W: Write + Seek>(
    mut reader: Mp4Reader<R>,
    mut writer: Mp4Writer<W>,
    start_seconds: f64,
    end_seconds: f64,
    included_audio_tracks: Option<&[usize]>,
) -> Result<u64> {
    if end_seconds <= start_seconds {
        return Err(Error::InvalidData(
            "end_seconds must be greater than start_seconds",
        ));
    }

    // 1. Discover video track and audio tracks
    let mut video_track_id = None;
    let mut audio_tracks = Vec::new();

    let mut sorted_tracks: Vec<(u32, Mp4Track)> = reader
        .tracks()
        .iter()
        .map(|(&id, t)| (id, t.clone()))
        .collect();
    sorted_tracks.sort_by_key(|(id, _)| *id);

    let mut video_track_opt = None;
    for (track_id, track) in sorted_tracks {
        match track.track_type() {
            Ok(TrackType::Video) => {
                if video_track_id.is_none() {
                    video_track_id = Some(track_id);
                    video_track_opt = Some(track);
                }
            }
            Ok(TrackType::Audio) => {
                audio_tracks.push((track_id, track));
            }
            _ => {}
        }
    }

    let video_track_id = video_track_id.ok_or(Error::InvalidData("no video track found"))?;
    let video_track = video_track_opt.unwrap();
    let video_timescale = video_track.timescale() as f64;
    let video_sample_count = video_track.sample_count();

    // 2. Find start keyframe (IDR) at or before start_seconds taking composition offsets (ctts) into account
    let mut start_sample_id = 1;
    let mut effective_start_pts_video: u64 = 0;

    for s_id in 1..=video_sample_count {
        if let Ok((dts_ticks, _)) = video_track.sample_time(s_id) {
            let offset = video_track.sample_rendering_offset(s_id);
            let pts_ticks = (dts_ticks as i64 + offset as i64).max(0) as u64;
            let pts_sec = (pts_ticks as f64) / video_timescale;
            let dts_sec = (dts_ticks as f64) / video_timescale;
            if pts_sec > start_seconds && dts_sec > start_seconds {
                break;
            }
            if video_track.is_sync_sample(s_id) {
                start_sample_id = s_id;
                effective_start_pts_video = dts_ticks;
            }
        }
    }

    let effective_start_sec = (effective_start_pts_video as f64) / video_timescale;

    // 3. Register video track into writer
    let new_video_track_id = writer.add_track_from_trak(video_track.trak());

    // 4. Register selected audio tracks
    let mut audio_mappings = Vec::new();
    for (audio_idx, (orig_id, track)) in audio_tracks.iter().enumerate() {
        let include = match included_audio_tracks {
            Some(tracks) => tracks.contains(&audio_idx),
            None => true,
        };
        if include {
            let new_id = writer.add_track_from_trak(track.trak());
            audio_mappings.push((*orig_id, new_id, track.clone()));
        }
    }

    // 5. Copy video samples from start_sample_id until end_seconds (preserving B-frame sequences)
    let mut video_written = 0_u64;
    for s_id in start_sample_id..=video_sample_count {
        if let Ok((dts_ticks, _)) = video_track.sample_time(s_id) {
            let offset = video_track.sample_rendering_offset(s_id);
            let pts_ticks = (dts_ticks as i64 + offset as i64).max(0) as u64;
            let pts_sec = (pts_ticks as f64) / video_timescale;
            let dts_sec = (dts_ticks as f64) / video_timescale;
            if pts_sec > end_seconds && dts_sec > end_seconds {
                break;
            }
        }
        if let Some(mut sample) = reader.read_sample(video_track_id, s_id)? {
            sample.start_time = sample.start_time.saturating_sub(effective_start_pts_video);
            writer.write_sample(new_video_track_id, &sample)?;
            video_written += 1;
        }
    }

    // 6. Copy audio samples aligned to effective_start_sec
    for (orig_id, new_id, audio_track) in audio_mappings {
        let audio_timescale = audio_track.timescale() as f64;
        let audio_start_ticks = (effective_start_sec * audio_timescale).round() as u64;
        let audio_end_ticks = (end_seconds * audio_timescale).round() as u64;
        let audio_sample_count = audio_track.sample_count();

        for s_id in 1..=audio_sample_count {
            if let Ok((start_ticks, _)) = audio_track.sample_time(s_id) {
                if start_ticks < audio_start_ticks {
                    continue;
                }
                if start_ticks > audio_end_ticks {
                    break;
                }
            }
            if let Some(mut sample) = reader.read_sample(orig_id, s_id)? {
                sample.start_time = sample.start_time.saturating_sub(audio_start_ticks);
                writer.write_sample(new_id, &sample)?;
            }
        }
    }

    // 7. Finalize container headers
    writer.write_end()?;

    Ok(video_written)
}

/// Convenience helper to trim an MP4 file on disk.
pub fn trim_mp4_file(
    source_path: &Path,
    output_path: &Path,
    start_seconds: f64,
    end_seconds: f64,
    included_audio_tracks: Option<&[usize]>,
) -> Result<u64> {
    let file = File::open(source_path)?;
    let size = file.metadata()?.len();
    let reader = Mp4Reader::read_header(BufReader::new(file), size)?;

    let out_file = File::create(output_path)?;
    let writer = Mp4Writer::write_start(
        BufWriter::new(out_file),
        &Mp4Config {
            major_brand: *reader.major_brand(),
            minor_version: reader.minor_version(),
            compatible_brands: reader.compatible_brands().to_vec(),
            timescale: reader.timescale(),
        },
    )?;

    trim_mp4(
        reader,
        writer,
        start_seconds,
        end_seconds,
        included_audio_tracks,
    )
}

/// Extract a single audio track from an MP4 file into an M4A audio file losslessly without re-encoding.
pub fn extract_audio_track_file(
    source_path: &Path,
    output_path: &Path,
    track_id: u32,
) -> Result<u64> {
    let file = File::open(source_path)?;
    let size = file.metadata()?.len();
    let mut reader = Mp4Reader::read_header(BufReader::new(file), size)?;

    let track = reader
        .tracks()
        .get(&track_id)
        .cloned()
        .ok_or(Error::TrakNotFound(track_id))?;
    let sample_count = track.sample_count();

    let out_file = File::create(output_path)?;
    let mut writer = Mp4Writer::write_start(
        BufWriter::new(out_file),
        &Mp4Config {
            major_brand: crate::FourCC::from(*b"M4A "),
            minor_version: 0,
            compatible_brands: vec![
                crate::FourCC::from(*b"M4A "),
                crate::FourCC::from(*b"mp42"),
                crate::FourCC::from(*b"isom"),
            ],
            timescale: track.timescale(),
        },
    )?;

    let _ = writer.add_track_from_trak(&track.trak);

    let mut samples_written = 0_u64;
    for sample_id in 1..=sample_count {
        if let Some(sample) = reader.read_sample(track_id, sample_id)? {
            writer.write_sample(1, &sample)?;
            samples_written += 1;
        }
    }

    writer.write_end()?;
    Ok(samples_written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AacConfig, AudioObjectType, AvcConfig, ChannelConfig, MediaConfig, Mp4Sample,
        SampleFreqIndex,
    };
    use crate::{FourCC, TrackConfig, TrackType};
    use bytes::Bytes;
    use std::io::Cursor;

    #[test]
    fn test_trim_mp4_in_memory() {
        let mp4_config = Mp4Config {
            major_brand: FourCC::from(*b"isom"),
            minor_version: 512,
            compatible_brands: vec![FourCC::from(*b"isom"), FourCC::from(*b"iso2")],
            timescale: 1000,
        };

        let mut data = Cursor::new(Vec::new());
        let mut writer = Mp4Writer::write_start(&mut data, &mp4_config).unwrap();

        let avc_conf = AvcConfig {
            width: 1920,
            height: 1080,
            seq_param_set: vec![0x67, 0x42, 0x00, 0x1f],
            pic_param_set: vec![0x68, 0xce, 0x3c, 0x80],
        };
        let video_track = TrackConfig {
            track_type: TrackType::Video,
            timescale: 1000,
            language: "und".to_string(),
            media_conf: MediaConfig::AvcConfig(avc_conf),
            track_name: None,
        };
        writer.add_track(&video_track).unwrap();

        let aac_conf = AacConfig {
            bitrate: 192_000,
            profile: AudioObjectType::AacLowComplexity,
            freq_index: SampleFreqIndex::Freq48000,
            chan_conf: ChannelConfig::Stereo,
        };
        let audio_track = TrackConfig {
            track_type: TrackType::Audio,
            timescale: 1000,
            language: "und".to_string(),
            media_conf: MediaConfig::AacConfig(aac_conf),
            track_name: Some("Game Audio".to_string()),
        };
        writer.add_track(&audio_track).unwrap();

        // 4 video samples: 0s (sync), 1s (non-sync), 2s (sync), 3s (non-sync)
        writer
            .write_sample(
                1,
                &Mp4Sample {
                    start_time: 0,
                    duration: 1000,
                    rendering_offset: 0,
                    is_sync: true,
                    bytes: Bytes::from(vec![0x00, 0x00, 0x00, 0x01, 0x26]),
                },
            )
            .unwrap();
        writer
            .write_sample(
                1,
                &Mp4Sample {
                    start_time: 1000,
                    duration: 1000,
                    rendering_offset: 0,
                    is_sync: false,
                    bytes: Bytes::from(vec![0x00, 0x00, 0x00, 0x01, 0x01]),
                },
            )
            .unwrap();
        writer
            .write_sample(
                1,
                &Mp4Sample {
                    start_time: 2000,
                    duration: 1000,
                    rendering_offset: 0,
                    is_sync: true,
                    bytes: Bytes::from(vec![0x00, 0x00, 0x00, 0x01, 0x26]),
                },
            )
            .unwrap();
        writer
            .write_sample(
                1,
                &Mp4Sample {
                    start_time: 3000,
                    duration: 1000,
                    rendering_offset: 0,
                    is_sync: false,
                    bytes: Bytes::from(vec![0x00, 0x00, 0x00, 0x01, 0x01]),
                },
            )
            .unwrap();

        // 4 audio samples at 0s, 1s, 2s, 3s
        for t in 0..4 {
            writer
                .write_sample(
                    2,
                    &Mp4Sample {
                        start_time: t * 1000,
                        duration: 1000,
                        rendering_offset: 0,
                        is_sync: true,
                        bytes: Bytes::from(vec![0x21, 0x10, 0x01]),
                    },
                )
                .unwrap();
        }

        writer.write_end().unwrap();

        // Now read back and trim from 1.5s to 3.5s
        let buf = data.into_inner();
        let size = buf.len() as u64;
        let reader = Mp4Reader::read_header(Cursor::new(buf), size).unwrap();

        let mut out_data = Cursor::new(Vec::new());
        let out_writer = Mp4Writer::write_start(&mut out_data, &mp4_config).unwrap();

        let written = trim_mp4(reader, out_writer, 2.0, 3.5, None).unwrap();
        assert_eq!(written, 2); // Samples at 2s and 3s

        // Validate trimmed output container
        let out_buf = out_data.into_inner();
        let out_size = out_buf.len() as u64;
        let mut trimmed_reader = Mp4Reader::read_header(Cursor::new(out_buf), out_size).unwrap();

        assert_eq!(trimmed_reader.tracks().len(), 2);
        let vid = trimmed_reader.tracks().get(&1).unwrap();
        assert_eq!(vid.sample_count(), 2);

        let s1 = trimmed_reader.read_sample(1, 1).unwrap().unwrap();
        assert_eq!(s1.start_time, 0); // Rebased to 0!
        assert!(s1.is_sync);

        let s2 = trimmed_reader.read_sample(1, 2).unwrap().unwrap();
        assert_eq!(s2.start_time, 1000);
    }
}
