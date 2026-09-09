use audio_api::{
    AudioCapture, AudioCaptureConfig, AudioCaptureError, AudioCaptureEvent, AudioFormat, Result,
};
use media_types::AudioDeviceInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasapiCaptureKind {
    Loopback,
    Microphone,
}

pub struct WasapiAudioCapture {
    _kind: WasapiCaptureKind,
}

impl WasapiAudioCapture {
    pub fn new(kind: WasapiCaptureKind) -> Self {
        Self { _kind: kind }
    }

    pub fn loopback() -> Self {
        Self::new(WasapiCaptureKind::Loopback)
    }

    pub fn microphone() -> Self {
        Self::new(WasapiCaptureKind::Microphone)
    }
}

pub fn enumerate_devices(_kind: WasapiCaptureKind) -> Result<Vec<AudioDeviceInfo>> {
    Ok(Vec::new())
}

impl AudioCapture for WasapiAudioCapture {
    fn enumerate_devices(&self) -> Result<Vec<AudioDeviceInfo>> {
        enumerate_devices(self._kind)
    }

    fn start(&mut self, _config: AudioCaptureConfig) -> Result<()> {
        Err(AudioCaptureError::Backend {
            details: "WASAPI capture is only available on Windows".to_string(),
        })
    }

    fn next_event(&mut self) -> Result<AudioCaptureEvent> {
        Err(AudioCaptureError::EndOfStream)
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    fn format(&self) -> Option<AudioFormat> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_unavailable_without_claiming_a_format() {
        let mut capture = WasapiAudioCapture::loopback();
        assert!(capture
            .enumerate_devices()
            .expect("portable enumeration")
            .is_empty());
        assert!(matches!(
            capture.start(AudioCaptureConfig::default()),
            Err(AudioCaptureError::Backend { .. })
        ));
        assert!(matches!(
            capture.next_event(),
            Err(AudioCaptureError::EndOfStream)
        ));
        assert!(capture.format().is_none());
    }
}
