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
    automatic_monitor: Option<isize>,
    next_monitor_check: std::time::Instant,
    initial_format: Option<(u32, u32)>,
    foreground_monitor: fn() -> Option<isize>,
}

impl DisplayCapture {
    pub fn new(stream: StreamId) -> Self {
        Self {
            stream,
            active: None,
            backend_name: None,
            automatic_monitor: None,
            next_monitor_check: std::time::Instant::now(),
            initial_format: None,
            foreground_monitor: crate::enumerate::foreground_monitor,
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
    fn start(&mut self, mut config: VideoCaptureConfig) -> Result<()> {
        if self.active.is_some() {
            return Err(VideoCaptureError::Backend {
                details: "display capture already started".into(),
            });
        }
        let automatic = config.source_id.trim().eq_ignore_ascii_case("auto");
        let selected = if automatic {
            let (output, monitor) = crate::enumerate::resolve_source("auto")?;
            let description =
                unsafe { output.GetDesc() }.map_err(|error| VideoCaptureError::Backend {
                    details: error.to_string(),
                })?;
            config.source_id = String::from_utf16_lossy(&description.DeviceName)
                .trim_end_matches('\0')
                .to_string();
            Some((
                monitor,
                (description.DesktopCoordinates.right - description.DesktopCoordinates.left) as u32,
                (description.DesktopCoordinates.bottom - description.DesktopCoordinates.top) as u32,
            ))
        } else {
            None
        };
        self.start_backends(
            config,
            Box::new(DxgiDisplayCapture::new(self.stream)),
            Box::new(WgcDisplayCapture::new(self.stream)),
        )?;
        if let Some((monitor, width, height)) = selected {
            self.automatic_monitor = Some(monitor);
            self.initial_format = Some((width, height));
        }
        self.next_monitor_check = std::time::Instant::now();
        Ok(())
    }
    fn next_event(&mut self) -> Result<VideoCaptureEvent> {
        if let Some((width, height)) = self.initial_format.take() {
            return Ok(VideoCaptureEvent::FormatChanged { width, height });
        }
        if self.automatic_monitor.is_some() && std::time::Instant::now() >= self.next_monitor_check
        {
            self.next_monitor_check =
                std::time::Instant::now() + std::time::Duration::from_millis(500);
            if monitor_changed(self.automatic_monitor, (self.foreground_monitor)()) {
                diagnostics::info(
                    "capture",
                    "foreground monitor changed; rebuilding Auto capture session",
                );
                return Ok(VideoCaptureEvent::SourceLost);
            }
        }
        self.active
            .as_mut()
            .ok_or(VideoCaptureError::EndOfStream)?
            .next_event()
    }
    fn stop(&mut self) -> Result<()> {
        self.automatic_monitor = None;
        self.initial_format = None;
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

fn monitor_changed(active: Option<isize>, foreground: Option<isize>) -> bool {
    matches!((active, foreground), (Some(active), Some(foreground)) if active != foreground)
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
    #[test]
    fn auto_follows_external_monitor_changes_but_explicit_selection_and_silk_focus_stay_put() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut capture = DisplayCapture::new(StreamId(0));
        capture.active = Some(Box::new(Fake {
            fails: false,
            name: "active",
            calls: calls.clone(),
        }));
        capture.automatic_monitor = Some(1);
        capture.foreground_monitor = || Some(2);
        assert!(matches!(
            capture.next_event(),
            Ok(VideoCaptureEvent::SourceLost)
        ));
        assert!(
            calls.lock().unwrap().is_empty(),
            "source changes must wait for engine-owned recovery"
        );
        capture.next_monitor_check = std::time::Instant::now();
        capture.foreground_monitor = || None;
        assert!(matches!(
            capture.next_event(),
            Err(VideoCaptureError::EndOfStream)
        ));
        capture.automatic_monitor = None;
        capture.foreground_monitor = || Some(2);
        assert!(matches!(
            capture.next_event(),
            Err(VideoCaptureError::EndOfStream)
        ));
    }
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
