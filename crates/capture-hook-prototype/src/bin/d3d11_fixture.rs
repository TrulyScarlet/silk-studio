//! Original controlled renderer. Never discovers, opens, or injects into games.

#[cfg(all(windows, target_arch = "x86_64"))]
#[path = "d3d11_fixture/transport_fixture.rs"]
mod transport_fixture;

#[cfg(all(windows, target_arch = "x86_64"))]
#[path = "d3d11_fixture/helper_fixture.rs"]
mod helper_fixture;

#[cfg(all(windows, target_arch = "x86_64", feature = "resident-hooks"))]
#[path = "d3d11_fixture/resident_fixture.rs"]
mod resident_fixture;

#[cfg(all(windows, target_arch = "x86_64", feature = "fixture-probes"))]
#[path = "d3d11_fixture/lifecycle_fixture.rs"]
mod lifecycle_fixture;

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn main() {
    eprintln!("The D3D11 hook fixture requires x64 Windows.");
    std::process::exit(2);
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !std::env::args().any(|arg| arg == "--allow-self-hook") {
        return Err("Development fixture only. Pass --allow-self-hook to hook this process's own renderer; no other process will be targeted.".into());
    }
    if let Some(fault) =
        std::env::args().find_map(|arg| arg.strip_prefix("--transport-fault=").map(str::to_owned))
    {
        return transport_fixture::fault(&fault).map_err(Into::into);
    }
    if std::env::args().any(|arg| arg == "--transport-test") {
        return transport_fixture::parent(std::env::args().any(|arg| arg == "--warp"))
            .map_err(Into::into);
    }
    if std::env::args().any(|arg| arg == "--helper-test") {
        return helper_fixture::parent(std::env::args().any(|arg| arg == "--warp"))
            .map_err(Into::into);
    }
    if std::env::args().any(|arg| arg == "--lifecycle-test") {
        #[cfg(feature = "fixture-probes")]
        return lifecycle_fixture::run(std::env::args().any(|arg| arg == "--warp"))
            .map_err(Into::into);
        #[cfg(not(feature = "fixture-probes"))]
        return Err("lifecycle tests require the explicit fixture-probes feature".into());
    }
    if std::env::args().any(|arg| arg == "--resident-test") {
        #[cfg(feature = "resident-hooks")]
        return resident_fixture::parent(std::env::args().any(|arg| arg == "--warp"))
            .map_err(Into::into);
        #[cfg(not(feature = "resident-hooks"))]
        return Err("resident tests require the explicit resident-hooks feature".into());
    }
    fixture::run().map_err(Into::into)
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod fixture {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use capture_hook_prototype::native::{own_window_identity, CapturedFrame, PresentTap};
    use capture_hook_prototype::{SessionGate, SessionIdentity};
    use gpu_windows::BgraReadback;
    use windows::core::{w, Interface};
    use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
        ID3D11Texture2D, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_MODE_DESC, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{
        IDXGIFactory, IDXGISwapChain, DXGI_PRESENT, DXGI_PRESENT_TEST, DXGI_SWAP_CHAIN_DESC,
        DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW,
        RegisterClassW, TranslateMessage, CS_OWNDC, MSG, PM_REMOVE, WINDOW_EX_STYLE, WNDCLASSW,
        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    struct Window(HWND);
    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        DefWindowProcW(window, message, wparam, lparam)
    }
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    fn render(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        chain: &IDXGISwapChain,
        color: [f32; 4],
    ) -> Result<(), String> {
        unsafe {
            let buffer: ID3D11Texture2D = chain.GetBuffer(0).map_err(|error| error.to_string())?;
            let mut view: Option<ID3D11RenderTargetView> = None;
            device
                .CreateRenderTargetView(&buffer, None, Some(&mut view))
                .map_err(|error| error.to_string())?;
            context.ClearRenderTargetView(view.as_ref().ok_or("null render target")?, &color);
        }
        Ok(())
    }

    fn present(chain: &IDXGISwapChain) -> Result<(), String> {
        // Use the COM method, not the original-function trampoline: this must
        // traverse the actual installed function detour.
        unsafe { chain.Present(0, DXGI_PRESENT(0)).ok() }.map_err(|error| error.to_string())
    }

    fn pixels(frame: &CapturedFrame, expected: [u8; 4]) -> Result<(), String> {
        let mut readback = BgraReadback::default();
        let bytes = readback
            .read_scaled(&frame.surface, frame.header.width, frame.header.height)
            .map_err(|error| error.to_string())?;
        if bytes.len() != frame.header.width as usize * frame.header.height as usize * 4 {
            return Err("readback dimensions do not match source dimensions".into());
        }
        if !bytes.as_chunks::<4>().0.iter().all(|pixel| {
            pixel
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
        }) {
            return Err(format!(
                "captured pixels do not match {expected:?}; first pixel {:?}",
                &bytes[..4]
            ));
        }
        Ok(())
    }

    pub fn run() -> Result<(), String> {
        let warp = std::env::args().any(|arg| arg == "--warp");
        let instance = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
        let class = w!("SilkControlledD3D11HookFixture");
        let window_class = WNDCLASSW {
            style: CS_OWNDC,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&window_class) } == 0 {
            return Err("could not register fixture window".into());
        }
        let window = Window(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class,
                    w!("Silk development-only D3D11 hook fixture"),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    40,
                    40,
                    680,
                    540,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
            }
            .map_err(|error| error.to_string())?,
        );
        let description = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: 640,
                Height: 480,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ..Default::default()
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            OutputWindow: window.0,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
            ..Default::default()
        };
        let mut chain = None;
        let mut device = None;
        let mut context = None;
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
                Some(&description),
                Some(&mut chain),
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|error| error.to_string())?;
        let chain: IDXGISwapChain = chain.ok_or("null swap chain")?;
        let device = device.ok_or("null D3D11 device")?;
        let context = context.ok_or("null D3D11 context")?;
        if std::env::args().any(|arg| arg == "--passive-renderer") {
            if !std::env::args().any(|arg| arg == "--allow-external-helper") {
                return Err(
                    "passive attachment fixture requires explicit experimental opt-in".into(),
                );
            }
            // Capture-blind renderer: never loads a helper, hands out a chain,
            // reads capture commands, or pauses for capture lifecycle changes.
            let other_window = Window(
                unsafe {
                    CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        class,
                        w!("Silk passive unrelated renderer"),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        760,
                        40,
                        680,
                        540,
                        None,
                        None,
                        Some(instance.into()),
                        None,
                    )
                }
                .map_err(|e| e.to_string())?,
            );
            let dxgi: windows::Win32::Graphics::Dxgi::IDXGIDevice =
                device.cast().map_err(|e| e.to_string())?;
            let adapter = unsafe { dxgi.GetAdapter() }.map_err(|e| e.to_string())?;
            let factory: IDXGIFactory =
                unsafe { adapter.GetParent() }.map_err(|e| e.to_string())?;
            let other_desc = DXGI_SWAP_CHAIN_DESC {
                OutputWindow: other_window.0,
                ..description
            };
            let mut other = None;
            unsafe { factory.CreateSwapChain(&device, &other_desc, &mut other) }
                .ok()
                .map_err(|e| e.to_string())?;
            let other = other.ok_or("null passive unrelated swap chain")?;
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            let mut count = 0usize;
            while std::time::Instant::now() < deadline {
                let mut message = MSG::default();
                while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                    unsafe {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                render(&device, &context, &chain, [1., 0., 0., 1.])?;
                present(&chain)?;
                render(&device, &context, &other, [0., 0., 1., 1.])?;
                present(&other)?;
                count += 1;
                // Test-only progress observation; no pointer/command handoff.
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::SetPropW(
                        window.0,
                        w!("SilkFixturePresents"),
                        Some(windows::Win32::Foundation::HANDLE(count as *mut _)),
                    )
                }
                .map_err(|e| e.to_string())?;
                std::thread::sleep(Duration::from_millis(16));
            }
            return Ok(());
        }
        if std::env::args().any(|arg| arg == "--helper-producer") {
            return crate::helper_fixture::producer(chain, device, context, warp);
        }
        if std::env::args().any(|arg| arg == "--transport-producer") {
            return crate::transport_fixture::producer(&chain, |color| {
                render(&device, &context, &chain, color)?;
                present(&chain)
            });
        }
        let session = SessionIdentity {
            target: own_window_identity(window.0)?,
            generation: 1,
            epoch: 1,
        };
        let mut gate = SessionGate::new(session).ok_or("invalid fixture identity")?;
        let wrong_identity = SessionIdentity {
            target: capture_hook_prototype::TargetIdentity {
                creation_time: session.target.creation_time + 1,
                ..session.target
            },
            ..session
        };
        if unsafe { PresentTap::attach(&chain, wrong_identity) }.is_ok() {
            return Err("a mismatched process creation time was accepted".into());
        }
        // Safety: this fixture has one rendering thread, owns the swap chain,
        // and no Present call is in flight during attachment or teardown.
        let (mut tap, receiver, stats) = unsafe { PresentTap::attach(&chain, session) }?;
        if unsafe { PresentTap::attach(&chain, session) }.is_ok() {
            return Err("a second concurrent tap was accepted".into());
        }
        unsafe { chain.Present(0, DXGI_PRESENT_TEST).ok() }.map_err(|error| error.to_string())?;
        if receiver.try_recv().is_ok() {
            return Err("a DXGI_PRESENT_TEST call published a frame".into());
        }
        render(&device, &context, &chain, [1.0, 0.0, 0.0, 1.0])?;
        present(&chain)?;
        let first = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| error.to_string())?;
        gate.accept(first.header)
            .map_err(|error| format!("metadata rejected: {error:?}"))?;
        pixels(&first, [0, 0, 255, 255])?;
        // A second swap chain calling the same hooked Present implementation
        // must keep presenting without becoming a capture source.
        let other_window = Window(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class,
                    w!("Silk unrelated fixture window"),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    760,
                    40,
                    680,
                    540,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
            }
            .map_err(|error| error.to_string())?,
        );
        let dxgi_device: windows::Win32::Graphics::Dxgi::IDXGIDevice =
            device.cast().map_err(|error| error.to_string())?;
        let adapter = unsafe { dxgi_device.GetAdapter() }.map_err(|error| error.to_string())?;
        let factory: IDXGIFactory =
            unsafe { adapter.GetParent() }.map_err(|error| error.to_string())?;
        let mut other_chain = None;
        unsafe {
            factory.CreateSwapChain(
                &device,
                &DXGI_SWAP_CHAIN_DESC {
                    OutputWindow: other_window.0,
                    ..description
                },
                &mut other_chain,
            )
        }
        .ok()
        .map_err(|error| error.to_string())?;
        let other_chain = other_chain.ok_or("null unrelated swap chain")?;
        render(&device, &context, &other_chain, [1.0, 1.0, 0.0, 1.0])?;
        present(&other_chain)?;
        if receiver.try_recv().is_ok() || stats.intercepted.load(Ordering::Acquire) != 2 {
            return Err("an unrelated swap chain leaked into the selected capture".into());
        }
        drop(other_chain);
        for _ in 0..12 {
            render(&device, &context, &chain, [0.0, 0.0, 1.0, 1.0])?;
            present(&chain)?;
        }
        if stats.intercepted.load(Ordering::Acquire) != 14
            || stats.dropped.load(Ordering::Acquire) == 0
        {
            return Err("detour/backpressure counters did not match Present calls".into());
        }
        // Holding the first frame while new blue frames arrive must keep it red.
        pixels(&first, [0, 0, 255, 255])?;
        for frame in receiver.try_iter() {
            gate.accept(frame.header)
                .map_err(|error| format!("metadata rejected: {error:?}"))?;
            pixels(&frame, [255, 0, 0, 255])?;
        }
        unsafe { chain.ResizeBuffers(2, 800, 600, DXGI_FORMAT_UNKNOWN, Default::default()) }
            .map_err(|error| error.to_string())?;
        render(&device, &context, &chain, [0.0, 1.0, 0.0, 1.0])?;
        present(&chain)?;
        if !stats.invalidated.load(Ordering::Acquire) || receiver.try_recv().is_ok() {
            return Err("resize did not invalidate the old capture epoch".into());
        }
        tap.detach()?;
        tap.detach()?;
        gate.close();
        pixels(&first, [0, 0, 255, 255])?;
        let next_session = SessionIdentity {
            epoch: 2,
            ..session
        };
        let (mut tap, receiver, stats) = unsafe { PresentTap::attach(&chain, next_session) }?;
        let mut next_gate = SessionGate::new(next_session).ok_or("invalid resized identity")?;
        render(&device, &context, &chain, [0.0, 1.0, 0.0, 1.0])?;
        present(&chain)?;
        let frame = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| error.to_string())?;
        next_gate
            .accept(frame.header)
            .map_err(|error| format!("resized metadata rejected: {error:?}"))?;
        if (frame.header.width, frame.header.height) != (800, 600) {
            return Err("resized source dimensions were lost".into());
        }
        pixels(&frame, [0, 255, 0, 255])?;
        drop(receiver);
        render(&device, &context, &chain, [0.0, 1.0, 0.0, 1.0])?;
        present(&chain)?;
        if !stats.invalidated.load(Ordering::Acquire) {
            return Err("consumer disconnect did not invalidate capture".into());
        }
        tap.detach()?;
        // A detached chain continues presenting normally.
        present(&chain)?;
        let mut message = MSG::default();
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        // Query the actual selected adapter rather than inferring vendor support.
        let dxgi: windows::Win32::Graphics::Dxgi::IDXGIDevice =
            device.cast().map_err(|error| error.to_string())?;
        let adapter = unsafe { dxgi.GetAdapter() }.map_err(|error| error.to_string())?;
        let adapter = unsafe { adapter.GetDesc() }.map_err(|error| error.to_string())?;
        let name = String::from_utf16_lossy(&adapter.Description)
            .trim_end_matches('\0')
            .to_string();
        println!("PASS: actual D3D11 Present detour; red/blue/green pixels; target isolation; identity rejection; 3-slot backpressure; retained leases; resize epoch; disconnect; detach/reattach. driver={} adapter={name}", if warp { "WARP" } else { "hardware" });
        Ok(())
    }
}
