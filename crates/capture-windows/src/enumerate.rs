//! DXGI display enumeration → `VideoSourceInfo` (spec VID-001).
//!
//! Identity note: DXGI output names (`\\.\DISPLAY1`) are stable across
//! reboots for a fixed topology but can be renumbered when monitors are
//! replugged. EDID-hash-based identity lands later if it proves necessary.

use windows::core::HRESULT;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, DXGI_OUTPUT_DESC,
};

use capture_api::{Result as CaptureResult, VideoCaptureError};
use media_types::VideoSourceInfo;

const DXGI_ERROR_NOT_FOUND: HRESULT = HRESULT(0x887A0002u32 as i32);

/// Authoritative check for games recognized across the application.
pub fn is_recognized_game(executable_stem: Option<&str>) -> bool {
    executable_stem.is_some_and(|name| {
        let lower = name.to_ascii_lowercase();
        let stem = lower.trim_end_matches(".exe");
        matches!(
            stem,
            "valorant-win64-shipping"
                | "valorant"
                | "cs2"
                | "csgo"
                | "r5apex"
                | "r5apex_dx12"
                | "fortniteclient-win64-shipping"
                | "fortniteclient"
                | "fortnitelauncher"
                | "overwatch"
                | "overwatch2"
                | "rocketleague"
                | "terraria"
                | "helldivers2"
                | "leagueclient"
                | "leagueclientux"
                | "league of legends"
                | "destiny2"
                | "rainbowsix"
                | "rainbowsix_vulkan"
                | "deadlock"
                | "gta5"
                | "playgtav"
                | "robloxplayerbeta"
                | "robloxplayer"
                | "genshinimpact"
                | "starrail"
                | "zenlesszonezero"
                | "marvelrivals-win64-shipping"
                | "marvelrivals"
                | "tslgame"
                | "discovery"
                | "discovery-d"
                | "discovery-shipping"
                | "discovery_shipping"
        )
    })
}

fn window_target_and_fullscreen(
    hwnd: windows::Win32::Foundation::HWND,
    window: media_types::WindowSourceInfo,
) -> Option<(media_types::WindowSourceInfo, bool)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic};
    let mut bounds = RECT::default();
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let fullscreen = unsafe {
        !IsIconic(hwnd).as_bool()
            && GetWindowRect(hwnd, &mut bounds).is_ok()
            && GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL), &mut monitor)
                .as_bool()
            && (bounds.left - monitor.rcMonitor.left).abs() <= 8
            && (bounds.top - monitor.rcMonitor.top).abs() <= 8
            && (bounds.right - monitor.rcMonitor.right).abs() <= 8
            && (bounds.bottom - monitor.rcMonitor.bottom).abs() <= 8
    };
    Some((window, fullscreen))
}

