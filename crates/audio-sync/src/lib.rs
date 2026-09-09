//! Audio format conversion and shared-timeline synchronization (Segment S4).
//!
//! This crate owns the deterministic part of the audio path: validating
//! packets, converting sample formats/channels, resampling to the encoder
//! target, applying a bounded gain, and keeping output timestamps monotonic.
//! Device access remains in `audio-wasapi` and audio encoding remains behind
//! `encoder-api`.

use std::sync::Arc;

use media_clock::{Discontinuity, MediaTimeline};
use media_types::{AudioFrame, SampleFormat, TimeBase};
use thiserror::Error;

pub const DEFAULT_AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const DEFAULT_AUDIO_CHANNELS: u16 = 2;
pub const DEFAULT_MAX_DISCONTINUITY_MS: i64 = 60_000;
pub const DEFAULT_MICROPHONE_GAIN: f32 = 1.0;

pub type Result<T, E = AudioSyncError> = std::result::Result<T, E>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AudioSyncError {
    #[error("invalid audio synchronization configuration: {reason}")]
    InvalidConfiguration { reason: String },

    #[error("audio frame format changed during a session")]
    FormatChanged,

    #[error("audio frame payload is truncated: expected {expected} bytes, got {actual}")]
    PayloadTruncated { expected: usize, actual: usize },

    #[error("audio timestamp discontinuity: {0}")]
    Discontinuity(Discontinuity),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioSyncConfig {
    pub target_sample_rate: u32,
    pub target_channels: u16,
    /// Linear multiplier applied after channel conversion and resampling.
    pub gain: f32,
    /// Maximum accepted jump between calibrated source timestamps.
    pub max_discontinuity_ms: i64,
}

impl Default for AudioSyncConfig {
    fn default() -> Self {
        Self {
            target_sample_rate: DEFAULT_AUDIO_SAMPLE_RATE,
            target_channels: DEFAULT_AUDIO_CHANNELS,
            gain: DEFAULT_MICROPHONE_GAIN,
            max_discontinuity_ms: DEFAULT_MAX_DISCONTINUITY_MS,
        }
    }
}

impl AudioSyncConfig {
    pub fn validate(&self) -> Result<()> {
        if self.target_sample_rate == 0 {
            return Err(AudioSyncError::InvalidConfiguration {
                reason: "target sample rate must be positive".to_string(),
            });
        }
        if self.target_channels == 0 {
            return Err(AudioSyncError::InvalidConfiguration {
                reason: "target channel count must be positive".to_string(),
            });
        }
        if !self.gain.is_finite() || !(0.0..=8.0).contains(&self.gain) {
            return Err(AudioSyncError::InvalidConfiguration {
                reason: "gain must be finite and between 0 and 8".to_string(),
            });
        }
        if self.max_discontinuity_ms <= 0 {
            return Err(AudioSyncError::InvalidConfiguration {
                reason: "maximum discontinuity must be positive".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioSyncMetrics {
    pub input_chunks: u64,
    pub output_chunks: u64,
    pub input_samples: u64,
    pub output_samples: u64,
    pub resampled_chunks: u64,
    pub channel_converted_chunks: u64,
    pub discontinuities: u64,
    pub last_drift_ms: i64,
    pub max_abs_drift_ms: u64,
}

/// Converts one source stream into the format expected by the audio encoder.
/// Each source owns one synchronizer so initial device offsets are compensated
/// independently while the resulting packets use the shared millisecond base.
#[derive(Debug, Clone)]
pub struct AudioSynchronizer {
    config: AudioSyncConfig,
    source_time_base: Option<TimeBase>,
    timeline: Option<MediaTimeline>,
    timeline_offset_ms: i64,
    last_output_pts: Option<i64>,
    expected_next_pts: Option<i64>,
    metrics: AudioSyncMetrics,
}

impl AudioSynchronizer {
    pub fn new(config: AudioSyncConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            source_time_base: None,
            timeline: None,
            timeline_offset_ms: 0,
            last_output_pts: None,
            expected_next_pts: None,
            metrics: AudioSyncMetrics::default(),
        })
    }

    pub fn config(&self) -> &AudioSyncConfig {
        &self.config
    }

    pub fn metrics(&self) -> AudioSyncMetrics {
        self.metrics.clone()
    }

    /// Reset source calibration after a device restart or an accepted
    /// discontinuity. Counters remain session totals for diagnostics.
    pub fn reset_timeline(&mut self) {
        self.source_time_base = None;
        self.timeline = None;
        self.timeline_offset_ms = 0;
        self.last_output_pts = None;
        self.expected_next_pts = None;
    }

    /// Reset only the source calibration after a device restart while keeping
    /// output timestamps contiguous with the packets already emitted.
    pub fn reset_source(&mut self) {
        self.source_time_base = None;
        self.timeline = None;
        self.timeline_offset_ms = self
            .expected_next_pts
            .or_else(|| self.last_output_pts.map(|pts| pts.saturating_add(1)))
            .unwrap_or(0);
    }

    pub fn process(&mut self, frame: AudioFrame) -> Result<AudioFrame> {
        validate_frame(&frame)?;
        if let Some(time_base) = self.source_time_base {
            if time_base != frame.time_base {
                return Err(AudioSyncError::FormatChanged);
            }
        } else {
            self.source_time_base = Some(frame.time_base);
            self.timeline = Some(MediaTimeline::new(
                frame.time_base,
                self.config.max_discontinuity_ms,
            ));
        }

        let timeline = self.timeline.as_mut().expect("timeline initialized above");
        let mut shared_pts = timeline.to_shared(frame.pts).map_err(|error| {
            self.metrics.discontinuities = self.metrics.discontinuities.saturating_add(1);
            AudioSyncError::Discontinuity(error)
        })?;
        shared_pts = shared_pts.saturating_add(self.timeline_offset_ms);

        if let Some(previous) = self.last_output_pts {
            if shared_pts < previous {
                shared_pts = previous;
            }
        }

        if let Some(expected) = self.expected_next_pts {
            let drift = shared_pts.saturating_sub(expected);
            self.metrics.last_drift_ms = drift;
            self.metrics.max_abs_drift_ms = self.metrics.max_abs_drift_ms.max(drift.unsigned_abs());
        }

        let source_samples = decode_samples(&frame)?;
        let converted = convert_channels(
            &source_samples,
            frame.sample_count as usize,
            frame.channels,
            self.config.target_channels,
        );
        if frame.channels != self.config.target_channels {
            self.metrics.channel_converted_chunks =
                self.metrics.channel_converted_chunks.saturating_add(1);
        }

        let (resampled, output_samples) = resample_linear(
            &converted,
            frame.sample_count as usize,
            self.config.target_channels,
            frame.sample_rate,
            self.config.target_sample_rate,
        );
        if frame.sample_rate != self.config.target_sample_rate {
            self.metrics.resampled_chunks = self.metrics.resampled_chunks.saturating_add(1);
        }

        let mut bytes = Vec::with_capacity(resampled.len() * std::mem::size_of::<f32>());
        for sample in resampled {
            bytes.extend_from_slice(&(sample * self.config.gain).to_le_bytes());
        }

        let output_samples = u32::try_from(output_samples).unwrap_or(u32::MAX);
        let output_duration_ms = i64::from(output_samples).saturating_mul(1_000)
            / i64::from(self.config.target_sample_rate);
        self.last_output_pts = Some(shared_pts);
        self.expected_next_pts = Some(shared_pts.saturating_add(output_duration_ms.max(1)));
        self.metrics.input_chunks = self.metrics.input_chunks.saturating_add(1);
        self.metrics.output_chunks = self.metrics.output_chunks.saturating_add(1);
        self.metrics.input_samples = self
            .metrics
            .input_samples
            .saturating_add(u64::from(frame.sample_count));
        self.metrics.output_samples = self
            .metrics
            .output_samples
            .saturating_add(u64::from(output_samples));

        Ok(AudioFrame {
            stream_id: frame.stream_id,
            sample_format: SampleFormat::F32,
            sample_rate: self.config.target_sample_rate,
            channels: self.config.target_channels,
            sample_count: output_samples,
            pts: shared_pts,
            time_base: TimeBase::MILLISECOND,
            data: Arc::from(bytes.into_boxed_slice()),
        })
    }
}

fn validate_frame(frame: &AudioFrame) -> Result<()> {
    if frame.sample_rate == 0 || frame.channels == 0 || frame.sample_count == 0 {
        return Err(AudioSyncError::InvalidConfiguration {
            reason: "audio frame rate, channels, and sample count must be positive".to_string(),
        });
    }
    let bytes_per_sample = match frame.sample_format {
        SampleFormat::S16 => 2,
        SampleFormat::F32 => 4,
    };
    let expected = (frame.sample_count as usize)
        .checked_mul(frame.channels as usize)
        .and_then(|samples| samples.checked_mul(bytes_per_sample))
        .ok_or_else(|| AudioSyncError::InvalidConfiguration {
            reason: "audio frame dimensions overflow host size".to_string(),
        })?;
    if frame.data.len() < expected {
        return Err(AudioSyncError::PayloadTruncated {
            expected,
            actual: frame.data.len(),
        });
    }
    Ok(())
}

fn decode_samples(frame: &AudioFrame) -> Result<Vec<f32>> {
    let bytes_per_sample = match frame.sample_format {
        SampleFormat::S16 => 2,
        SampleFormat::F32 => 4,
    };
    let sample_count = (frame.sample_count as usize) * frame.channels as usize;
    let expected = sample_count * bytes_per_sample;
    if frame.data.len() < expected {
        return Err(AudioSyncError::PayloadTruncated {
            expected,
            actual: frame.data.len(),
        });
    }
    let bytes = &frame.data[..expected];
    let mut samples = Vec::with_capacity(sample_count);
    match frame.sample_format {
        SampleFormat::S16 => {
            for &[first, second] in bytes.as_chunks::<2>().0 {
                samples.push(i16::from_le_bytes([first, second]) as f32 / 32_768.0);
            }
        }
        SampleFormat::F32 => {
            for &chunk in bytes.as_chunks::<4>().0 {
                samples.push(f32::from_le_bytes(chunk));
            }
        }
    }
    Ok(samples)
}

fn convert_channels(
    samples: &[f32],
    frames: usize,
    source_channels: u16,
    target_channels: u16,
) -> Vec<f32> {
    if source_channels == target_channels {
        return samples.to_vec();
    }
    let source_channels = source_channels as usize;
    let target_channels = target_channels as usize;
    let mut output = Vec::with_capacity(frames * target_channels);
    for frame in samples.chunks_exact(source_channels).take(frames) {
        if target_channels == 1 {
            let sum: f32 = frame.iter().sum();
            output.push(sum / source_channels as f32);
        } else {
            for channel in 0..target_channels {
                output.push(frame[channel.min(source_channels - 1)]);
            }
        }
    }
    output
}

fn resample_linear(
    samples: &[f32],
    input_frames: usize,
    channels: u16,
    source_rate: u32,
    target_rate: u32,
) -> (Vec<f32>, usize) {
    if source_rate == target_rate || input_frames <= 1 {
        return (samples.to_vec(), input_frames);
    }
    let channels = channels as usize;
    let output_frames = ((input_frames as u64 * u64::from(target_rate)
        + u64::from(source_rate) / 2)
        / u64::from(source_rate))
    .max(1) as usize;
    let mut output = Vec::with_capacity(output_frames * channels);
    for output_index in 0..output_frames {
        let source_position = output_index as f64 * f64::from(source_rate) / f64::from(target_rate);
        let lower = source_position.floor() as usize;
        let upper = (lower + 1).min(input_frames - 1);
        let fraction = (source_position - lower as f64) as f32;
        for channel in 0..channels {
            let first = samples[lower * channels + channel];
            let second = samples[upper * channels + channel];
            output.push(first + (second - first) * fraction);
        }
    }
    (output, output_frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_types::{AudioFrame, SampleFormat, StreamId};

    fn frame(
        sample_format: SampleFormat,
        sample_rate: u32,
        channels: u16,
        pts: i64,
        data: Vec<u8>,
        sample_count: u32,
    ) -> AudioFrame {
        AudioFrame {
            stream_id: StreamId(1),
            sample_format,
            sample_rate,
            channels,
            sample_count,
            pts,
            time_base: TimeBase::from_hz(sample_rate),
            data: data.into(),
        }
    }

    #[test]
    fn converts_s16_mono_to_f32_stereo_and_applies_gain() {
        let mut sync = AudioSynchronizer::new(AudioSyncConfig {
            gain: 0.5,
            ..AudioSyncConfig::default()
        })
        .expect("config");
        let mut data = Vec::new();
        data.extend_from_slice(&i16::MAX.to_le_bytes());
        data.extend_from_slice(&i16::MIN.to_le_bytes());
        let output = sync
            .process(frame(SampleFormat::S16, 48_000, 1, 0, data, 2))
            .expect("converted");
        assert_eq!(output.sample_format, SampleFormat::F32);
        assert_eq!(output.sample_rate, 48_000);
        assert_eq!(output.channels, 2);
        assert_eq!(output.sample_count, 2);
        let samples: Vec<f32> = output
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect();
        assert!((samples[0] - 0.5).abs() < 0.001);
        assert!((samples[1] - 0.5).abs() < 0.001);
        assert!((samples[2] + 0.5).abs() < 0.001);
        assert!((samples[3] + 0.5).abs() < 0.001);
    }

    #[test]
    fn resamples_24khz_to_48khz() {
        let mut sync = AudioSynchronizer::new(AudioSyncConfig::default()).expect("config");
        let data = [0.0_f32, 1.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let output = sync
            .process(frame(SampleFormat::F32, 24_000, 1, 0, data, 2))
            .expect("resampled");
        assert_eq!(output.sample_count, 4);
        assert_eq!(sync.metrics().resampled_chunks, 1);
    }

    #[test]
    fn clamps_small_backward_jitter_and_tracks_drift() {
        let mut sync = AudioSynchronizer::new(AudioSyncConfig::default()).expect("config");
        let data = vec![0_u8; 4 * 2 * 960];
        let first = sync
            .process(frame(SampleFormat::F32, 48_000, 2, 0, data.clone(), 960))
            .expect("first");
        let second = sync
            .process(frame(SampleFormat::F32, 48_000, 2, 960, data, 960))
            .expect("second");
        assert_eq!(first.pts, 0);
        assert_eq!(second.pts, 20);
        assert_eq!(sync.metrics().last_drift_ms, 0);
    }

    #[test]
    fn rejects_large_timestamp_jump_and_truncated_payload() {
        let mut sync = AudioSynchronizer::new(AudioSyncConfig {
            max_discontinuity_ms: 50,
            ..AudioSyncConfig::default()
        })
        .expect("config");
        let data = vec![0_u8; 4 * 2 * 2];
        sync.process(frame(SampleFormat::F32, 48_000, 2, 0, data.clone(), 2))
            .expect("first");
        sync.process(frame(SampleFormat::F32, 48_000, 2, 960, data.clone(), 2))
            .expect("second");
        assert!(matches!(
            sync.process(frame(SampleFormat::F32, 48_000, 2, 48_000, data, 2)),
            Err(AudioSyncError::Discontinuity(_))
        ));

        let mut sync = AudioSynchronizer::new(AudioSyncConfig::default()).expect("config");
        let error = sync.process(frame(SampleFormat::S16, 48_000, 2, 0, vec![0_u8; 3], 1));
        assert!(matches!(
            error,
            Err(AudioSyncError::PayloadTruncated { .. })
        ));
    }

    #[test]
    fn source_restart_keeps_output_timestamps_contiguous() {
        let mut sync = AudioSynchronizer::new(AudioSyncConfig::default()).expect("config");
        let data = vec![0_u8; 4 * 2 * 960];
        let first = sync
            .process(frame(SampleFormat::F32, 48_000, 2, 0, data.clone(), 960))
            .expect("first");
        let second = sync
            .process(frame(SampleFormat::F32, 48_000, 2, 960, data.clone(), 960))
            .expect("second");
        sync.reset_source();
        let recovered = sync
            .process(frame(SampleFormat::F32, 48_000, 2, 0, data, 960))
            .expect("recovered");

        assert_eq!(first.pts, 0);
        assert_eq!(second.pts, 20);
        assert_eq!(recovered.pts, 40);
    }
}
