//! Mock encoders and muxer with injectable failure points.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use encoder_api::{
    AudioEncoder, AudioEncoderConfig, EncoderCapabilities, EncoderError, Result as EncResult,
    VideoEncoder, VideoEncoderConfig,
};
use media_types::MediaSnapshot;
use media_types::{
    AudioFrame, EncodedPacket, MediaType, PacketPayload, PixelFormat, TimeBase, VideoFrame,
};
use muxer::{validate_snapshot, ClipMetadata, Muxer, MuxerError, Result as MuxResult, SaveOptions};

/// Software-encoder stand-in: emits deterministic packets, one per frame.
#[derive(Default)]
pub struct MockVideoEncoder {
    /// When set, the Nth `encode` call (0-based) fails.
    pub fail_on_encode: Option<usize>,
    pub keyframe_every: u32,
    /// Optional diagnostic identity used by fallback tests.
    pub backend_name: Option<String>,
    pub codec: Option<String>,
    pub hardware_accelerated: bool,
    /// Simulates B-frame-style reordering: DTS trails PTS by N frames
    /// (exercises distinct PTS/DTS normalization downstream).
    pub dts_lag_frames: u32,
    /// Number of successful configuration calls, useful for recovery tests.
    pub configure_count: Arc<AtomicUsize>,
    pub max_dimension: Option<u32>,
    pub fail_if_resolution: Option<(u32, u32)>,
    pub supported_pixel_formats: Option<Vec<PixelFormat>>,
    pub configured_configs: Arc<Mutex<Vec<VideoEncoderConfig>>>,
    encoded: usize,
    configured: bool,
    extradata: Option<PacketPayload>,
}

impl MockVideoEncoder {
    pub fn new() -> Self {
        Self {
            keyframe_every: 30,
            ..Self::default()
        }
    }
}

impl VideoEncoder for MockVideoEncoder {
    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities {
            backend_name: self
                .backend_name
                .clone()
                .unwrap_or_else(|| "mock-sw".to_string()),
            codec: self.codec.clone().unwrap_or_else(|| "avc".to_string()),
            hardware_accelerated: self.hardware_accelerated,
            max_width: 7680,
            max_height: 4320,
            supported_fps: vec![30, 60, 120],
            supported_pixel_formats: self
                .supported_pixel_formats
                .clone()
                .unwrap_or_else(|| vec![PixelFormat::Nv12]),
        }
    }

    fn configure(&mut self, config: VideoEncoderConfig) -> EncResult<()> {
        if !matches!(config.fps, 30 | 60 | 120) {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: format!("fps must be 30, 60, or 120, got {}", config.fps),
            });
        }
        if config.width == 0 || config.height == 0 {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "zero-sized output is invalid".to_string(),
            });
        }
        if let Some(max) = self.max_dimension {
            if config.width > max || config.height > max {
                return Err(EncoderError::UnsupportedConfiguration {
                    reason: format!(
                        "dimensions {}x{} exceed maximum supported dimension {}",
                        config.width, config.height, max
                    ),
                });
            }
        }
        if let Some((fail_w, fail_h)) = self.fail_if_resolution {
            if config.width == fail_w && config.height == fail_h {
                return Err(EncoderError::UnsupportedConfiguration {
                    reason: format!("resolution {fail_w}x{fail_h} is explicitly rejected by mock"),
                });
            }
        }
        if let Ok(mut list) = self.configured_configs.lock() {
            list.push(config.clone());
        }
        self.codec = Some(config.codec.clone());
        self.configure_count.fetch_add(1, Ordering::SeqCst);
        self.configured = true;
        if self.extradata.is_none() {
            self.extradata = Some(PacketPayload::from(vec![
                1, 0x42, 0x00, 0x1E, 0xFF, 0xE1, 0x00, 0x09, 0x67, 0x42, 0x00, 0x1E, 0xE9, 0x01,
                0x40, 0x7B, 0x20, 0x01, 0x00, 0x04, 0x68, 0xCE, 0x3C, 0x80,
            ]));
        }
        Ok(())
    }

    fn encode(&mut self, frame: VideoFrame) -> EncResult<Vec<EncodedPacket>> {
        if !self.configured {
            return Err(EncoderError::NotConfigured);
        }
        if self.fail_on_encode == Some(self.encoded) {
            return Err(EncoderError::EncodeFailed {
                details: format!("injected failure on encode #{}", self.encoded),
            });
        }
        let index = self.encoded;
        self.encoded += 1;
        let keyframe_period = if self.keyframe_every == 0 {
            1
        } else {
            self.keyframe_every
        };
        // Keep the mock elementary stream shaped like the native H.264
        // backend so container integration tests exercise packet conversion.
        let mut payload = Vec::new();
        payload.extend_from_slice(&[0, 0, 0, 1]);
        if index == 0 {
            payload.extend_from_slice(&[0x67, 0x42, 0x00, 0x1E, 0xE9, 0x01, 0x40, 0x7B, 0x20]);
            payload.extend_from_slice(&[0, 0, 0, 1, 0x68, 0xCE, 0x3C, 0x80]);
        }
        payload.push(
            if (index as u64).is_multiple_of(u64::from(keyframe_period)) {
                0x65
            } else {
                0x41
            },
        );
        payload.extend_from_slice(&[index as u8, 0x88, 0x84, 0x00]);
        let dts = (frame.pts - i64::from(self.dts_lag_frames) * 33).max(0);
        Ok(vec![EncodedPacket {
            stream_id: frame.stream_id,
            media_type: MediaType::Video,
            pts: frame.pts,
            dts,
            duration: 33,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: (index as u64).is_multiple_of(u64::from(keyframe_period)),
            sequence: 0,
            payload: PacketPayload::from(payload),
        }])
    }

    fn drain(&mut self) -> EncResult<Vec<EncodedPacket>> {
        Ok(Vec::new())
    }

    fn codec_extradata(&self) -> Option<PacketPayload> {
        self.extradata.clone()
    }
}

