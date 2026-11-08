//! Select Desktop Duplication at startup, retaining WGC for unsupported displays.
//! Recovery restarts selection through the engine; GPU contexts are never switched
//! behind a running encoder's back.
use crate::{DxgiDisplayCapture, WgcDisplayCapture};
use capture_api::{Result, VideoCapture, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent};
use encoder_api::GpuFrameContext;
use media_types::{StreamId, VideoSourceInfo};

pub struct DisplayCapture {
    stream: StreamId,
    active: Option<Box<dyn VideoCapture>>,
    backend_name: Option<&'static str>,
}

impl DisplayCapture {
    pub fn new(stream: StreamId) -> Self {
        Self {
            stream,
            active: None,
            backend_name: None,
        }
    }
    pub fn backend_name(&self) -> Option<&'static str> {
        self.backend_name
    }

    fn start_backends(
        &mut self,
        config: VideoCaptureConfig,
        mut preferred: Box<dyn VideoCapture>,
        mut fallback: Box<dyn VideoCapture>,
    ) -> Result<()> {
        match preferred.start(config.clone()) {
            Ok(()) => {
                self.active = Some(preferred);
                self.backend_name = Some("dxgi");
                Ok(())
            }
            Err(error) => {
                preferred.stop()?;
                if matches!(
                    error,
                    VideoCaptureError::InvalidConfiguration { .. }
                        | VideoCaptureError::SourceUnavailable
                ) {
                    return Err(error);
                }
                diagnostics::warn("capture", &format!("Desktop Duplication unavailable ({error}); falling back to Windows Graphics Capture, which may show the capture border"));
                fallback.start(config)?;
                self.active = Some(fallback);
                self.backend_name = Some("wgc");
                Ok(())
            }
        }
    }
}

impl VideoCapture for DisplayCapture {
    fn enumerate_sources(&self) -> Result<Vec<VideoSourceInfo>> {
        crate::enumerate_displays()
    }
    fn start(&mut self, config: VideoCaptureConfig) -> Result<()> {
        if self.active.is_some() {
            return Err(VideoCaptureError::Backend {
                details: "display capture already started".into(),
            });
        }
        self.start_backends(
            config,
            Box::new(DxgiDisplayCapture::new(self.stream)),
            Box::new(WgcDisplayCapture::new(self.stream)),
        )
    }
    fn next_event(&mut self) -> Result<VideoCaptureEvent> {
        self.active
            .as_mut()
            .ok_or(VideoCaptureError::EndOfStream)?
            .next_event()
    }
    fn stop(&mut self) -> Result<()> {
        self.backend_name = None;
        if let Some(mut active) = self.active.take() {
            active.stop()?;
        }
        Ok(())
    }
    fn gpu_frame_context(&self) -> Option<GpuFrameContext> {
        self.active.as_ref()?.gpu_frame_context()
    }
}
impl Drop for DisplayCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Fake {
        fails: bool,
        name: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
    }
    impl VideoCapture for Fake {
        fn enumerate_sources(&self) -> Result<Vec<VideoSourceInfo>> {
            Ok(vec![])
        }
        fn start(&mut self, _: VideoCaptureConfig) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{} start", self.name));
            if self.fails {
                Err(VideoCaptureError::Backend {
                    details: "unsupported display".into(),
                })
            } else {
                Ok(())
            }
        }
        fn stop(&mut self) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{} stop", self.name));
            Ok(())
        }
        fn next_event(&mut self) -> Result<VideoCaptureEvent> {
            Err(VideoCaptureError::EndOfStream)
        }
    }
    #[test]
    fn fallback_releases_failed_backend_and_stop_allows_restart() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let fake = |name, fails| {
            Box::new(Fake {
                name,
                fails,
                calls: calls.clone(),
            }) as Box<dyn VideoCapture>
        };
        let config = VideoCaptureConfig {
            source_id: "default".into(),
            target_fps: 60,
        };
        let mut capture = DisplayCapture::new(StreamId(0));
        capture
            .start_backends(config.clone(), fake("dxgi", true), fake("wgc", false))
            .unwrap();
        assert_eq!(capture.backend_name(), Some("wgc"));
        capture.stop().unwrap();
        capture
            .start_backends(config, fake("dxgi", false), fake("wgc", false))
            .unwrap();
        assert_eq!(capture.backend_name(), Some("dxgi"));
        capture.stop().unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            [
                "dxgi start",
                "dxgi stop",
                "wgc start",
                "wgc stop",
                "dxgi start",
                "dxgi stop"
            ]
        );
    }
}