/// Top visible application covering the captured display's center, in Z order.
/// This is display attribution, not a promise of isolated process capture.
pub fn capture_target_window(source_id: &str) -> Option<(media_types::WindowSourceInfo, bool)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic};

    if source_id == "game:auto" || source_id == "auto" {
        if let Ok(windows) = enumerate_windows() {
            // First preference: any recognized game currently in the foreground
            for window in &windows {
                if !is_recognized_game(window.app_name.as_deref()) {
                    continue;
                }
                if let Ok(hwnd) = resolve_window(&window.id) {
                    if let Some((pid, _, _)) = parse_window_source(&window.id) {
                        if crate::guarded::is_window_foreground(hwnd, pid) {
                            return window_target_and_fullscreen(hwnd, window.clone());
                        }
                    }
                }
            }
            // Second preference: any active running recognized game
            for window in &windows {
                if !is_recognized_game(window.app_name.as_deref()) {
                    continue;
                }
                if let Ok(hwnd) = resolve_window(&window.id) {
                    return window_target_and_fullscreen(hwnd, window.clone());
                }
            }
        }
        return None;
    }

    let target_opt = source_id
        .strip_prefix("game:")
        .or_else(|| source_id.strip_prefix("game-guarded:"))
        .or_else(|| source_id.strip_prefix("window:"))
        .or_else(|| {
            (source_id.starts_with("pid:") || source_id.starts_with("hwnd:")).then_some(source_id)
        });

    if let Some(target) = target_opt {
        let hwnd = resolve_window(target).ok()?;
        let window = enumerate_windows()
            .ok()?
            .into_iter()
            .find(|window| resolve_window(&window.id).ok() == Some(hwnd))?;
        return window_target_and_fullscreen(hwnd, window);
    }

    let (output, _) = resolve_source(source_id).ok()?;
    let display = unsafe { output.GetDesc().ok()? }.DesktopCoordinates;
    let center = (
        (display.left + display.right) / 2,
        (display.top + display.bottom) / 2,
    );
    for window in enumerate_windows().ok()? {
        let hwnd = resolve_window(&window.id).ok()?;
        let mut bounds = RECT::default();
        if unsafe { IsIconic(hwnd).as_bool() || GetWindowRect(hwnd, &mut bounds).is_err() } {
            continue;
        }
        if bounds.left <= center.0
            && bounds.right > center.0
            && bounds.top <= center.1
            && bounds.bottom > center.1
        {
            return window_target_and_fullscreen(hwnd, window);
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsAdapterInfo {
    pub name: String,
    pub vendor_id: u32,
}

pub(crate) fn factory() -> CaptureResult<IDXGIFactory1> {
    unsafe {
        CreateDXGIFactory1::<IDXGIFactory1>().map_err(|err| VideoCaptureError::Backend {
            details: format!("CreateDXGIFactory1 failed: {err}"),
        })
    }
}

pub(crate) fn adapter_outputs(adapter: &IDXGIAdapter1) -> CaptureResult<Vec<IDXGIOutput>> {
    let mut outputs = Vec::new();
    let mut index = 0_u32;
    loop {
        match unsafe { adapter.EnumOutputs(index) } {
            Ok(output) => {
                outputs.push(output);
                index += 1;
            }
            Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(err) => {
                return Err(VideoCaptureError::Backend {
                    details: format!("EnumOutputs({index}) failed: {err}"),
                })
            }
        }
    }
    Ok(outputs)
}

pub(crate) fn describe(output: &IDXGIOutput) -> Option<DXGI_OUTPUT_DESC> {
    let desc = unsafe { output.GetDesc() }.ok()?;
    if !desc.AttachedToDesktop.as_bool() {
        None
    } else {
        Some(desc)
    }
}

/// Enumerate attached desktop displays in DXGI order (primary first).
pub fn enumerate_displays() -> CaptureResult<Vec<VideoSourceInfo>> {
    let factory = factory()?;
    let mut sources = Vec::new();
    let mut adapter_index = 0_u32;
    let mut display_counter = 1_u32;
    loop {
        let adapter: Option<IDXGIAdapter1> = unsafe { factory.EnumAdapters1(adapter_index) }.ok();
        let Some(adapter) = adapter else { break };
        for output in adapter_outputs(&adapter)? {
            let Some(desc) = describe(&output) else {
                continue;
            };
            let id = String::from_utf16_lossy(
                &desc
                    .DeviceName
                    .iter()
                    .take_while(|c| **c != 0)
                    .copied()
                    .collect::<Vec<_>>(),
            );
            // Primary display: desktop origin (0,0) inside its bounds.
            let is_primary = desc.DesktopCoordinates.left == 0 && desc.DesktopCoordinates.top == 0;
            let width =
                (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left).unsigned_abs();
            let height =
                (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top).unsigned_abs();
            let display_num = id
                .strip_prefix(r"\\.\DISPLAY")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(display_counter);
            display_counter += 1;
            let name = if is_primary {
                format!("Display {display_num} (Primary - {width}x{height})")
            } else {
                format!("Display {display_num} ({width}x{height})")
            };
            sources.push(VideoSourceInfo {
                id,
                name,
                is_primary,
                width,
                height,
            });
        }
        adapter_index += 1;
    }

    if sources.is_empty() {
        return Err(VideoCaptureError::SourceUnavailable);
    }

    sources.sort_by_key(|s| !s.is_primary);
    Ok(sources)
}

/// Enumerate top-level application windows for window capture.
#[cfg(windows)]
pub fn enumerate_windows() -> CaptureResult<Vec<media_types::WindowSourceInfo>> {
    use std::path::PathBuf;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE, GWL_STYLE, WS_CHILD,
        WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };

    let mut windows_list = Vec::new();

    unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let windows_list = &mut *(lparam.0 as *mut Vec<media_types::WindowSourceInfo>);

        if !IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }

        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if style & WS_CHILD.0 != 0 {
            return BOOL(1);
        }

        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 && ex_style & WS_EX_APPWINDOW.0 == 0 {
            return BOOL(1);
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width <= 0 || height <= 0 {
                return BOOL(1);
            }
        }

        // A cloaked/minimized game can still be a useful process-bound target.
        // List it without promising that WGC will supply frames while minimized.

        let title_len = GetWindowTextLengthW(hwnd);
        if title_len <= 0 {
            return BOOL(1);
        }

        let mut title_buf = vec![0_u16; (title_len + 1) as usize];
        let len = GetWindowTextW(hwnd, &mut title_buf);
        if len <= 0 {
            return BOOL(1);
        }

        let title = String::from_utf16_lossy(&title_buf[..len as usize])
            .trim()
            .to_string();
        if title.is_empty() {
            return BOOL(1);
        }

        if title == "Program Manager" || title == "Windows Shell Experience Host" {
            return BOOL(1);
        }

        let mut process_id = 0_u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == 0 || process_id == std::process::id() {
            return BOOL(1);
        }
        let Ok(created) = process_creation_time(process_id) else {
            return BOOL(1);
        };
        let mut app_name = None;
        if process_id != 0 {
            if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) {
                let mut path_buf = vec![0_u16; 1024];
                let mut path_len = path_buf.len() as u32;
                if QueryFullProcessImageNameW(
                    process,
                    PROCESS_NAME_FORMAT(0),
                    windows::core::PWSTR(path_buf.as_mut_ptr()),
                    &mut path_len,
                )
                .is_ok()
                    && path_len > 0
                {
                    let path =
                        PathBuf::from(String::from_utf16_lossy(&path_buf[..path_len as usize]));
                    if let Some(stem) = path.file_stem() {
                        let name = stem.to_string_lossy().trim().to_string();
                        if !name.is_empty() {
                            app_name = Some(name);
                        }
                    }
                }
                let _ = windows::Win32::Foundation::CloseHandle(process);
            }
        }

        if is_picker_system_utility(app_name.as_deref()) {
            return BOOL(1);
        }

        let id = format!(
            "pid:{process_id}:created:{created}:hwnd:{}",
            hwnd.0 as usize
        );
        windows_list.push(media_types::WindowSourceInfo {
            id,
            title,
            app_name,
        });

        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut windows_list as *mut _ as isize),
        );
    }

    Ok(windows_list)
}

