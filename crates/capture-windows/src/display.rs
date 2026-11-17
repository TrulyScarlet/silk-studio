//! DXGI-only source routing, including opt-in recognized-game discovery.
//! Recovery restarts selection through the engine; GPU contexts are never switched
//! behind a running encoder's back.
use crate::DxgiDisplayCapture;
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
    automatic_game: Option<VideoCaptureConfig>,
    next_game_check: std::time::Instant,
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
            automatic_game: None,
            next_game_check: std::time::Instant::now(),
        }
    }
    pub fn backend_name(&self) -> Option<&'static str> {
        self.backend_name
    }

    fn start_backends(
        &mut self,
        config: VideoCaptureConfig,
        mut preferred: Box<dyn VideoCapture>,
    ) -> Result<()> {
        match preferred.start(config.clone()) {
            Ok(()) => {
                self.active = Some(preferred);
                self.backend_name = Some("dxgi");
                Ok(())
            }
            Err(error) => {
                preferred.stop()?;
                // Product policy: no WGC sessions or yellow-border fallback.
                Err(error)
            }
        }
    }
}

impl VideoCapture for DisplayCapture {
    fn active_source_id(&self) -> Option<String> {
        self.active.as_ref()?.active_source_id()
    }
    fn timestamp_origin(&self) -> Option<std::time::Instant> {
        self.active.as_ref()?.timestamp_origin()
    }
    fn enumerate_sources(&self) -> Result<Vec<VideoSourceInfo>> {
        let mut sources = crate::enumerate_displays()?;
        // Configuration placeholder only: no monitor or pixels are selected
        // until a recognized foreground game supplies its actual FormatChanged.
        sources.push(VideoSourceInfo {
            id: "game:auto".into(),
            name: "Auto-detect game (waiting)".into(),
            is_primary: false,
            width: 1280,
            height: 720,
        });
        for window in crate::enumerate_windows()? {
            let Ok(hwnd) = crate::enumerate::resolve_window(&window.id) else {
                continue;
            };
            let mut bounds = windows::Win32::Foundation::RECT::default();
            if unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut bounds) }
                .is_err()
            {
                continue;
            }
            let width = (bounds.right - bounds.left).max(0) as u32;
            let height = (bounds.bottom - bounds.top).max(0) as u32;
            if width == 0 || height == 0 {
                continue;
            }
            for mode in ["window", "game", "game-guarded"] {
                sources.push(VideoSourceInfo {
                    id: format!("{mode}:{}", window.id),
                    name: if mode == "game-guarded" {
                        format!("{} (Guarded)", window.title)
                    } else {
                        window.title.clone()
                    },
                    is_primary: false,
                    width,
                    height,
                });
            }
        }
        Ok(sources)
    }
    fn start(&mut self, mut config: VideoCaptureConfig) -> Result<()> {
        if self.active.is_some() || self.automatic_game.is_some() {
            return Err(VideoCaptureError::Backend {
                details: "display capture already started".into(),
            });
        }
        if config.source_id == "game:auto" {
            if !matches!(config.target_fps, 30 | 60 | 120) {
                return Err(VideoCaptureError::InvalidConfiguration {
                    reason: "target_fps must be 30, 60, or 120".into(),
                });
            }
            self.automatic_game = Some(config);
            self.next_game_check = std::time::Instant::now();
            return Ok(());
        }
        if let Some(target) = crate::guarded::is_guarded_source(&config.source_id) {
            config.source_id = target.to_owned();
            crate::enumerate::resolve_window(&config.source_id)?;
            let mut capture = crate::GuardedWindowCapture::new(self.stream);
            capture.start(config)?;
            self.active = Some(Box::new(capture));
            self.backend_name = Some("guarded-window");
            return Ok(());
        }
        if let Some(source) = isolated_window_source(&config.source_id) {
            config.source_id = source.to_owned();
            // A window failure must never fall back to monitor capture.
            let hwnd = crate::enumerate::resolve_window(&config.source_id)?;
            crate::enumerate::check_window_capture_affinity(hwnd)?;
            let mut capture = crate::GuardedWindowCapture::new(self.stream);
            capture.start(config)?;
            self.active = Some(Box::new(capture));
            self.backend_name = Some("guarded-window");
            return Ok(());
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
        self.start_backends(config, Box::new(DxgiDisplayCapture::new(self.stream)))?;
        if let Some((monitor, width, height)) = selected {
            self.automatic_monitor = Some(monitor);
            self.initial_format = Some((width, height));
        }
        self.next_monitor_check = std::time::Instant::now();
        Ok(())
    }
    fn next_event(&mut self) -> Result<VideoCaptureEvent> {
        if self.active.is_none() && self.automatic_game.is_some() {
            if std::time::Instant::now() >= self.next_game_check {
                self.next_game_check =
                    std::time::Instant::now() + std::time::Duration::from_millis(500);
                for window in crate::enumerate_windows()? {
                    if !recognized_game(window.app_name.as_deref()) {
                        continue;
                    }
                    let Ok(hwnd) = crate::enumerate::resolve_window(&window.id) else {
                        continue;
                    };
                    if !crate::guarded::is_window_foreground(
                        hwnd,
                        crate::enumerate::parse_window_source(&window.id).unwrap().0,
                    ) {
                        continue;
                    }
                    let mut capture = crate::GuardedWindowCapture::new(self.stream);
                    let mut config = self.automatic_game.clone().unwrap();
                    config.source_id = window.id;
                    capture.start(config)?;
                    self.active = Some(Box::new(capture));
                    self.backend_name = Some("guarded-window");
                    break;
                }
            }
            if self.active.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(25));
                return Ok(VideoCaptureEvent::Idle);
            }
        }
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
        self.automatic_game = None;
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

