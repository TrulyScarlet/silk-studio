//! Explicit remote worker, never called under loader lock or from DllMain.
use capture_hook_prototype::cooperative_peer::{read_message, write_message};
use capture_hook_prototype::native::own_window_identity;
use capture_hook_prototype::resident::{ResidentCapture, FORWARDED};
use capture_hook_prototype::shared_transport::ProducerRing;
use capture_hook_prototype::transport::{Message, PACKET_SIZE};
use capture_hook_prototype::SessionIdentity;
use gpu_windows::GpuDeviceContext;
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::{
    mpsc::{sync_channel, TryRecvError},
    Arc,
};
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_DESC, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WNDCLASSW,
};

struct Probe {
    chain: IDXGISwapChain,
    window: HWND,
}
unsafe extern "system" fn wndproc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(window, message, wparam, lparam)
}
impl Probe {
    fn new(warp: bool) -> Result<Self, String> {
        let instance = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;
        let class = w!("SilkResidentD3D11Probe");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&wc) } == 0 {
            return Err("resident probe class registration failed".into());
        }
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                class,
                WINDOW_STYLE(0),
                0,
                0,
                16,
                16,
                None,
                None,
                Some(instance.into()),
                None,
            )
        }
        .map_err(|e| e.to_string())?;
        let result = (|| {
            let desc = DXGI_SWAP_CHAIN_DESC {
                BufferDesc: DXGI_MODE_DESC {
                    Width: 16,
                    Height: 16,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    ..Default::default()
                },
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                OutputWindow: window,
                Windowed: true.into(),
                SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                ..Default::default()
            };
            let mut chain = None;
            unsafe {
                D3D11CreateDeviceAndSwapChain(
                    None,
                    if warp {
                        D3D_DRIVER_TYPE_WARP
                    } else {
                        D3D_DRIVER_TYPE_HARDWARE
                    },
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&desc),
                    Some(&mut chain),
                    None,
                    None,
                    None,
                )
            }
            .map_err(|e| e.to_string())?;
            Ok(Self {
                chain: chain.ok_or("null resident probe chain")?,
                window,
            })
        })();
        if result.is_err() {
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
        result
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.window);
        }
    }
}

fn run(session: SessionIdentity) -> Result<(), String> {
    if !std::env::args().any(|arg| arg == "--allow-external-helper")
        || !std::env::args().any(|arg| arg == "--passive-renderer")
        || !std::env::current_exe().ok().is_some_and(|path| {
            path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .eq_ignore_ascii_case("silk-d3d11-hook-fixture.exe")
            })
        })
    {
        return Err(
            "external worker is restricted to the explicitly opted-in passive owned fixture".into(),
        );
    }
    let window = HWND(session.target.window as usize as *mut c_void);
    if own_window_identity(window)? != session.target {
        return Err("resident bootstrap target identity mismatch".into());
    }
    let mut pinned = HMODULE::default();
    unsafe {
        windows::Win32::System::LibraryLoader::GetModuleHandleExW(
            windows::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                | windows::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_PIN,
            windows::core::PCWSTR(silk_capture_resident_worker_v1 as *const () as *const u16),
            &mut pinned,
        )
    }
    .map_err(|e| e.to_string())?;
    let probe = Probe::new(std::env::args().any(|arg| arg == "--warp"))?;
    let (capture, chain) = unsafe { ResidentCapture::discover(&probe.chain, session) }?;
    let desc = unsafe { chain.GetDesc() }.map_err(|e| e.to_string())?;
    let device: ID3D11Device = unsafe { chain.GetDevice() }.map_err(|e| e.to_string())?;
    let context = unsafe { device.GetImmediateContext() }.map_err(|e| e.to_string())?;
    let mut ring = ProducerRing::new(
        Arc::new(GpuDeviceContext::new(device, context)),
        session,
        desc.BufferDesc.Width,
        desc.BufferDesc.Height,
    )?;
    let (commands, input) = sync_channel(4);
    let _reader = std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        loop {
            let message = read_message(&mut stdin);
            let failed = message.is_err();
            if commands.send(message).is_err() || failed {
                break;
            }
        }
    });
    let (output, messages) = sync_channel(8);
    let writer = std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        for message in messages {
            write_message(&mut stdout, message)?;
        }
        Ok::<(), String>(())
    });
    let emit = |message| {
        output
            .try_send(message)
            .map_err(|e| format!("resident bounded output unavailable: {e}"))
    };
    emit(Message::Hello(ring.hello()))?;
    let mut published = 0;
    let mut dropped = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = (|| loop {
        if Instant::now() > deadline {
            return Err("resident test session deadline".into());
        }
        match input.try_recv() {
            Ok(Ok(Message::Ack {
                session,
                slot,
                sequence,
            })) => ring.acknowledge(session, slot, sequence)?,
            Ok(Ok(Message::Render { .. })) => emit(Message::Report {
                presented: FORWARDED.load(Ordering::Acquire),
                published,
                dropped,
            })?,
            Ok(Ok(Message::Stop)) | Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                if !capture.cancel() {
                    return Err("resident capture drain uncertain; state retained".into());
                }
                emit(Message::Bye)?;
                return Ok(());
            }
            Ok(Ok(_)) => return Err("unexpected resident control packet".into()),
            Err(TryRecvError::Empty) => {}
        }
        if capture.stats.invalidated.load(Ordering::Acquire) {
            return Err("resident source invalidated; no rearm or desktop fallback".into());
        }
        if let Ok(frame) = capture.frames.try_recv() {
            match ring.export(&frame)? {
                Some(header) => {
                    emit(Message::Frame(header))?;
                    published += 1;
                }
                None => dropped += 1,
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    })();
    capture.cancel();
    drop(output);
    if result.is_ok() {
        writer.join().map_err(|_| "resident writer panicked")??;
    }
    result
}

/// # Safety
/// Only the owned-fixture loader may call this export. Parameter must point to
/// a readable PACKET_SIZE allocation retained until the Hello or thread exit.
/// The normal LoadLibrary reference and executable forwarder remain until exit.
#[no_mangle]
pub unsafe extern "system" fn silk_capture_resident_worker_v1(parameter: *mut c_void) -> u32 {
    if parameter.is_null() {
        return 10;
    }
    let mut bytes = [0; PACKET_SIZE];
    std::ptr::copy_nonoverlapping(parameter.cast::<u8>(), bytes.as_mut_ptr(), PACKET_SIZE);
    match std::panic::catch_unwind(|| {
        let Message::Select(session) =
            Message::decode(&bytes).map_err(|_| "invalid bootstrap packet")?
        else {
            return Err("bootstrap selection required".to_string());
        };
        run(session)
    }) {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            eprintln!("resident owned-fixture worker stopped: {error}");
            10
        }
        Err(_) => 11,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_and_invalid_bootstrap_fail_before_attachment() {
        assert_eq!(
            unsafe { silk_capture_resident_worker_v1(std::ptr::null_mut()) },
            10
        );
        let mut bytes = Message::Stop.encode();
        assert_eq!(
            unsafe { silk_capture_resident_worker_v1(bytes.as_mut_ptr().cast()) },
            10
        );
    }
    #[test]
    fn non_fixture_process_cannot_start_resident_capture() {
        let mut bytes = Message::Select(SessionIdentity {
            target: capture_hook_prototype::TargetIdentity {
                pid: 7,
                creation_time: 123,
                window: 456,
            },
            generation: 1,
            epoch: 1,
        })
        .encode();
        assert_eq!(
            unsafe { silk_capture_resident_worker_v1(bytes.as_mut_ptr().cast()) },
            10
        );
    }
}
