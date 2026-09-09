//! Audio-capture abstraction (spec §13.3).
//!
//! Desktop loopback and microphone are separate instances of this trait;
//! a microphone failure must never take down desktop audio (AUD-005).
//! The WASAPI backends land in Segment S4 inside `audio-wasapi`.

use media_types::{AudioDeviceInfo, AudioFrame, SampleFormat};
use thiserror::Error;

pub type Result<T, E = AudioCaptureError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum AudioCaptureError {
    #[error("requested audio device is unavailable")]
    DeviceUnavailable,

    #[error("unsupported audio format: {reason}")]
    FormatUnsupported { reason: String },

    #[error("invalid audio capture configuration: {reason}")]
    InvalidConfiguration { reason: String },

    #[error("the audio device changed while capturing")]
    DeviceChanged,

    #[error("the capture stream ended")]
    EndOfStream,

    #[error("audio capture wait timed out")]
    Timeout,

    #[error("audio capture backend failure: {details}")]
    Backend { details: String },
}

impl AudioCaptureError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::DeviceUnavailable => "AUDIO_DEVICE_UNAVAILABLE",
            Self::FormatUnsupported { .. } => "AUDIO_FORMAT_UNSUPPORTED",
            Self::InvalidConfiguration { .. } => "AUDIO_INVALID_CONFIGURATION",
            Self::DeviceChanged => "AUDIO_DEVICE_CHANGED",
            Self::EndOfStream => "AUDIO_END_OF_STREAM",
            Self::Timeout => "AUDIO_CAPTURE_TIMEOUT",
            Self::Backend { .. } => "AUDIO_BACKEND_FAILURE",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioCaptureConfig {
    /// `None` selects the system default device for the stream kind.
    pub device_id: Option<String>,
}

/// Native format produced by an audio capture endpoint before synchronization
/// converts it to the encoder target format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_format: SampleFormat,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone)]
pub enum AudioCaptureEvent {
    Frames(AudioFrame),
    /// The device changed or disappeared; the engine decides whether to
    /// recover or mark the source unavailable (spec §21.3).
    DeviceLost,
}

/// Original interface shared by loopback and microphone capture.
pub trait AudioCapture: Send {
    fn enumerate_devices(&self) -> Result<Vec<AudioDeviceInfo>>;
    fn start(&mut self, config: AudioCaptureConfig) -> Result<()>;
    fn next_event(&mut self) -> Result<AudioCaptureEvent>;
    fn stop(&mut self) -> Result<()>;

    /// Format discovered during `start`; unavailable before a session starts.
    fn format(&self) -> Option<AudioFormat> {
        None
    }
}
