//! Scripted capture sources driven by explicit step lists.

use std::collections::VecDeque;
use std::time::Duration;

use audio_api::{
    AudioCapture, AudioCaptureConfig, AudioCaptureError, AudioCaptureEvent, Result as AudioResult,
};
use capture_api::{
    Result as VideoResult, VideoCapture, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent,
};
use media_types::{
    AudioDeviceInfo, AudioDeviceKind, AudioFrame, FramePayload, GpuFrameHandle, PixelFormat,
    SampleFormat, StreamId, TimeBase, VideoFrame, VideoSourceInfo,
};

const MOCK_STREAM: StreamId = StreamId(0);

/// Steps replayed in order by [`ScriptedVideoCapture`].
#[derive(Debug, Clone)]
pub enum VideoStep {
    /// Produce `n` frames at the configured cadence.
    Frames(u32),
    /// Emit one format-changed event and switch surface size.
    FormatChange { width: u32, height: u32 },
    /// Emit a source-lost event (then end of stream).
    SourceLost,
}

/// Deterministic display-capture stand-in.
pub struct ScriptedVideoCapture {
    pending: VecDeque<VideoStep>,
    active_frames_remaining: Option<u32>,
    lost_pending: bool,
    pts_ms: i64,
    interval_ms: i64,
    width: u32,
    height: u32,
    counter: u64,
    start_count: usize,
    started: bool,
    cpu_frames: bool,
    jitter_seed: u64,
    jitter_max_ms: i64,
    continuous: bool,
    frame_delay: Duration,
    recovery_failures_remaining: usize,
    gpu_context: Option<encoder_api::GpuFrameContext>,
}

impl ScriptedVideoCapture {
    pub fn new(steps: Vec<VideoStep>) -> Self {
        Self {
            pending: steps.into(),
            active_frames_remaining: None,
            lost_pending: false,
            pts_ms: 0,
            interval_ms: 33,
            width: 1920,
            height: 1080,
            counter: 0,
            start_count: 0,
            started: false,
            cpu_frames: false,
            jitter_seed: 0,
            jitter_max_ms: 0,
            continuous: false,
            frame_delay: Duration::ZERO,
            recovery_failures_remaining: 0,
            gpu_context: None,
        }
    }

    pub fn with_gpu_context(mut self, context: encoder_api::GpuFrameContext) -> Self {
        self.gpu_context = Some(context);
        self
    }

    pub fn with_interval_ms(mut self, interval_ms: i64) -> Self {
        self.interval_ms = interval_ms;
        self
    }

    /// Produce CPU-backed BGRA frames for exercising a CPU-capable encoder.
    /// The default remains GPU handles so capture/encoder bridge tests retain
    /// the native-shaped payload contract.
    pub fn with_cpu_frames(mut self) -> Self {
        self.cpu_frames = true;
        self
    }

    /// Set the dimensions reported by subsequent frames. This keeps native
    /// encoder smoke tests small while preserving the configured shape.
    pub fn with_dimensions(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Deterministic backwards PTS jitter (capture-style display stamps);
    /// seed fixes the exact sequence so failures reproduce (spec §27.3).
    pub fn with_jitter(mut self, seed: u64, max_ms: i64) -> Self {
        self.jitter_seed = seed | 1;
        self.jitter_max_ms = max_ms;
        self
    }

    /// Continue producing frames after the scripted steps are exhausted until
    /// the engine stops the capture. This keeps a synthetic session active
    /// while save and resume tests exercise concurrent work.
    pub fn with_continuous_frames(mut self) -> Self {
        self.continuous = true;
        self
    }

    /// Add a small deterministic delay to each delivered frame so tests do not
    /// turn an overlap scenario into a CPU-bound tight loop.
    pub fn with_frame_delay_ms(mut self, delay_ms: u64) -> Self {
        self.frame_delay = Duration::from_millis(delay_ms);
        self
    }

    /// Fail this many `start` calls after the initial start. The failures are
    /// used to exercise bounded display reacquisition without hardware.
    pub fn with_recovery_failures(mut self, failures: usize) -> Self {
        self.recovery_failures_remaining = failures;
        self
    }

    fn next_jitter(&mut self) -> i64 {
        if self.jitter_max_ms <= 0 {
            return 0;
        }
        let mut x = self.jitter_seed;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.jitter_seed = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D)) >> 33) as i64 % (self.jitter_max_ms + 1)
    }

    fn next_frame(&mut self) -> VideoFrame {
        self.counter += 1;
        let pts = (self.pts_ms - self.next_jitter()).max(0);
        self.pts_ms += self.interval_ms;
        let payload = if self.cpu_frames {
            let pixel_count = (self.width as usize) * (self.height as usize);
            let mut bytes = vec![0_u8; pixel_count * 4];
            let blue = (self.counter % 251) as u8;
            for pixel in bytes.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[blue, 96, 160, 255]);
            }
            FramePayload::Cpu(bytes.into())
        } else {
            FramePayload::Gpu(GpuFrameHandle(self.counter))
        };
        VideoFrame {
            stream_id: MOCK_STREAM,
            width: self.width,
            height: self.height,
            pixel_format: PixelFormat::Bgra8,
            pts,
            time_base: TimeBase::MILLISECOND,
            payload,
        }
    }
}