fn isolated_window_source(source: &str) -> Option<&str> {
    source
        .strip_prefix("window:")
        .or_else(|| source.strip_prefix("game:"))
        .or_else(|| source.starts_with("hwnd:").then_some(source))
        .or_else(|| source.starts_with("pid:").then_some(source))
}

fn recognized_game(executable_stem: Option<&str>) -> bool {
    crate::enumerate::is_recognized_game(executable_stem)
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
    #[test]
    fn automatic_game_wait_is_idle_without_selecting_a_monitor() {
        use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
        let mut capture = super::DisplayCapture::new(media_types::StreamId(0));
        capture
            .start(VideoCaptureConfig {
                source_id: "game:auto".into(),
                target_fps: 60,
            })
            .unwrap();
        // Deterministically exercise waiting without inspecting the live desktop.
        capture.next_game_check = std::time::Instant::now() + std::time::Duration::from_secs(60);
        assert!(matches!(capture.next_event(), Ok(VideoCaptureEvent::Idle)));
        assert!(capture.active.is_none());
        assert!(capture.gpu_frame_context().is_none());
        assert!(capture.automatic_monitor.is_none());
        capture.stop().unwrap();
        assert!(capture.automatic_game.is_none());
    }
    #[test]
    fn automatic_game_recognition_never_defaults_to_desktop_apps() {
        for name in [
            None,
            Some("chrome"),
            Some("Discord"),
            Some("explorer"),
            Some("RiotClientServices"),
        ] {
            assert!(!super::recognized_game(name));
        }
        assert!(super::recognized_game(Some("VALORANT-Win64-Shipping")));
    }
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    #[ignore = "owned visible window fixture; launched by lifecycle regression"]
    fn owned_wgc_window_fixture() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::*;
        let Ok(ready) = std::env::var("SILK_OWNED_WGC_READY") else {
            return;
        };
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Silk owned WGC lifecycle fixture"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                40,
                40,
                640,
                480,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let pid = std::process::id();
        let created = crate::enumerate::process_creation_time(pid).unwrap();
        std::fs::write(
            &ready,
            format!("pid:{pid}:created:{created}:hwnd:{}", window.0 as usize),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline
            && !std::path::Path::new(&format!("{ready}.stop")).exists()
        {
            let mut message = MSG::default();
            unsafe {
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = unsafe { DestroyWindow(window) };
    }

    #[test]
    #[ignore = "requires interactive desktop + WGC; opens a separate owned test window"]
    fn selected_wgc_window_resumes_after_minimize_and_resize_and_closes_safely() {
        use std::time::{Duration, Instant};
        use windows::Win32::UI::WindowsAndMessaging::*;
        let ready = std::env::temp_dir().join(format!(
            "silk-owned-wgc-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct Fixture {
            child: std::process::Child,
            ready: std::path::PathBuf,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = self.child.kill();
                let _ = self.child.wait();
                let _ = std::fs::remove_file(&self.ready);
                let _ = std::fs::remove_file(format!("{}.stop", self.ready.display()));
            }
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "display::tests::owned_wgc_window_fixture",
                "--nocapture",
            ])
            .env("SILK_OWNED_WGC_READY", &ready)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut fixture = Fixture { child, ready };
        let deadline = Instant::now() + Duration::from_secs(5);
        let source = loop {
            if let Ok(source) = std::fs::read_to_string(&fixture.ready) {
                if crate::enumerate::parse_window_source(&source).is_some() {
                    break source;
                }
            }
            assert!(Instant::now() < deadline, "owned window startup timed out");
            assert!(
                fixture.child.try_wait().unwrap().is_none(),
                "fixture exited early"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let window = crate::enumerate::resolve_window(&source).unwrap();
        let mut capture = DisplayCapture::new(StreamId(0));
        capture
            .start(VideoCaptureConfig {
                source_id: format!("game:{source}"),
                target_fps: 60,
            })
            .unwrap();
        assert_eq!(capture.backend_name(), Some("wgc-window"));
        fn frame(capture: &mut DisplayCapture, expected: Option<(u32, u32)>) -> i64 {
            let deadline = Instant::now() + Duration::from_secs(8);
            while Instant::now() < deadline {
                match capture.next_event().unwrap() {
                    VideoCaptureEvent::Frame(frame)
                        if expected.is_none_or(|size| size == (frame.width, frame.height)) =>
                    {
                        return frame.pts
                    }
                    VideoCaptureEvent::SourceLost => panic!("owned window unexpectedly lost"),
                    _ => {}
                }
            }
            panic!("selected window did not deliver expected frame");
        }
        let before = frame(&mut capture, None);
        fn center_pixel(capture: &mut DisplayCapture) -> Vec<u8> {
            let context = capture.gpu_frame_context().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let VideoCaptureEvent::Frame(frame) = capture.next_event().unwrap() {
                    let Ok(lease) = context.resolve(&frame) else {
                        continue;
                    };
                    let slot = lease.downcast_ref::<crate::GpuTextureSlot>().unwrap();
                    // Diagnostic readback of the owned fixture only; production
                    // capture/encoding remains GPU-backed.
                    let pixels = gpu_windows::BgraReadback::default()
                        .read_scaled(slot, frame.width, frame.height)
                        .unwrap();
                    let center = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
                    return pixels[center..center + 3].to_vec();
                }
            }
            panic!("no owned-window pixel evidence");
        }
        let baseline = center_pixel(&mut capture);
        assert!(
            baseline.iter().any(|channel| *channel > 32),
            "fixture center must differ from black occluder"
        );
        struct Occluder(windows::Win32::Foundation::HWND);
        impl Drop for Occluder {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let occluder = Occluder(unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                windows::core::w!("STATIC"),
                windows::core::w!("Silk owned black occluder"),
                WS_POPUP | WS_VISIBLE | WINDOW_STYLE(4), // SS_BLACKRECT
                0,
                0,
                1000,
                800,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        });
        // Discard queued pre-overlap frames before comparing fixture pixels.
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            let _ = capture.next_event().unwrap();
        }
        assert_eq!(
            center_pixel(&mut capture),
            baseline,
            "occluding desktop pixels entered WGC window capture"
        );
        drop(occluder);
        unsafe {
            let _ = ShowWindow(window, SW_MINIMIZE);
        }
        // Drain old publications while minimized; no desktop fallback is allowed.
        let until = Instant::now() + Duration::from_millis(700);
        while Instant::now() < until {
            let _ = capture.next_event().unwrap();
            assert_eq!(capture.backend_name(), Some("wgc-window"));
        }
        unsafe {
            let _ = ShowWindow(window, SW_RESTORE);
            SetWindowPos(
                window,
                None,
                80,
                80,
                800,
                600,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
        }
        // WGC reports the captured window extent (including non-client borders).
        let after = frame(&mut capture, None);
        assert!(after >= before, "resume moved timestamps backwards");
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut resized = false;
        while Instant::now() < deadline {
            if let VideoCaptureEvent::Frame(frame) = capture.next_event().unwrap() {
                assert!(frame.pts >= after);
                if frame.width > 700 && frame.height > 500 {
                    resized = true;
                    break;
                }
            }
        }
        assert!(resized, "WGC did not adopt resized window bounds");
        std::fs::write(format!("{}.stop", fixture.ready.display()), "stop").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut closed = false;
        while Instant::now() < deadline {
            match capture.next_event() {
                Ok(VideoCaptureEvent::SourceLost)
                | Err(VideoCaptureError::SourceUnavailable | VideoCaptureError::EndOfStream) => {
                    closed = true;
                    break;
                }
                Err(error) => panic!("unexpected close error: {error}"),
                _ => {}
            }
        }
        assert!(closed, "closed target did not stop delivery");
        assert_eq!(capture.backend_name(), Some("wgc-window"));
        capture.stop().unwrap();
    }

    #[test]
    #[ignore = "creates an owned Windows HWND; requires interactive desktop"]
    fn owned_capture_exclusion_and_self_target_fail_closed() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
            WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
        };
        struct OwnedWindow(windows::Win32::Foundation::HWND);
        impl Drop for OwnedWindow {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let window = OwnedWindow(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Silk owned exclusion regression"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                320,
                240,
                None,
                None,
                None,
                None,
            )
            .expect("create owned test window")
        });
        unsafe { SetWindowDisplayAffinity(window.0, WDA_EXCLUDEFROMCAPTURE) }
            .expect("exclude owned test window");
        assert!(matches!(
            crate::enumerate::check_window_capture_affinity(window.0),
            Err(VideoCaptureError::PermissionDenied)
        ));
        let pid = std::process::id();
        let created = crate::enumerate::process_creation_time(pid).unwrap();
        for mode in ["game", "window"] {
            let mut capture = DisplayCapture::new(StreamId(0));
            let result = capture.start(VideoCaptureConfig {
                source_id: format!(
                    "{mode}:pid:{pid}:created:{created}:hwnd:{}",
                    window.0 .0 as usize
                ),
                target_fps: 60,
            });
            // Self-capture is rejected even before affinity validation. Do not
            // relax production identity checks just to support this fixture.
            assert!(matches!(result, Err(VideoCaptureError::SourceUnavailable)));
            assert_eq!(capture.backend_name(), None);
            assert!(capture.active.is_none());
            assert!(capture.gpu_frame_context().is_none());
        }
    }
    #[test]
    fn isolated_modes_never_route_to_monitor_selection() {
        for (source, expected) in [
            ("game:auto", Some("auto")),
            ("window:pid:42:hwnd:12", Some("pid:42:hwnd:12")),
            ("hwnd:12", Some("hwnd:12")),
            (
                "pid:42:created:34:hwnd:12",
                Some("pid:42:created:34:hwnd:12"),
            ),
            ("game-guarded:pid:42:hwnd:12", None),
            ("guarded:pid:42:hwnd:12", None),
            ("auto", None),
            (r"\\.\DISPLAY1", None),
        ] {
            assert_eq!(isolated_window_source(source), expected);
        }
    }
    #[test]
    fn invalid_game_targets_fail_closed_without_starting_a_monitor_backend() {
        for source_id in [
            "game:primary",
            "window:default",
            "window:pid:42:hwnd:0",
            "game-guarded:auto",
            "guarded:pid:42:hwnd:0",
        ] {
            let mut capture = DisplayCapture::new(StreamId(0));
            assert!(capture
                .start(VideoCaptureConfig {
                    source_id: source_id.into(),
                    target_fps: 60
                })
                .is_err());
            assert!(capture.active.is_none());
            assert_eq!(capture.backend_name(), None);
        }
    }
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
    fn dxgi_failure_never_falls_back_and_stop_allows_restart() {
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
        assert!(capture
            .start_backends(config.clone(), fake("dxgi", true))
            .is_err());
        assert_eq!(capture.backend_name(), None);
        capture.stop().unwrap();
        capture.start_backends(config, fake("dxgi", false)).unwrap();
        assert_eq!(capture.backend_name(), Some("dxgi"));
        capture.stop().unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            ["dxgi start", "dxgi stop", "dxgi start", "dxgi stop"]
        );
    }
}