/// Picker hygiene only, not a security boundary or a game classifier. Use exact
/// executable stems rather than localized titles or broad "Host" matching.
fn is_picker_system_utility(executable_stem: Option<&str>) -> bool {
    executable_stem.is_some_and(|name| {
        [
            "TextInputHost",
            "InputApp",
            "ShellExperienceHost",
            "StartMenuExperienceHost",
            "SearchHost",
            "SearchApp",
            "LockApp",
        ]
        .iter()
        .any(|utility| name.eq_ignore_ascii_case(utility))
    })
}

/// Resolve a selected window, rejecting stale handles reassigned to another process.
pub(crate) fn resolve_window(source: &str) -> CaptureResult<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
    let (expected_pid, expected_creation, handle) = parse_window_source(source).ok_or_else(|| {
        VideoCaptureError::InvalidConfiguration {
            reason: "Reselect the running game/application window to bind its process lifetime; legacy HWND/PID-only selections and automatic monitor capture are not permitted in game/window mode".into(),
        }
    })?;
    let hwnd = HWND(handle as *mut _);
    let mut pid = 0;
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Err(VideoCaptureError::SourceUnavailable);
        }
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0
        || pid == std::process::id()
        || expected_pid != pid
        || process_creation_time(pid)? != expected_creation
    {
        return Err(VideoCaptureError::SourceUnavailable);
    }
    Ok(hwnd)
}

pub(crate) fn parse_window_source(source: &str) -> Option<(u32, u64, usize)> {
    if source.len() > 96 {
        return None;
    }
    let rest = source.strip_prefix("pid:")?;
    let (pid, rest) = rest.split_once(":created:")?;
    let (created, handle) = rest.split_once(":hwnd:")?;
    if [pid, created, handle]
        .iter()
        .any(|field| field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let pid = pid.parse::<u32>().ok().filter(|pid| *pid != 0)?;
    let created = created
        .parse::<u64>()
        .ok()
        .filter(|created| *created != 0)?;
    let handle = handle.parse::<usize>().ok().filter(|handle| *handle != 0)?;
    Some((pid, created, handle))
}

pub(crate) fn process_creation_time(pid: u32) -> CaptureResult<u64> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::Win32::Foundation::{FILETIME, HANDLE};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|_| VideoCaptureError::SourceUnavailable)?;
    let process = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetProcessTimes(
            HANDLE(process.as_raw_handle()),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    }
    .map_err(|_| VideoCaptureError::SourceUnavailable)?;
    let created = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    if created == 0 {
        return Err(VideoCaptureError::SourceUnavailable);
    }
    Ok(created)
}