impl VideoCapture for ScriptedVideoCapture {
    fn enumerate_sources(&self) -> VideoResult<Vec<VideoSourceInfo>> {
        Ok(vec![VideoSourceInfo {
            id: "display-1".to_string(),
            name: "Mock Display 1".to_string(),
            is_primary: true,
            width: self.width,
            height: self.height,
        }])
    }

    fn start(&mut self, config: VideoCaptureConfig) -> VideoResult<()> {
        if !matches!(config.target_fps, 30 | 60 | 120) {
            return Err(VideoCaptureError::InvalidConfiguration {
                reason: format!(
                    "target_fps must be 30, 60, or 120, got {}",
                    config.target_fps
                ),
            });
        }
        if self.started {
            return Err(VideoCaptureError::Backend {
                details: "capture already started".to_string(),
            });
        }
        self.start_count += 1;
        if self.start_count > 1 && self.recovery_failures_remaining > 0 {
            self.recovery_failures_remaining -= 1;
            return Err(VideoCaptureError::SourceUnavailable);
        }
        self.started = true;
        Ok(())
    }

    fn next_event(&mut self) -> VideoResult<VideoCaptureEvent> {
        if !self.started {
            return Err(VideoCaptureError::Backend {
                details: "capture not started".to_string(),
            });
        }

        loop {
            if self.lost_pending {
                self.lost_pending = false;
                self.started = false;
                return Ok(VideoCaptureEvent::SourceLost);
            }

            if let Some(remaining) = self.active_frames_remaining.as_mut() {
                if *remaining > 0 {
                    *remaining -= 1;
                    std::thread::sleep(self.frame_delay);
                    return Ok(VideoCaptureEvent::Frame(self.next_frame()));
                }
                self.active_frames_remaining = None;
            }

            match self.pending.pop_front() {
                None if self.continuous => {
                    std::thread::sleep(self.frame_delay);
                    return Ok(VideoCaptureEvent::Frame(self.next_frame()));
                }
                None => {
                    self.started = false;
                    return Err(VideoCaptureError::EndOfStream);
                }
                Some(VideoStep::Frames(n)) => {
                    self.active_frames_remaining = Some(n);
                }
                Some(VideoStep::FormatChange { width, height }) => {
                    self.width = width;
                    self.height = height;
                    return Ok(VideoCaptureEvent::FormatChanged { width, height });
                }
                Some(VideoStep::SourceLost) => {
                    self.lost_pending = true;
                }
            }
        }
    }

    fn stop(&mut self) -> VideoResult<()> {
        self.started = false;
        Ok(())
    }

    fn gpu_frame_context(&self) -> Option<encoder_api::GpuFrameContext> {
        self.gpu_context.clone()
    }
}

/// Steps replayed in order by [`ScriptedAudioCapture`].
#[derive(Debug, Clone)]
pub enum AudioStep {
    Chunks(u32),
    DeviceLost,
}

