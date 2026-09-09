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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsAdapterInfo {
    pub name: String,
    pub vendor_id: u32,
}

fn factory() -> CaptureResult<IDXGIFactory1> {
    unsafe {
        CreateDXGIFactory1::<IDXGIFactory1>().map_err(|err| VideoCaptureError::Backend {
            details: format!("CreateDXGIFactory1 failed: {err}"),
        })
    }
}

fn adapter_outputs(adapter: &IDXGIAdapter1) -> CaptureResult<Vec<IDXGIOutput>> {
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

fn describe(output: &IDXGIOutput) -> Option<DXGI_OUTPUT_DESC> {
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
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GWL_STYLE, WS_CHILD,
        WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };

    let mut windows_list = Vec::new();

    unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let windows_list = &mut *(lparam.0 as *mut Vec<media_types::WindowSourceInfo>);

        if !IsWindowVisible(hwnd).as_bool() {
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

        let mut cloaked: u32 = 0;
        let cloaked_res = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );
        if cloaked_res.is_ok() && cloaked != 0 {
            return BOOL(1);
        }

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

        let id = format!("hwnd:{}", hwnd.0 as usize);
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