/// Audio encoder stand-in: one packet per chunk, payload passthrough.
#[derive(Default)]
pub struct MockAudioEncoder {
    configured: bool,
    extradata: Option<PacketPayload>,
}

impl MockAudioEncoder {
    /// Attach deterministic codec initialization bytes for descriptor tests.
    pub fn with_extradata(mut self, extradata: impl Into<PacketPayload>) -> Self {
        self.extradata = Some(extradata.into());
        self
    }
}

impl AudioEncoder for MockAudioEncoder {
    fn configure(&mut self, config: AudioEncoderConfig) -> EncResult<()> {
        if config.channels == 0 || config.sample_rate == 0 {
            return Err(EncoderError::UnsupportedConfiguration {
                reason: "audio channels and sample rate must be positive".to_string(),
            });
        }
        self.configured = true;
        if self.extradata.is_none() {
            self.extradata = Some(PacketPayload::from(
                mock_aac_audio_specific_config(config.sample_rate, config.channels).to_vec(),
            ));
        }
        Ok(())
    }

    fn encode(&mut self, frame: AudioFrame) -> EncResult<Vec<EncodedPacket>> {
        if !self.configured {
            return Err(EncoderError::NotConfigured);
        }
        let duration_ms =
            i64::from(frame.sample_count) * 1000 / i64::from(frame.sample_rate.max(1));
        Ok(vec![EncodedPacket {
            stream_id: frame.stream_id,
            media_type: MediaType::Audio,
            pts: frame.pts,
            dts: frame.pts,
            duration: duration_ms,
            time_base: TimeBase::MILLISECOND,
            is_keyframe: true,
            sequence: 0,
            payload: PacketPayload::from(frame.data.to_vec()),
        }])
    }

    fn drain(&mut self) -> EncResult<Vec<EncodedPacket>> {
        Ok(Vec::new())
    }

    fn codec_extradata(&self) -> Option<PacketPayload> {
        self.extradata.clone()
    }
}