/// Deterministic loopback/microphone stand-in producing 20 ms f32 chunks.
pub struct ScriptedAudioCapture {
    pending: VecDeque<AudioStep>,
    active_chunks_remaining: Option<u32>,
    lost_pending: bool,
    sample_rate: u32,
    channels: u16,
    chunk_samples: u32,
    pts_samples: i64,
    counter: u64,
    start_count: usize,
    started: bool,
    continuous: bool,
    recovery_failures_remaining: usize,
}

impl ScriptedAudioCapture {
    pub fn new(steps: Vec<AudioStep>) -> Self {
        Self {
            pending: steps.into(),
            active_chunks_remaining: None,
            lost_pending: false,
            sample_rate: 48_000,
            channels: 2,
            chunk_samples: 960,
            pts_samples: 0,
            counter: 0,
            start_count: 0,
            started: false,
            continuous: false,
            recovery_failures_remaining: 0,
        }
    }

    pub fn with_continuous_chunks(mut self) -> Self {
        self.continuous = true;
        self
    }

    /// Fail this many `start` calls after the initial start for bounded audio
    /// recovery tests.
    pub fn with_recovery_failures(mut self, failures: usize) -> Self {
        self.recovery_failures_remaining = failures;
        self
    }

    fn next_chunk(&mut self) -> AudioFrame {
        let interleaved = (self.chunk_samples * u32::from(self.channels)) as usize;
        let mut samples = Vec::<u8>::with_capacity(interleaved * 4);
        let byte_pattern = (self.counter % 251) as u8;
        for _ in 0..interleaved * 4 {
            samples.push(byte_pattern);
        }
        self.counter += 1;
        let pts = self.pts_samples;
        self.pts_samples += i64::from(self.chunk_samples);
        AudioFrame {
            stream_id: MOCK_STREAM,
            sample_format: SampleFormat::F32,
            sample_rate: self.sample_rate,
            channels: self.channels,
            sample_count: self.chunk_samples,
            pts,
            time_base: TimeBase::from_hz(self.sample_rate),
            data: samples.into(),
        }
    }
}

impl AudioCapture for ScriptedAudioCapture {
    fn enumerate_devices(&self) -> AudioResult<Vec<AudioDeviceInfo>> {
        Ok(vec![
            AudioDeviceInfo {
                id: "mock-output-default".to_string(),
                name: "Mock Speakers".to_string(),
                kind: AudioDeviceKind::Output,
                is_default: true,
            },
            AudioDeviceInfo {
                id: "mock-input-1".to_string(),
                name: "Mock Microphone".to_string(),
                kind: AudioDeviceKind::Input,
                is_default: false,
            },
        ])
    }

    fn start(&mut self, _config: AudioCaptureConfig) -> AudioResult<()> {
        if self.started {
            return Err(AudioCaptureError::InvalidConfiguration {
                reason: "audio capture already started".to_string(),
            });
        }
        self.start_count += 1;
        if self.start_count > 1 && self.recovery_failures_remaining > 0 {
            self.recovery_failures_remaining -= 1;
            return Err(AudioCaptureError::DeviceUnavailable);
        }
        self.started = true;
        Ok(())
    }

    fn next_event(&mut self) -> AudioResult<AudioCaptureEvent> {
        if !self.started {
            return Err(AudioCaptureError::Backend {
                details: "capture not started".to_string(),
            });
        }

        loop {
            if self.lost_pending {
                self.lost_pending = false;
                self.started = false;
                return Ok(AudioCaptureEvent::DeviceLost);
            }

            if let Some(remaining) = self.active_chunks_remaining.as_mut() {
                if *remaining > 0 {
                    *remaining -= 1;
                    return Ok(AudioCaptureEvent::Frames(self.next_chunk()));
                }
                self.active_chunks_remaining = None;
            }

            match self.pending.pop_front() {
                None if self.continuous => {
                    return Ok(AudioCaptureEvent::Frames(self.next_chunk()));
                }
                None => {
                    self.started = false;
                    return Err(AudioCaptureError::EndOfStream);
                }
                Some(AudioStep::Chunks(n)) => {
                    self.active_chunks_remaining = Some(n);
                }
                Some(AudioStep::DeviceLost) => {
                    self.lost_pending = true;
                }
            }
        }
    }

    fn stop(&mut self) -> AudioResult<()> {
        self.started = false;
        Ok(())
    }
}