pub(crate) fn capture_affinity_denied(affinity: Option<u32>) -> bool {
    matches!(affinity, Some(1 | 17))
}

/// Honor the application's existing OS exclusion. Query failure is not permission
/// to bypass it: ordinary WGC still decides support, with no monitor fallback.
pub(crate) fn check_window_capture_affinity(
    hwnd: windows::Win32::Foundation::HWND,
) -> CaptureResult<()> {
    let mut affinity = 0;
    let observed = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowDisplayAffinity(hwnd, &mut affinity)
    }
    .ok()
    .map(|()| affinity);
    if capture_affinity_denied(observed) {
        diagnostics::warn("wgc",&format!("selected window excludes OS capture via display affinity ({affinity:#x}); no capture or monitor fallback"));
        return Err(VideoCaptureError::PermissionDenied);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn enumerate_windows() -> CaptureResult<Vec<media_types::WindowSourceInfo>> {
    Ok(Vec::new())
}

/// Enumerate DXGI adapters for diagnostics. This reports the adapter name,
/// vendor id, and driver version without exposing user paths or display
/// topology identifiers.
pub fn enumerate_adapters() -> CaptureResult<Vec<GraphicsAdapterInfo>> {
    let factory = factory()?;
    let mut adapters = Vec::new();
    let mut adapter_index = 0_u32;
    loop {
        let adapter: Option<IDXGIAdapter1> = unsafe { factory.EnumAdapters1(adapter_index) }.ok();
        let Some(adapter) = adapter else { break };
        let desc = unsafe {
            adapter
                .GetDesc1()
                .map_err(|error| VideoCaptureError::Backend {
                    details: format!("GetDesc1({adapter_index}) failed: {error}"),
                })?
        };
        let name = String::from_utf16_lossy(
            &desc
                .Description
                .iter()
                .take_while(|c| **c != 0)
                .copied()
                .collect::<Vec<_>>(),
        );
        adapters.push(GraphicsAdapterInfo {
            name,
            vendor_id: desc.VendorId,
        });
        adapter_index += 1;
    }
    Ok(adapters)
}

/// Resolve a source id (`""`, `"default"`, `"display-1"` → primary) to its
/// DXGI output plus raw HMONITOR handle.
pub(crate) fn resolve_source(source_id: &str) -> CaptureResult<(IDXGIOutput, isize)> {
    let factory = factory()?;
    let automatic_monitor = source_id
        .trim()
        .eq_ignore_ascii_case("auto")
        .then(foreground_monitor)
        .flatten();
    let mut fallback: Option<(IDXGIOutput, isize)> = None;
    let mut first_output: Option<(IDXGIOutput, isize)> = None;
    let mut adapter_index = 0_u32;
    loop {
        let adapter: Option<IDXGIAdapter1> = unsafe { factory.EnumAdapters1(adapter_index) }.ok();
        let Some(adapter) = adapter else { break };
        for output in adapter_outputs(&adapter)? {
            let Some(desc) = describe(&output) else {
                continue;
            };
            let name = String::from_utf16_lossy(
                &desc
                    .DeviceName
                    .iter()
                    .take_while(|c| **c != 0)
                    .copied()
                    .collect::<Vec<_>>(),
            );
            let handle = desc.Monitor.0 as isize;
            if automatic_monitor == Some(handle) {
                return Ok((output, handle));
            }
            if first_output.is_none() {
                first_output = Some((output.clone(), handle));
            }
            let norm_name = name.trim().to_ascii_lowercase();
            let norm_source = source_id.trim().to_ascii_lowercase();
            if norm_name == norm_source
                || norm_name.trim_start_matches(r"\\.\") == norm_source.trim_start_matches(r"\\.\")
            {
                return Ok((output, handle));
            }
            let is_primary = desc.DesktopCoordinates.left == 0 && desc.DesktopCoordinates.top == 0;
            if fallback.is_none() && is_primary {
                fallback = Some((output, handle));
            }
        }
        adapter_index += 1;
    }

    let norm_source = source_id.trim().to_ascii_lowercase();
    if matches!(
        norm_source.as_str(),
        "" | "default" | "display-1" | "primary" | "auto" | "main"
    ) {
        if let Some(pair) = fallback.or(first_output) {
            return Ok(pair);
        }
    }
    Err(VideoCaptureError::SourceUnavailable)
}

/// Ignore Silk itself so opening its settings does not move an Auto capture.
pub(crate) fn foreground_monitor() -> Option<isize> {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONULL};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsIconic,
    };
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() || IsIconic(window).as_bool() {
            return None;
        }
        let mut process = 0;
        GetWindowThreadProcessId(window, Some(&mut process));
        if process == 0 || process == std::process::id() {
            return None;
        }
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONULL);
        (!monitor.0.is_null()).then_some(monitor.0 as isize)
    }
}