fn mock_aac_audio_specific_config(sample_rate: u32, channels: u16) -> [u8; 2] {
    let sample_rate_index = match sample_rate {
        96_000 => 0,
        88_200 => 1,
        64_000 => 2,
        48_000 => 3,
        44_100 => 4,
        32_000 => 5,
        24_000 => 6,
        22_050 => 7,
        16_000 => 8,
        12_000 => 9,
        11_025 => 10,
        8_000 => 11,
        7_350 => 12,
        _ => 3,
    };
    let channel_configuration = match channels {
        1..=6 => channels,
        _ => 2,
    };
    let bits = (2_u16 << 11) | ((sample_rate_index as u16) << 7) | (channel_configuration << 3);
    [(bits >> 8) as u8, bits as u8]
}

/// Muxer stand-in that honors the staging + atomic rename contract using
/// a recognizable placeholder header instead of real containers.
#[derive(Default)]
pub struct MockMuxer {
    /// When set, the Nth `write_snapshot` call (0-based) fails.
    pub fail_on_call: Option<usize>,
    /// Optional deterministic delay for save-overlap tests.
    pub write_delay_ms: u64,
    calls: Mutex<usize>,
}

const MOCK_HEADER: &[u8] = b"SILKMOCKV1";

impl MockMuxer {
    pub fn call_count(&self) -> usize {
        *self.calls.lock().expect("muxer counter")
    }
}

impl Muxer for MockMuxer {
    fn write_snapshot(
        &mut self,
        snapshot: &MediaSnapshot,
        final_path: &Path,
        _options: &SaveOptions,
    ) -> MuxResult<ClipMetadata> {
        {
            let mut calls = self.calls.lock().map_err(|_| MuxerError::WriteFailed {
                details: "counter poisoned".to_string(),
            })?;
            let call_index = *calls;
            *calls += 1;
            if self.fail_on_call == Some(call_index) {
                return Err(MuxerError::WriteFailed {
                    details: "injected mux failure".to_string(),
                });
            }
        }

        if self.write_delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.write_delay_ms));
        }

        validate_snapshot(snapshot)?;

        let video = snapshot
            .streams
            .iter()
            .find(|s| s.descriptor.media_type == MediaType::Video)
            .ok_or(MuxerError::ValidationFailed {
                reason: "snapshot has no video stream".to_string(),
            })?;
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| MuxerError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        // Stage under a distinct temporary name, then publish atomically
        // (MUX-006/007 contract rehearsal).
        let mut temp_name = final_path.as_os_str().to_os_string();
        temp_name.push(".part");
        let temp_path = PathBuf::from(temp_name);

        let packet_count = snapshot.packet_count();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MOCK_HEADER);
        bytes.extend_from_slice(&(packet_count as u64).to_le_bytes());
        for stream in &snapshot.streams {
            for packet in &stream.packets {
                bytes.extend_from_slice(&(packet.payload.len() as u32).to_le_bytes());
                bytes.extend_from_slice(packet.payload.as_slice());
            }
        }
        let result = (|| {
            std::fs::write(&temp_path, &bytes).map_err(|source| MuxerError::Io {
                path: temp_path.clone(),
                source,
            })?;
            let size_bytes = std::fs::metadata(&temp_path)
                .map(|m| m.len())
                .map_err(|source| MuxerError::Io {
                    path: temp_path.clone(),
                    source,
                })?;
            std::fs::rename(&temp_path, final_path).map_err(|source| MuxerError::Io {
                path: final_path.to_path_buf(),
                source,
            })?;
            Ok(size_bytes)
        })();
        let size_bytes = match result {
            Ok(size_bytes) => size_bytes,
            Err(error) => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(error);
            }
        };

        Ok(ClipMetadata {
            path: final_path.to_path_buf(),
            duration_ms: snapshot.video_duration_ticks().unwrap_or(0).max(0) as u64,
            size_bytes,
            created_at_unix_ms: snapshot.captured_at_unix_ms,
            video_codec: Some(video.descriptor.codec.clone()),
            audio_codecs: snapshot
                .streams
                .iter()
                .filter(|s| s.descriptor.media_type == MediaType::Audio)
                .map(|s| s.descriptor.codec.clone())
                .collect(),
        })
    }
}
