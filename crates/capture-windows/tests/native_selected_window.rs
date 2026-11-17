//! Explicit opt-in non-injected VALORANT window probe. Never a monitor fallback.
#![cfg(windows)]
use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureEvent};
use capture_windows::{DisplayCapture, GpuTextureSlot};
use gpu_windows::BgraReadback;
use media_types::StreamId;
use std::collections::BTreeSet;
use std::io::Write;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{FILETIME, HWND};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
};

#[derive(Debug)]
struct Selection {
    pid: u32,
    hwnd: usize,
    created: u64,
}
fn selection(pid: &str, hwnd: &str, created: &str) -> Result<Selection, String> {
    let s = Selection {
        pid: pid.parse().map_err(|_| "invalid PID")?,
        hwnd: hwnd.parse().map_err(|_| "invalid HWND")?,
        created: created
            .parse()
            .map_err(|_| "invalid process creation time")?,
    };
    if s.pid == 0 || s.hwnd == 0 || s.created == 0 {
        return Err("explicit nonzero PID/HWND/creation time required".into());
    }
    Ok(s)
}
#[test]
fn selection_never_defaults_to_desktop_or_foreground() {
    for (pid, hwnd, created) in [
        ("", "1", "1"),
        ("0", "1", "1"),
        ("1", "0", "1"),
        ("1", "1", "0"),
        ("auto", "1", "1"),
    ] {
        assert!(selection(pid, hwnd, created).is_err());
    }
}
fn probe() -> Result<String, String> {
    let get = |name| std::env::var(name).map_err(|_| format!("explicit {name} opt-in is required"));
    let selected = selection(
        &get("SILK_GAME_PROBE_PID")?,
        &get("SILK_GAME_PROBE_HWND")?,
        &get("SILK_GAME_PROBE_CREATION")?,
    )?;
    let directory = PathBuf::from(get("SILK_GAME_PROBE_OUTPUT")?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let allowed = std::env::temp_dir()
        .join("opencode")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !directory.starts_with(allowed) || !directory.is_dir() {
        return Err("probe artifacts must use the approved opencode temp directory".into());
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, selected.pid) }
        .map_err(|e| e.to_string())?;
    let _process = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    let mut path = [0u16; 32768];
    let mut length = path.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|e| e.to_string())?;
    let path = PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
    let expected_exe = std::env::var("SILK_GAME_PROBE_EXE")
        .unwrap_or_else(|_| "VALORANT-Win64-Shipping.exe".to_string());
    if !path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(&expected_exe))
    {
        return Err(format!(
            "probe accepts only the explicitly selected {expected_exe} executable"
        ));
    }
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|e| e.to_string())?;
    if ((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
        != selected.created
    {
        return Err("selected VALORANT process lifetime changed".into());
    }
    let window = HWND(selected.hwnd as *mut _);
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    if pid != selected.pid || !unsafe { IsWindow(Some(window)) }.as_bool() {
        return Err("selected VALORANT HWND no longer matches".into());
    }
    let foreground = unsafe { GetForegroundWindow() } == window;
    let iconic = unsafe { IsIconic(window) }.as_bool();
    println!("Selected VALORANT window preflight: pid={}, hwnd={}, foreground={foreground}, minimized={iconic}; no game/security configuration changes",selected.pid,selected.hwnd);
    let source_id = format!(
        "game:pid:{}:created:{}:hwnd:{}",
        selected.pid, selected.created, selected.hwnd
    );
    let mut capture = DisplayCapture::new(StreamId(0));
    let mut affinity = 0;
    let affinity = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowDisplayAffinity(window, &mut affinity)
    }
    .ok()
    .map(|()| affinity);
    if matches!(affinity, Some(1 | 17)) {
        let result = capture.start(VideoCaptureConfig {
            source_id: source_id.clone(),
            target_fps: 30,
        });
        assert!(
            matches!(
                result,
                Err(capture_api::VideoCaptureError::PermissionDenied)
            ),
            "excluded window was not denied"
        );
        assert!(capture.backend_name().is_none());
        assert!(capture.gpu_frame_context().is_none());
        let evidence=format!("{{\"route\":\"wgc-window\",\"result\":\"blocked_by_application_display_affinity\",\"affinity\":{},\"captured_frames\":0,\"source\":\"{source_id}\",\"foreground_at_start\":{foreground},\"minimized_at_start\":{iconic},\"monitor_fallback\":false,\"injection\":false,\"game_settings_modified\":false}}",affinity.unwrap());
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("evidence.json"))
            .map_err(|e| e.to_string())?;
        file.write_all(evidence.as_bytes())
            .map_err(|e| e.to_string())?;
        return Ok(evidence);
    }
    capture
        .start(VideoCaptureConfig {
            source_id: source_id.clone(),
            target_fps: 30,
        })
        .map_err(|e| e.to_string())?;
    if capture.backend_name() != Some("wgc-window") {
        return Err("isolated source did not select wgc-window".into());
    }
    let context = capture
        .gpu_frame_context()
        .ok_or("missing WGC GPU context")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut frames = 0;
    let mut last = -1;
    let mut dimensions = (0, 0);
    let mut preview = (0, 0);
    let mut colors = BTreeSet::new();
    let mut nonblack = 0;
    let mut readback = BgraReadback::default();
    while frames < 30 && Instant::now() < deadline {
        match capture.next_event().map_err(|e| e.to_string())? {
            VideoCaptureEvent::FormatChanged { .. } | VideoCaptureEvent::Idle => {}
            VideoCaptureEvent::SourceLost => return Err("VALORANT WGC source lost".into()),
            VideoCaptureEvent::Frame(frame) => {
                if frame.pts < last {
                    return Err("WGC timestamp moved backwards".into());
                }
                last = frame.pts;
                let Ok(lease) = context.resolve(&frame) else {
                    continue;
                };
                let texture = lease
                    .downcast_ref::<GpuTextureSlot>()
                    .ok_or("WGC frame was not a D3D11 texture")?;
                if frame.width == 0 || frame.height == 0 {
                    return Err("empty WGC frame".into());
                }
                if frames == 0 {
                    dimensions = (frame.width, frame.height);
                    preview = (
                        640,
                        (u64::from(frame.height) * 640 / u64::from(frame.width)).clamp(1, 720)
                            as u32,
                    );
                    let bytes = readback
                        .read_scaled(texture, preview.0, preview.1)
                        .map_err(|e| e.to_string())?;
                    for pixel in bytes.as_chunks::<4>().0.iter().step_by(97) {
                        colors.insert([pixel[0], pixel[1], pixel[2]]);
                    }
                    nonblack = bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
                        .count();
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(directory.join("preview.bgra"))
                        .map_err(|e| e.to_string())?;
                    file.write_all(&bytes).map_err(|e| e.to_string())?;
                }
                frames += 1;
            }
        }
    }
    capture.stop().map_err(|e| e.to_string())?;
    if frames == 0 {
        return Err("no resolved VALORANT WGC frames".into());
    }
    let evidence=format!("{{\"route\":\"wgc-window\",\"source\":\"{source_id}\",\"frames\":{frames},\"width\":{},\"height\":{},\"last_pts_ms\":{last},\"preview_width\":{},\"preview_height\":{},\"sampled_colors\":{},\"nonblack_preview_pixels\":{nonblack},\"foreground_at_start\":{foreground},\"minimized_at_start\":{iconic},\"injection\":false,\"game_settings_modified\":false}}",dimensions.0,dimensions.1,preview.0,preview.1,colors.len());
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("evidence.json"))
        .map_err(|e| e.to_string())?;
    file.write_all(evidence.as_bytes())
        .map_err(|e| e.to_string())?;
    if colors.len() < 16 || nonblack == 0 {
        return Err(format!("resolved frames but preview is blank/low-information; visual verification required: {evidence}"));
    }
    Ok(evidence)
}
#[test]
#[ignore = "explicit user-selected VALORANT PID/HWND/lifetime; OS WGC only; writes diagnostic preview under approved temp"]
fn explicitly_selected_valorant_wgc_probe() {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = send.send(probe());
    });
    let evidence = receive
        .recv_timeout(Duration::from_secs(20))
        .expect("non-injected window probe timed out; no game/security settings will be changed")
        .expect("VALORANT WGC probe did not verify capture");
    println!("OBSERVED non-injected selected-window probe result: {evidence}. A blocked result is NOT capture support; no hook/fullscreen/anti-cheat endorsement or account-safety evidence.");
}