#[cfg(test)]
mod window_identity_tests {
    use super::{capture_affinity_denied, is_picker_system_utility, parse_window_source};
    #[test]
    fn window_picker_hides_known_windows_input_and_shell_helpers() {
        for name in [
            "TextInputHost",
            "textinputhost",
            "TEXTINPUTHOST",
            "InputApp",
            "ShellExperienceHost",
            "StartMenuExperienceHost",
            "SearchHost",
            "SearchApp",
            "LockApp",
        ] {
            assert!(is_picker_system_utility(Some(name)), "{name}");
        }
    }

    #[test]
    fn window_picker_keeps_games_apps_and_generic_application_hosts() {
        for name in [
            None,
            Some(""),
            Some("VALORANT-Win64-Shipping"),
            Some("cs2"),
            Some("unknown-game"),
            Some("ApplicationFrameHost"),
            Some("explorer"),
            Some("ScreenSketch"),
            Some("notepad"),
            Some("Discord"),
            Some("brave"),
            Some("TextInputHostGame"),
            Some("MySearchApp"),
        ] {
            assert!(!is_picker_system_utility(name), "{name:?}");
        }
    }

    #[test]
    fn recognized_games_matches_known_titles_and_rejects_desktop_shells() {
        for game in [
            "VALORANT-Win64-Shipping",
            "valorant",
            "cs2",
            "r5apex",
            "FortniteClient-Win64-Shipping",
            "Overwatch",
            "RocketLeague",
            "Terraria",
            "helldivers2",
            "Discovery-D",
        ] {
            assert!(super::is_recognized_game(Some(game)), "{game}");
        }
        for not_game in [
            None,
            Some("explorer"),
            Some("chrome"),
            Some("Discord"),
            Some("TextInputHost"),
        ] {
            assert!(!super::is_recognized_game(not_game), "{not_game:?}");
        }
    }

    #[test]
    fn automatic_game_target_never_resolves_to_desktop_shell() {
        if let Some((target, _fullscreen)) = super::capture_target_window("game:auto") {
            assert_ne!(target.app_name.as_deref(), Some("explorer"));
            assert!(super::is_recognized_game(target.app_name.as_deref()));
        }
    }

    #[test]
    #[ignore = "enumerates the interactive desktop; optional SILK_PICKER_EXPECT_APP requires a live target"]
    fn native_window_picker_filters_helpers_and_keeps_expected_app() {
        let sources = super::enumerate_windows().unwrap();
        assert!(
            !sources.is_empty(),
            "No live application windows available for verification"
        );
        for source in &sources {
            assert!(!is_picker_system_utility(source.app_name.as_deref()));
            assert!(parse_window_source(&source.id).is_some());
        }
        if let Ok(expected) = std::env::var("SILK_PICKER_EXPECT_APP") {
            assert!(
                sources.iter().any(|source| source
                    .app_name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&expected))),
                "Expected running app was missing from picker: {expected}"
            );
        }
        println!(
            "Native picker verified: {} application windows; system helpers absent",
            sources.len()
        );
    }

    #[test]
    fn application_capture_exclusions_are_respected() {
        assert!(!capture_affinity_denied(None));
        assert!(!capture_affinity_denied(Some(0)));
        assert!(capture_affinity_denied(Some(1)));
        assert!(capture_affinity_denied(Some(17)));
    }

    #[test]
    fn window_identity_is_strict_and_preserves_process_binding() {
        assert_eq!(
            parse_window_source("pid:42:created:123:hwnd:120"),
            Some((42, 123, 120))
        );
        assert_eq!(parse_window_source("hwnd:120"), None);
        for source in [
            "auto",
            "pid:0:hwnd:120",
            "pid:42:hwnd:0",
            "pid:42:hwnd:x",
            "hwnd:0",
            "hwnd:-1",
            "pid:42:hwnd:120",
            "pid:42:created:0:hwnd:120",
            "pid:0:created:123:hwnd:120",
            "pid:42:created:123:hwnd:0",
            "pid:42:created:123:hwnd:120:extra",
            "pid:+42:created:123:hwnd:120",
            "pid:42:created:+123:hwnd:120",
            "pid:42:created:18446744073709551616:hwnd:120",
        ] {
            assert_eq!(parse_window_source(source), None, "{source}");
        }
    }
}
