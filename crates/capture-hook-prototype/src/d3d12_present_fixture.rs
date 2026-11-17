//! Owned swap-chain renderer with validated cooperative queue associations.
//! Optional owned Present detour; no external HWND or general queue discovery.
use super::d3d12_fixture::{barrier, D3d12Fixture, FixtureFrame};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::core::{w, Interface};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

fn error(e: windows::core::Error) -> String {
    e.to_string()
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
static GEOMETRY_WINDOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "d3d12-auto-resize-fixture")]
static GEOMETRY_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "d3d12-auto-resize-fixture")]
pub const OWNED_GEOMETRY_MESSAGE: u32 = WM_APP + 77;
#[cfg(feature = "d3d12-auto-resize-fixture")]
pub const OWNED_RESIZE_FAULT_MESSAGE: u32 = WM_APP + 78;
#[cfg(feature = "d3d12-observed-loss-fixture")]
pub const OWNED_HELPER_LOSS_MESSAGE: u32 = WM_APP + 79;
#[cfg(feature = "d3d12-observed-loss-fixture")]
pub const OWNED_HELPER_RELEASED_MESSAGE: u32 = WM_APP + 80;
#[cfg(feature = "d3d12-observed-loss-fixture")]
mod owner_loss {
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
    pub static WINDOW: AtomicUsize = AtomicUsize::new(0);
    pub static ARMED: AtomicBool = AtomicBool::new(false);
    pub static REQUESTED: AtomicBool = AtomicBool::new(false);
    pub static WAITING: AtomicBool = AtomicBool::new(false);
    pub static RELEASED: AtomicU64 = AtomicU64::new(0);
}
unsafe extern "system" fn wndproc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    #[cfg(feature = "d3d12-observed-loss-fixture")]
    if owner_loss::WINDOW.load(std::sync::atomic::Ordering::Acquire) == window.0 as usize {
        use std::sync::atomic::Ordering;
        if message == OWNED_HELPER_LOSS_MESSAGE
            && wparam.0 == 0
            && lparam.0 == 0
            && owner_loss::ARMED
                .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            owner_loss::REQUESTED.store(true, Ordering::Release);
            return LRESULT(0);
        }
        if message == OWNED_HELPER_RELEASED_MESSAGE
            && wparam.0 == 2
            && lparam.0 == 0
            && owner_loss::WAITING.load(Ordering::Acquire)
        {
            owner_loss::RELEASED.store(2, Ordering::Release);
            return LRESULT(0);
        }
    }
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    if message == OWNED_RESIZE_FAULT_MESSAGE
        && GEOMETRY_WINDOW.load(std::sync::atomic::Ordering::Acquire) == window.0 as usize
        && matches!(wparam.0, 1 | 2)
        && lparam.0 == 0
    {
        let _ = GEOMETRY_REQUEST.compare_exchange(
            0,
            wparam.0 as u64,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        );
        return LRESULT(0);
    }
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    if message == OWNED_GEOMETRY_MESSAGE
        && GEOMETRY_WINDOW.load(std::sync::atomic::Ordering::Acquire) == window.0 as usize
        && (2..=1920).contains(&wparam.0)
        && (2..=1080).contains(&lparam.0)
        && wparam.0.is_multiple_of(2)
        && lparam.0 % 2 == 0
    {
        let packed = ((wparam.0 as u64) << 32) | lparam.0 as u64;
        // Exactly one pending scalar request; never allocate or wait here.
        let _ = GEOMETRY_REQUEST.compare_exchange(
            0,
            packed,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        );
        return LRESULT(0);
    }
    DefWindowProcW(window, message, wparam, lparam)
}
pub(crate) struct Window(pub(crate) HWND);
impl Window {
    pub(crate) fn new(selected: bool, width: u32, height: u32) -> Result<Self, String> {
        static CLASS: OnceLock<Result<(), String>> = OnceLock::new();
        CLASS
            .get_or_init(|| unsafe {
                let instance = GetModuleHandleW(None).map_err(error)?;
                let class = WNDCLASSW {
                    lpfnWndProc: Some(wndproc),
                    hInstance: instance.into(),
                    lpszClassName: w!("SilkOwnedD3d12PresentationFixture"),
                    ..Default::default()
                };
                if RegisterClassW(&class) == 0 {
                    return Err("owned D3D12 window class registration failed".into());
                }
                Ok(())
            })
            .clone()?;
        unsafe {
            let instance = GetModuleHandleW(None).map_err(error)?;
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("SilkOwnedD3d12PresentationFixture"),
                if selected {
                    w!("Silk owned D3D12 selected fixture")
                } else {
                    w!("Silk owned D3D12 unrelated fixture")
                },
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                if selected { 40 } else { 420 },
                40,
                width as i32 + 32,
                height as i32 + 64,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map(Self)
            .map_err(error)
        }
    }
    fn pump(&self) -> Result<(), String> {
        unsafe {
            if !IsWindow(Some(self.0)).as_bool() {
                return Err("owned D3D12 window closed".into());
            }
            let mut message = MSG::default();
            while PeekMessageW(&mut message, Some(self.0), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if !IsWindow(Some(self.0)).as_bool() {
                return Err("owned D3D12 window closed".into());
            }
        }
        Ok(())
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

struct RenderSlot {
    target: ID3D12Resource,
    rtv: ID3D12DescriptorHeap,
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    fence_value: u64,
}
struct Chain {
    slots: Vec<RenderSlot>,
    chain: IDXGISwapChain3,
    window: Rc<Window>,
    association: Option<crate::d3d12_queue_registry::AssociationToken>,
}
impl Chain {
    fn slots(device: &ID3D12Device, chain: &IDXGISwapChain3) -> Result<Vec<RenderSlot>, String> {
        let mut slots = Vec::with_capacity(2);
        for index in 0..2 {
            unsafe {
                let target: ID3D12Resource = chain.GetBuffer(index).map_err(error)?;
                let rtv: ID3D12DescriptorHeap = device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                        NumDescriptors: 1,
                        ..Default::default()
                    })
                    .map_err(error)?;
                device.CreateRenderTargetView(
                    &target,
                    None,
                    rtv.GetCPUDescriptorHandleForHeapStart(),
                );
                let allocator: ID3D12CommandAllocator = device
                    .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                    .map_err(error)?;
                let list: ID3D12GraphicsCommandList = device
                    .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
                    .map_err(error)?;
                list.Close().map_err(error)?;
                slots.push(RenderSlot {
                    target,
                    rtv,
                    allocator,
                    list,
                    fence_value: 0,
                });
            }
        }
        Ok(slots)
    }
    fn with_window(
        device: &ID3D12Device,
        queue: &ID3D12CommandQueue,
        associations: &mut crate::d3d12_queue_owner::NativeQueueOwner,
        window: Rc<Window>,
        width: u32,
        height: u32,
        register: bool,
    ) -> Result<Self, String> {
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory1().map_err(error)?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                ..Default::default()
            };
            #[cfg(feature = "d3d12-factory-hook-fixture")]
            let intercepted = crate::d3d12_factory_hook::active();
            #[cfg(not(feature = "d3d12-factory-hook-fixture"))]
            let intercepted = false;
            let create_direct = || -> Result<IDXGISwapChain3, String> {
                if std::env::args().any(|argument| argument == "--legacy-factory") {
                    #[cfg(feature = "d3d12-remote-helper-fixture")]
                    let passive_helper = std::env::args().any(|arg| arg == "--passive-d3d12")
                        && std::env::args().any(|arg| arg == "--allow-external-helper")
                        && crate::d3d12_factory_hook::OwnedFactoryHook::validate_opt_in().is_ok();
                    #[cfg(not(feature = "d3d12-remote-helper-fixture"))]
                    let passive_helper = false;
                    if !intercepted && !passive_helper {
                        return Err(
                            "legacy factory fixture requires resident factory observer".into()
                        );
                    }
                    let legacy = DXGI_SWAP_CHAIN_DESC {
                        BufferDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_MODE_DESC {
                            Width: width,
                            Height: height,
                            Format: desc.Format,
                            ..Default::default()
                        },
                        SampleDesc: desc.SampleDesc,
                        BufferUsage: desc.BufferUsage,
                        BufferCount: desc.BufferCount,
                        OutputWindow: window.0,
                        Windowed: true.into(),
                        SwapEffect: desc.SwapEffect,
                        Flags: desc.Flags,
                    };
                    let mut chain = None;
                    factory
                        .CreateSwapChain(queue, &legacy, &mut chain)
                        .ok()
                        .map_err(error)?;
                    chain
                        .ok_or("legacy factory returned no chain")?
                        .cast()
                        .map_err(error)
                } else {
                    factory
                        .CreateSwapChainForHwnd(queue, window.0, &desc, None, None)
                        .map_err(error)?
                        .cast()
                        .map_err(error)
                }
            };
            let (chain, association) = if !register {
                let chain = create_direct()?;
                (chain, None)
            } else if intercepted {
                #[cfg(feature = "d3d12-factory-hook-fixture")]
                {
                    // Renderer calls DXGI directly. Queue ownership is learned
                    // exclusively by the installed factory detour, not this call site.
                    let chain = create_direct()?;
                    let token = associations.claim_intercepted(&chain, window.clone())?;
                    (chain, Some(token))
                }
                #[cfg(not(feature = "d3d12-factory-hook-fixture"))]
                {
                    unreachable!()
                }
            } else {
                let (chain, token) = associations.create(&factory, queue, window.clone(), &desc)?;
                (chain, Some(token))
            };
            // Never let Alt+Enter change display mode during this windowed test.
            factory
                .MakeWindowAssociation(window.0, DXGI_MWA_NO_ALT_ENTER)
                .map_err(error)?;
            let slots = Self::slots(device, &chain)?;
            Ok(Self {
                slots,
                chain,
                window,
                association,
            })
        }
    }
}
struct Presentation {
    associations: crate::d3d12_queue_owner::NativeQueueOwner,
    chains: Vec<Chain>,
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    fence: ID3D12Fence,
    last_signal: u64,
    uncertain: bool,
}
#[derive(Default, Clone, Copy, Debug)]
pub struct PresentStats {
    pub selected: u64,
    pub unrelated: u64,
    pub captured: u64,
    pub dropped_capture: u64,
}

/// Owner-thread-only: HWND creation, message pumping, Present and destruction.
pub struct D3d12PresentFixture {
    capture: D3d12Fixture,
    #[cfg(feature = "d3d12-gpu-fixture")]
    gpu: Option<crate::d3d12_gpu_bridge::D3d12GpuBridge>,
    #[cfg(feature = "d3d12-present-hook-fixture")]
    present_hook: Option<crate::d3d12_owned_hook::OwnedPresentHook>,
    #[cfg(feature = "d3d12-factory-hook-fixture")]
    factory_hook: Option<crate::d3d12_factory_hook::OwnedFactoryHook>,
    presentation: Option<Presentation>,
    capture_open: bool,
    failed: bool,
    stats: PresentStats,
    selected_color: [f32; 4],
    #[cfg(feature = "d3d12-cross-process")]
    windowed: Option<(isize, RECT)>,
    _owner_thread: PhantomData<Rc<()>>,
}
impl D3d12PresentFixture {
    #[cfg(feature = "d3d12-late-resize-fixture")]
    pub(crate) fn late_discovery_probe(warp: bool) -> Result<Self, String> {
        // Built-in, invisible probe class: a separately loaded DLL must not
        // register the executable's renderer class with a different wndproc.
        let window = || unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("Silk independent late queue probe"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                64,
                64,
                None,
                None,
                None,
                None,
            )
            .map(|window| Rc::new(Window(window)))
            .map_err(error)
        };
        Self::renderer_with_windows(warp, 64, 64, false, [window()?, window()?])
    }
    #[cfg(feature = "d3d12-late-resize-fixture")]
    pub(crate) fn independent_probe_chain(&self) -> IDXGISwapChain3 {
        self.presentation.as_ref().unwrap().chains[0].chain.clone()
    }
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    pub fn run_passive_renderer(warp: bool) -> Result<(), String> {
        use crate::d3d12_cross_process::{read_wire, write_wire};
        use crate::d3d12_protocol::Wire;
        use crate::transport::Message;
        use std::sync::mpsc::{sync_channel, TryRecvError};
        crate::d3d12_factory_hook::OwnedFactoryHook::validate_opt_in()?;
        if std::env::args().any(|arg| arg == "--observe-owned-loss") {
            crate::d3d12_bootstrap::validate_owned_attached_process()?;
        }
        if !std::env::args().any(|arg| arg == "--passive-d3d12")
            || !std::env::args().any(|arg| arg == "--allow-external-helper")
        {
            return Err("passive D3D12 requires explicit external-helper opt-in".into());
        }
        if std::env::args().any(|arg| {
            matches!(arg.as_str(), "--bootstrap" | "--rearm" | "--replace-device")
                || arg.starts_with("--fault=")
                || arg.starts_with("--fixture-lifetime-seconds=")
        }) {
            return Err(
                "passive D3D12 does not support integrated bootstrap/rearm/fault/extended lifetime"
                    .into(),
            );
        }
        let windows = [
            Rc::new(Window::new(true, 318, 178)?),
            Rc::new(Window::new(false, 318, 178)?),
        ];
        let session = crate::SessionIdentity {
            target: crate::native::own_window_identity(windows[0].0)?,
            generation: 1,
            epoch: 1,
        };
        let late = std::env::args().any(|arg| arg == "--late-d3d12");
        if late && !cfg!(feature = "d3d12-late-resize-fixture") {
            return Err("late fixture feature not enabled".into());
        }
        let mut existing = if late {
            let mut renderer = Self::renderer_with_windows(warp, 318, 178, false, windows.clone())?;
            renderer.cover_selected()?;
            let deadline = Instant::now() + Duration::from_secs(3);
            while renderer.stats().selected < 12 {
                renderer.step()?;
                if Instant::now() >= deadline {
                    return Err(
                        "late target did not complete twelve Presents before identity publication"
                            .into(),
                    );
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Some(renderer)
        } else {
            None
        };
        write_wire(
            &mut std::io::stdout(),
            Wire::Control(Message::Select(session)),
        )?;
        if let Some(renderer) = existing.as_ref() {
            write_wire(
                &mut std::io::stdout(),
                Wire::Control(Message::Report {
                    presented: renderer.stats().selected,
                    published: 0,
                    dropped: 0,
                }),
            )?;
        }
        // Exactly one startup packet belongs to the renderer. The helper cannot
        // read media approval until it has observed a subsequently created chain.
        let (send, receive) = sync_channel(1);
        let reader = std::thread::spawn(move || {
            let packet = read_wire(&mut std::io::stdin());
            let _ = send.try_send(packet);
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            windows[0].pump()?;
            windows[1].pump()?;
            if let Some(renderer) = existing.as_mut() {
                renderer.step()?;
            }
            match receive.try_recv() {
                Ok(Ok(Wire::Control(Message::Select(selected)))) if selected == session => {
                    reader
                        .join()
                        .map_err(|_| "passive startup reader panicked")?;
                    break;
                }
                Ok(_) | Err(TryRecvError::Disconnected) => {
                    return Err("passive startup approval mismatch/disconnected".into())
                }
                Err(TryRecvError::Empty) => {}
            }
            if Instant::now() >= deadline {
                return Err("passive startup deadline".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut renderer = if let Some(mut renderer) = existing {
            // The already-running chain remains the same COM object. Its normal
            // later ResizeBuffers1 call supplies the real presentation queue.
            if !std::env::args().any(|arg| arg == "--late-no-resize") {
                renderer.resize_selected(318, 178)?;
            }
            renderer
        } else {
            Self::renderer_with_windows(warp, 318, 178, false, windows)?
        };
        renderer.cover_selected()?;
        #[cfg(feature = "d3d12-observed-loss-fixture")]
        let mut loss_waiting = None;
        #[cfg(feature = "d3d12-observed-loss-fixture")]
        if std::env::args().any(|arg| arg == "--observe-owned-loss") {
            owner_loss::WINDOW.store(
                renderer.presentation.as_ref().unwrap().chains[0]
                    .window
                    .0
                     .0 as usize,
                std::sync::atomic::Ordering::Release,
            );
            owner_loss::ARMED.store(true, std::sync::atomic::Ordering::Release);
        }
        #[cfg(feature = "d3d12-auto-resize-fixture")]
        if std::env::args().any(|arg| arg == "--auto-resize-owned") {
            GEOMETRY_WINDOW.store(
                renderer.presentation.as_ref().unwrap().chains[0]
                    .window
                    .0
                     .0 as usize,
                std::sync::atomic::Ordering::Release,
            );
        }
        let deadline = Instant::now() + Duration::from_secs(175);
        let distinct_queues = std::env::args().any(|arg| arg == "--late-distinct-queues");
        while Instant::now() < deadline {
            #[cfg(feature = "d3d12-observed-loss-fixture")]
            {
                use std::sync::atomic::Ordering;
                if owner_loss::REQUESTED.swap(false, Ordering::AcqRel) {
                    renderer.trigger_owned_device_loss()?;
                    owner_loss::WAITING.store(true, Ordering::Release);
                    loss_waiting = Some(Instant::now());
                }
                if let Some(since) = loss_waiting {
                    renderer.pump_paused_windows()?;
                    if owner_loss::RELEASED.load(Ordering::Acquire) == 2 {
                        let windows = renderer.removed_replacement_windows()?;
                        drop(renderer);
                        renderer = Self::replace_render_only(false, 640, 360, windows)?;
                        renderer.cover_selected()?;
                        owner_loss::WAITING.store(false, Ordering::Release);
                        loss_waiting = None;
                    } else {
                        if since.elapsed() >= Duration::from_secs(10) {
                            return Err(
                                "owned renderer did not receive helper loss-release proof".into()
                            );
                        }
                        std::thread::sleep(Duration::from_millis(8));
                        continue;
                    }
                }
            }
            #[cfg(feature = "d3d12-auto-resize-fixture")]
            {
                let packed = GEOMETRY_REQUEST.swap(0, std::sync::atomic::Ordering::AcqRel);
                if packed == 1 {
                    renderer.probe_rejected_resize()?;
                } else if packed == 2 {
                    renderer.resize_selected(641, 360)?;
                } else if packed != 0 {
                    renderer.resize_selected((packed >> 32) as u32, packed as u32)?;
                    renderer.selected_color = [0., 1., 0., 1.];
                }
            }
            if distinct_queues {
                // Negative observer fixture: do not submit frames using an
                // application graph that has not implemented rotating queues.
                for chain in &renderer.presentation.as_ref().unwrap().chains {
                    chain.window.pump()?;
                }
            } else {
                renderer.step()?;
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        Ok(())
    }
    pub fn new(warp: bool, width: u32, height: u32) -> Result<Self, String> {
        Self::new_renderer(warp, width, height, true)
    }
    #[cfg(feature = "d3d12-bootstrap-fixture")]
    pub(crate) fn render_only(warp: bool, width: u32, height: u32) -> Result<Self, String> {
        Self::new_renderer(warp, width, height, false)
    }
    fn new_renderer(warp: bool, width: u32, height: u32, register: bool) -> Result<Self, String> {
        let windows = [
            Rc::new(Window::new(true, width, height)?),
            Rc::new(Window::new(false, width, height)?),
        ];
        Self::renderer_with_windows(warp, width, height, register, windows)
    }
    fn renderer_with_windows(
        warp: bool,
        width: u32,
        height: u32,
        register: bool,
        windows: [Rc<Window>; 2],
    ) -> Result<Self, String> {
        let capture = D3d12Fixture::new(warp, width, height)?;
        let (device, queue) = capture.device_queue();
        let fence: ID3D12Fence =
            unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.map_err(error)?;
        let mut associations = crate::d3d12_queue_owner::NativeQueueOwner::default();
        let selected = Chain::with_window(
            &device,
            &queue,
            &mut associations,
            windows[0].clone(),
            width,
            height,
            register,
        )?;
        let unrelated = Chain::with_window(
            &device,
            &queue,
            &mut associations,
            windows[1].clone(),
            width,
            height,
            register,
        )?;
        if let Some(token) = selected.association {
            associations.select(token)?;
        }
        let (device, queue) = if register {
            associations.selected(&selected.chain)?
        } else {
            (device, queue)
        };
        Ok(Self {
            capture,
            #[cfg(feature = "d3d12-gpu-fixture")]
            gpu: None,
            #[cfg(feature = "d3d12-present-hook-fixture")]
            present_hook: None,
            #[cfg(feature = "d3d12-factory-hook-fixture")]
            factory_hook: None,
            presentation: Some(Presentation {
                associations,
                chains: vec![selected, unrelated],
                device,
                queue,
                fence,
                last_signal: 0,
                uncertain: false,
            }),
            capture_open: register,
            failed: false,
            stats: PresentStats::default(),
            selected_color: [1., 0., 0., 1.],
            #[cfg(feature = "d3d12-cross-process")]
            windowed: None,
            _owner_thread: PhantomData,
        })
    }
    #[cfg(feature = "d3d12-bootstrap-fixture")]
    pub(crate) fn selected_target(&self) -> Result<crate::TargetIdentity, String> {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(error)?;
        Ok(crate::TargetIdentity {
            pid: std::process::id(),
            creation_time: (u64::from(created.dwHighDateTime) << 32)
                | u64::from(created.dwLowDateTime),
            window: self.presentation.as_ref().unwrap().chains[0].window.0 .0 as usize as u64,
        })
    }
    #[cfg(feature = "d3d12-gpu-fixture")]
    pub fn new_gpu(
        warp: bool,
        width: u32,
        height: u32,
        budget: std::sync::Arc<crate::local_gpu_bridge::GpuPoolBudget>,
    ) -> Result<Self, String> {
        let mut fixture = Self::new(warp, width, height)?;
        let presentation = fixture.presentation.as_ref().unwrap();
        let (device, queue) = presentation
            .associations
            .selected(&presentation.chains[0].chain)?;
        fixture.gpu = Some(crate::d3d12_gpu_bridge::D3d12GpuBridge::new(
            &device, &queue, width, height, budget, warp,
        )?);
        Ok(fixture)
    }
    #[cfg(feature = "d3d12-gpu-fixture")]
    fn access_gpu<R>(
        &mut self,
        f: impl FnOnce(&mut crate::d3d12_gpu_bridge::D3d12GpuBridge) -> Result<R, String>,
    ) -> Result<R, String> {
        #[cfg(feature = "d3d12-factory-hook-fixture")]
        if self.factory_hook.as_ref().is_some_and(|hook| hook.failed()) {
            return Err("owned factory observation lost; capture terminal".into());
        }
        #[cfg(feature = "d3d12-present-hook-fixture")]
        if let Some(hook) = &self.present_hook {
            if hook.failed() {
                return Err("owned D3D12 Present capture failed".into());
            }
            return hook.access(f);
        }
        f(self.gpu.as_mut().ok_or("not a GPU presentation fixture")?)
    }
    #[cfg(feature = "d3d12-present-hook-fixture")]
    pub fn new_gpu_present_hook(
        warp: bool,
        width: u32,
        height: u32,
        budget: std::sync::Arc<crate::local_gpu_bridge::GpuPoolBudget>,
    ) -> Result<Self, String> {
        crate::d3d12_owned_hook::OwnedPresentHook::validate_opt_in()?;
        let mut fixture = Self::new_gpu(warp, width, height, budget)?;
        let gpu = fixture.gpu.take().ok_or("missing hook media")?;
        fixture.present_hook = Some(unsafe {
            crate::d3d12_owned_hook::OwnedPresentHook::install(
                &fixture.presentation.as_ref().unwrap().chains[0].chain,
                gpu,
            )
        }?);
        Ok(fixture)
    }
    #[cfg(feature = "d3d12-present-hook-fixture")]
    pub fn hook_counts(&self) -> Option<(u64, u64, u64, u64)> {
        self.present_hook.as_ref().map(|h| h.counts())
    }
    #[cfg(feature = "d3d12-factory-hook-fixture")]
    pub fn new_gpu_factory_hook(
        warp: bool,
        width: u32,
        height: u32,
        budget: std::sync::Arc<crate::local_gpu_bridge::GpuPoolBudget>,
    ) -> Result<Self, String> {
        use crate::d3d12_factory_hook::OwnedFactoryHook;
        OwnedFactoryHook::validate_opt_in()?;
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.map_err(error)?;
        let hook = unsafe { OwnedFactoryHook::install(&factory) }?;
        let mut fixture = Self::new_gpu_present_hook(warp, width, height, budget)?;
        let (_, observed, claimed, dropped, errors) = hook.counts();
        if (observed, claimed, dropped, errors) != (2, 2, 0, 0) {
            return Err(
                "owned factory interception did not exclusively supply both associations".into(),
            );
        }
        fixture.factory_hook = Some(hook);
        Ok(fixture)
    }
    #[cfg(feature = "d3d12-factory-hook-fixture")]
    pub fn factory_counts(&self) -> Option<(u64, u64, u64, u64, u64)> {
        self.factory_hook.as_ref().map(|hook| hook.counts())
    }
    #[cfg(feature = "d3d12-gpu-fixture")]
    pub fn gpu_context(&self) -> Option<encoder_api::GpuFrameContext> {
        if !self.capture_open {
            return None;
        }
        #[cfg(feature = "d3d12-present-hook-fixture")]
        if let Some(hook) = &self.present_hook {
            return hook.access(|g| Ok(g.context())).ok();
        }
        self.gpu.as_ref().map(|gpu| gpu.context())
    }
    #[cfg(feature = "d3d12-gpu-fixture")]
    pub fn gpu_stats(&self) -> Option<crate::d3d12_gpu_bridge::GpuInteropStats> {
        #[cfg(feature = "d3d12-present-hook-fixture")]
        if let Some(hook) = &self.present_hook {
            return hook.gpu_stats();
        }
        self.gpu.as_ref().map(|gpu| gpu.stats())
    }
    #[cfg(feature = "d3d12-cross-process")]
    pub fn export_remote(&mut self) -> Result<crate::d3d12_protocol::GpuHello, String> {
        self.export_remote_epoch(1)
    }
    #[cfg(feature = "d3d12-cross-process")]
    pub(crate) fn export_remote_epoch(
        &mut self,
        epoch: u64,
    ) -> Result<crate::d3d12_protocol::GpuHello, String> {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(error)?;
        let session = crate::SessionIdentity {
            target: crate::TargetIdentity {
                pid: std::process::id(),
                creation_time: (u64::from(created.dwHighDateTime) << 32)
                    | u64::from(created.dwLowDateTime),
                window: self.presentation.as_ref().unwrap().chains[0].window.0 .0 as u64,
            },
            generation: 1,
            epoch,
        };
        let (device, _) = self.capture.device_queue();
        self.access_gpu(|gpu| gpu.export_hello(&device, session))
    }
    #[cfg(feature = "d3d12-cross-process")]
    pub fn poll_remote(&mut self) -> Result<Option<crate::FrameHeader>, String> {
        let result = self.access_gpu(|gpu| gpu.export_frame());
        if result.is_err() {
            self.cancel_capture();
        }
        result
    }
    #[cfg(feature = "d3d12-cross-process")]
    pub fn acknowledge_remote(
        &mut self,
        session: crate::SessionIdentity,
        slot: u32,
        sequence: u64,
    ) -> Result<(), String> {
        self.access_gpu(|gpu| gpu.remote_ack(session, slot, sequence))
    }
    #[cfg(feature = "d3d12-gpu-fixture")]
    pub fn poll_gpu(&mut self) -> Result<Option<crate::d3d12_gpu_bridge::GpuInteropFrame>, String> {
        let result = self.access_gpu(|gpu| gpu.poll());
        if result.is_err() {
            self.cancel_capture();
        }
        result
    }
    pub fn stats(&self) -> PresentStats {
        self.stats
    }
    pub fn selected_dimensions(&self) -> Result<(u32, u32), String> {
        let desc = unsafe {
            self.presentation.as_ref().unwrap().chains[0]
                .chain
                .GetDesc1()
        }
        .map_err(error)?;
        Ok((desc.Width, desc.Height))
    }

    /// Test control: cover the selected red HWND with our unrelated blue HWND.
    /// Never moves or raises any external application's window.
    pub fn cover_selected(&mut self) -> Result<(), String> {
        let chains = &self.presentation.as_ref().unwrap().chains;
        unsafe {
            let mut rect = RECT::default();
            GetWindowRect(chains[0].window.0, &mut rect).map_err(error)?;
            SetWindowPos(
                chains[1].window.0,
                Some(HWND_TOP),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE,
            )
            .map_err(error)?;
        }
        Ok(())
    }

    /// Owner-only window-state control; no display mode or external HWND changes.
    #[cfg(feature = "d3d12-cross-process")]
    pub fn window_action(
        &mut self,
        action: crate::d3d12_protocol::WindowAction,
    ) -> Result<u64, String> {
        use crate::d3d12_protocol::WindowAction;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        let window = self.presentation.as_ref().unwrap().chains[0].window.0;
        unsafe {
            match action {
                WindowAction::Minimize => {
                    let _ = ShowWindow(window, SW_MINIMIZE);
                    self.selected_color = [0., 1., 0., 1.];
                }
                WindowAction::ExcludeCapture => {
                    SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE).map_err(error)?;
                }
                WindowAction::Restore => {
                    let _ = ShowWindow(window, SW_RESTORE);
                    self.selected_color = [1., 0., 1., 1.];
                }
                WindowAction::Borderless => {
                    if self.windowed.is_none() {
                        let mut rect = RECT::default();
                        GetWindowRect(window, &mut rect).map_err(error)?;
                        self.windowed = Some((GetWindowLongPtrW(window, GWL_STYLE), rect));
                    }
                    let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
                    let mut info = MONITORINFO {
                        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                        ..Default::default()
                    };
                    if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                        return Err("owned monitor geometry unavailable".into());
                    }
                    let _ =
                        SetWindowLongPtrW(window, GWL_STYLE, (WS_POPUP | WS_VISIBLE).0 as isize);
                    let r = info.rcMonitor;
                    SetWindowPos(
                        window,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    )
                    .map_err(error)?;
                    self.selected_color = [1., 1., 0., 1.];
                }
                WindowAction::Windowed => {
                    let (style, r) = self
                        .windowed
                        .take()
                        .ok_or("owned window was not borderless")?;
                    let _ = SetWindowLongPtrW(window, GWL_STYLE, style);
                    SetWindowPos(
                        window,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    )
                    .map_err(error)?;
                    self.selected_color = [1., 0., 0., 1.];
                }
            }
            let mut flags = u64::from(IsIconic(window).as_bool());
            if GetWindowLongPtrW(window, GWL_STYLE) & WS_POPUP.0 as isize != 0 {
                flags |= 2;
            }
            let mut r = RECT::default();
            GetWindowRect(window, &mut r).map_err(error)?;
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(
                MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
                &mut info,
            )
            .as_bool()
                && r == info.rcMonitor
            {
                flags |= 4;
            }
            Ok(flags)
        }
    }

    /// Render both chains, copy only the selected source, then forward Present.
    /// Busy GPU renderer slots and full capture rings skip work without fence waits.
    pub fn step(&mut self) -> Result<(), String> {
        if self.failed {
            return Err("owned presentation fixture failed".into());
        }
        let result = self.step_inner();
        if result.is_err() {
            self.failed = true;
            self.cancel_capture();
        }
        result
    }
    fn step_inner(&mut self) -> Result<(), String> {
        let presentation = self.presentation.as_mut().unwrap();
        for chain in &presentation.chains {
            chain.window.pump()?;
        }
        for index in 0..2 {
            let chain = &mut presentation.chains[index];
            let current = unsafe { chain.chain.GetCurrentBackBufferIndex() } as usize;
            let slot = chain
                .slots
                .get_mut(current)
                .ok_or("unexpected D3D12 backbuffer index")?;
            let completed = unsafe { presentation.fence.GetCompletedValue() };
            if completed == u64::MAX {
                return Err("owned renderer device removed".into());
            }
            if completed < slot.fence_value {
                continue;
            }
            let value = presentation
                .last_signal
                .checked_add(1)
                .filter(|v| *v < u64::MAX)
                .ok_or("renderer fence exhausted")?;
            unsafe {
                slot.allocator.Reset().map_err(error)?;
                slot.list.Reset(&slot.allocator, None).map_err(error)?;
                barrier(
                    &slot.list,
                    &slot.target,
                    D3D12_RESOURCE_STATE_PRESENT,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                );
                slot.list.ClearRenderTargetView(
                    slot.rtv.GetCPUDescriptorHandleForHeapStart(),
                    if index == 0 {
                        &self.selected_color
                    } else {
                        &[0., 0., 1., 1.]
                    },
                    None,
                );
                barrier(
                    &slot.list,
                    &slot.target,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                    D3D12_RESOURCE_STATE_PRESENT,
                );
                slot.list.Close().map_err(error)?;
                let command: ID3D12CommandList = slot.list.cast().map_err(error)?;
                presentation.uncertain = true;
                presentation.queue.ExecuteCommandLists(&[Some(command)]);
            }
            #[cfg(feature = "d3d12-present-hook-fixture")]
            let manual_capture = self.present_hook.is_none();
            #[cfg(not(feature = "d3d12-present-hook-fixture"))]
            let manual_capture = true;
            if index == 0 && self.capture_open && manual_capture {
                #[cfg(feature = "d3d12-gpu-fixture")]
                let captured = match self.gpu.as_mut() {
                    Some(gpu) => gpu.submit(&slot.target)?,
                    None => self.capture.copy_owned_present_target(&slot.target)?,
                };
                #[cfg(not(feature = "d3d12-gpu-fixture"))]
                let captured = self.capture.copy_owned_present_target(&slot.target)?;
                if captured {
                    self.stats.captured += 1;
                } else {
                    self.stats.dropped_capture += 1;
                }
            }
            unsafe { chain.chain.Present(0, DXGI_PRESENT(0)) }
                .ok()
                .map_err(error)?;
            // Fence the entire iteration, including the copy and queued Present,
            // so control-side ResizeBuffers does not retire an earlier boundary.
            unsafe { presentation.queue.Signal(&presentation.fence, value) }.map_err(error)?;
            presentation.last_signal = value;
            slot.fence_value = value;
            presentation.uncertain = false;
            if index == 0 {
                self.stats.selected += 1;
            } else {
                self.stats.unrelated += 1;
            }
        }
        #[cfg(feature = "d3d12-present-hook-fixture")]
        if let Some(hook) = &self.present_hook {
            let (_, _, captured, dropped) = hook.counts();
            self.stats.captured = captured;
            self.stats.dropped_capture = dropped;
        }
        Ok(())
    }
    pub fn poll(&mut self) -> Result<Option<FixtureFrame>, String> {
        #[cfg(feature = "d3d12-present-hook-fixture")]
        if self.present_hook.is_some() {
            return Err("Present hook cannot fall back to CPU polling".into());
        }
        #[cfg(feature = "d3d12-gpu-fixture")]
        if self.gpu.is_some() {
            return Err("GPU fixture cannot fall back to CPU polling".into());
        }
        let result = self.capture.poll();
        if result.is_err() {
            self.cancel_capture();
        }
        result
    }
    pub fn cancel_capture(&mut self) {
        #[cfg(feature = "d3d12-factory-hook-fixture")]
        if let Some(hook) = &self.factory_hook {
            hook.close();
        }
        if self.capture_open {
            if let Some(presentation) = &mut self.presentation {
                if let Some(token) = presentation.chains[0].association {
                    let _ = presentation.associations.retire(token);
                }
            }
        }
        self.capture_open = false;
        self.capture.cancel();
        #[cfg(feature = "d3d12-gpu-fixture")]
        if let Some(gpu) = &mut self.gpu {
            gpu.cancel();
        }
        #[cfg(feature = "d3d12-present-hook-fixture")]
        if let Some(hook) = &self.present_hook {
            hook.cancel();
        }
    }

    #[cfg(feature = "d3d12-auto-resize-fixture")]
    fn probe_rejected_resize(&self) -> Result<(), String> {
        let presentation = self
            .presentation
            .as_ref()
            .ok_or("missing fault-probe renderer")?;
        if self.capture_open || !drain(presentation) {
            return Err("resize fault probe requires drained render-only fixture".into());
        }
        // Deliberately retain the application's own back-buffer slots. DXGI
        // rejects ResizeBuffers1 while those references exist. No GPU work is
        // submitted on the invalid call; the original renderer remains usable.
        let queue: windows::core::IUnknown = presentation.queue.cast().map_err(error)?;
        let queues = [Some(queue.clone()), Some(queue)];
        let result = unsafe {
            presentation.chains[0].chain.ResizeBuffers1(
                2,
                640,
                360,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG(0),
                [1u32, 1].as_ptr(),
                queues.as_ptr(),
            )
        };
        if result.is_ok() {
            return Err("retained-buffer resize unexpectedly succeeded".into());
        }
        eprintln!(
            "owned expected retained-buffer ResizeBuffers1 rejection: {}",
            result.unwrap_err()
        );
        Ok(())
    }

    /// Explicit control operation. Resizing stops capture permanently; rendering
    /// can continue at the new size, but stale-sized capture is never republished.
    pub fn resize_selected(&mut self, width: u32, height: u32) -> Result<(), String> {
        if self.failed || width == 0 || height == 0 || width > 1920 || height > 1080 {
            return Err("invalid owned swap-chain resize".into());
        }
        self.cancel_capture();
        let result = (|| {
            #[cfg(feature = "d3d12-present-hook-fixture")]
            if let Some(hook) = &self.present_hook {
                hook.access(|gpu| gpu.release_sources())?;
            }
            #[cfg(feature = "d3d12-gpu-fixture")]
            if let Some(gpu) = &mut self.gpu {
                gpu.release_sources()?;
            }
            if !self.capture.drain() || !drain(self.presentation.as_ref().unwrap()) {
                return Err("resize drain uncertain".into());
            }
            self.capture.release_drained_sources()?;
            let presentation = self.presentation.as_mut().unwrap();
            let chain = &mut presentation.chains[0];
            chain.slots.clear(); // release resources AND recorded command lists
            #[cfg(feature = "d3d12-late-resize-fixture")]
            let late = std::env::args().any(|arg| arg == "--late-d3d12");
            #[cfg(not(feature = "d3d12-late-resize-fixture"))]
            let late = false;
            unsafe {
                if late {
                    let queue: windows::core::IUnknown =
                        presentation.queue.cast().map_err(error)?;
                    let second = if std::env::args().any(|arg| arg == "--late-distinct-queues") {
                        let other: ID3D12CommandQueue = presentation
                            .device
                            .CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                                ..Default::default()
                            })
                            .map_err(error)?;
                        other.cast().map_err(error)?
                    } else {
                        queue.clone()
                    };
                    let queues = [Some(queue), Some(second)];
                    chain.chain.ResizeBuffers1(
                        2,
                        width,
                        height,
                        DXGI_FORMAT_B8G8R8A8_UNORM,
                        DXGI_SWAP_CHAIN_FLAG(0),
                        [1u32, 1].as_ptr(),
                        queues.as_ptr(),
                    )
                } else {
                    chain.chain.ResizeBuffers(
                        2,
                        width,
                        height,
                        DXGI_FORMAT_B8G8R8A8_UNORM,
                        DXGI_SWAP_CHAIN_FLAG(0),
                    )
                }
            }
            .map_err(error)?;
            chain.slots = Chain::slots(&presentation.device, &chain.chain)?;
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    #[cfg(feature = "d3d12-rearm-fixture")]
    pub(crate) fn resize_render_only(&mut self, width: u32, height: u32) -> Result<(), String> {
        if self.capture_open {
            return Err("resize control requires render-only fixture".into());
        }
        self.resize_selected(width, height)?;
        self.selected_color = if width == 640 {
            [0., 1., 0., 1.]
        } else {
            [0., 0., 1., 1.]
        };
        Ok(())
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub fn probe_pending_media_removal(warp: bool, expose_consumer: bool) -> Result<(), String> {
        let budget = std::sync::Arc::new(crate::local_gpu_bridge::GpuPoolBudget::default());
        let mut fixture = Self::new_gpu(warp, 318, 178, budget.clone())?;
        let mut retained = None;
        if expose_consumer {
            let deadline = Instant::now() + Duration::from_secs(5);
            while retained.is_none() {
                fixture.step()?;
                if let Some(frame) = fixture.poll_gpu()? {
                    retained = Some(frame.publication);
                }
                if Instant::now() >= deadline {
                    return Err("consumer exposure probe deadline".into());
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let gpu = fixture.gpu.as_mut().unwrap();
            if !gpu.drain() {
                return Err("pre-loss exposure producer did not drain".into());
            }
            while gpu.poll()?.is_some() {}
            if !gpu.drain() {
                return Err("pre-loss exposure consumer did not drain".into());
            }
        }
        let presentation = fixture
            .presentation
            .as_ref()
            .ok_or("missing removal probe renderer")?;
        let removable: ID3D12Device5 = presentation.device.cast().map_err(error)?;
        let gate: ID3D12Fence =
            unsafe { presentation.device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.map_err(error)?;
        let pending: ID3D12Fence =
            unsafe { presentation.device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.map_err(error)?;
        struct ReleaseWait(ID3D12Fence);
        impl Drop for ReleaseWait {
            fn drop(&mut self) {
                let _ = unsafe { self.0.Signal(1) };
            }
        }
        let release = ReleaseWait(gate.clone());
        let source: ID3D12Resource =
            unsafe { presentation.chains[0].chain.GetBuffer(0) }.map_err(error)?;
        unsafe { presentation.queue.Wait(&gate, 1) }.map_err(error)?;
        let gpu = fixture.gpu.as_mut().ok_or("missing probe GPU bridge")?;
        let before = gpu.stats();
        if !gpu.submit(&source)? {
            return Err("probe capture copy was not submitted".into());
        }
        unsafe { presentation.queue.Signal(&pending, 1) }.map_err(error)?;
        if unsafe { pending.GetCompletedValue() } != 0
            || gpu.stats().submitted != before.submitted + 1
            || gpu.poll()?.is_some()
        {
            return Err("probe capture work was not provably pending".into());
        }
        unsafe { removable.RemoveDevice() };
        if unsafe { presentation.device.GetDeviceRemovedReason() }.is_ok()
            || unsafe { pending.GetCompletedValue() } != u64::MAX
            || gpu.poll().is_ok()
        {
            return Err("removed pending media was not rejected".into());
        }
        if gpu.stats().gpu_copies_to_output != before.gpu_copies_to_output
            || gpu.stats().cpu_readbacks != 0
        {
            return Err("removed pending media reached consumer".into());
        }
        if !gpu.removed_graph_releasable() {
            return Err("removed graph retirement policy mismatch".into());
        }
        let started = Instant::now();
        fixture.cancel_capture();
        drop(release);
        drop(source);
        drop(pending);
        drop(gate);
        drop(removable);
        drop(fixture);
        if started.elapsed() > Duration::from_secs(8) {
            return Err("pending-media removal teardown exceeded bound".into());
        }
        if let Some(retained) = retained.as_ref() {
            let pixels = gpu_windows::BgraReadback::default()
                .read_scaled(retained.surface(), 64, 36)
                .map_err(error)?;
            if !pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [0, 0, 255, 255])
            {
                return Err("retained exposed local pixels changed after removal".into());
            }
        }
        // Creation on this same process/adapter fails if any removed singleton
        // graph leaked. Reuse the exact local budget rather than allocating around
        // retained old surfaces. Local work is independently retired; no remote
        // media was exported by this probe.
        let mut recovered = Self::new_gpu(warp, 318, 178, budget)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut frames = 0;
        while frames < 3 {
            recovered.step()?;
            if let Some(frame) = recovered.poll_gpu()? {
                let bytes = gpu_windows::BgraReadback::default()
                    .read_scaled(frame.publication.surface(), 64, 36)
                    .map_err(error)?;
                if !bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| *pixel == [0, 0, 255, 255])
                {
                    return Err("recreated capture pixels did not match owned red target".into());
                }
                frames += 1;
            }
            if Instant::now() >= deadline {
                return Err("recreated capture did not publish by deadline".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if let Some(retained) = retained.as_ref() {
            let pixels = gpu_windows::BgraReadback::default()
                .read_scaled(retained.surface(), 64, 36)
                .map_err(error)?;
            if !pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [0, 0, 255, 255])
            {
                return Err("old local pixels changed after recreation".into());
            }
        }
        eprintln!("PASS: owned pending capture copy rejected after explicit removal; independently retired local consumer, same-process/same-budget recreation and three red GPU frames; retained old pixels preserved when exposed. Not remote-consumer recovery or TDR.");
        Ok(())
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn remote_epoch_primed(&mut self) -> Result<bool, String> {
        self.access_gpu(|gpu| Ok(gpu.remote_primed()))
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn reconstruct_remote_loss(
        mut self,
        session: crate::SessionIdentity,
        width: u32,
        height: u32,
        budget: std::sync::Arc<crate::local_gpu_bridge::GpuPoolBudget>,
    ) -> Result<Self, String> {
        let windows = self.removed_replacement_windows()?;
        self.access_gpu(|gpu| {
            gpu.approve_remote_retirement(session)?;
            gpu.release_sources()
        })?;
        self.gpu.take();
        drop(self); // Every old producer/application graph must go before creation.
        let mut replacement = Self::renderer_with_windows(false, width, height, true, windows)?;
        let presentation = replacement.presentation.as_ref().unwrap();
        let (device, queue) = presentation
            .associations
            .selected(&presentation.chains[0].chain)?;
        replacement.gpu = Some(crate::d3d12_gpu_bridge::D3d12GpuBridge::new(
            &device, &queue, width, height, budget, false,
        )?);
        replacement.selected_color = if width == 640 {
            [0., 1., 0., 1.]
        } else {
            [0., 0., 1., 1.]
        };
        replacement.cover_selected()?;
        Ok(replacement)
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn trigger_owned_device_loss(
        &mut self,
    ) -> Result<crate::d3d12_protocol::DeviceLossReason, String> {
        #[cfg(feature = "d3d12-present-hook-fixture")]
        if self.present_hook.is_some() {
            return Err("unannounced loss probe cannot use a Present hook".into());
        }
        let presentation = self
            .presentation
            .as_ref()
            .ok_or("missing owned loss graph")?;
        let removable: ID3D12Device5 = presentation.device.cast().map_err(error)?;
        let gate: ID3D12Fence =
            unsafe { presentation.device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.map_err(error)?;
        let pending: ID3D12Fence =
            unsafe { presentation.device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.map_err(error)?;
        struct ReleaseWait(ID3D12Fence);
        impl Drop for ReleaseWait {
            fn drop(&mut self) {
                let _ = unsafe { self.0.Signal(1) };
            }
        }
        let release = ReleaseWait(gate.clone());
        unsafe { presentation.queue.Wait(&gate, 1) }.map_err(error)?;
        unsafe { presentation.queue.Signal(&pending, 1) }.map_err(error)?;
        if unsafe { pending.GetCompletedValue() } != 0 {
            return Err("owned loss probe was not pending".into());
        }
        unsafe { removable.RemoveDevice() };
        let reason = unsafe { presentation.device.GetDeviceRemovedReason() }
            .err()
            .ok_or("owned device did not become removed")?;
        if unsafe { pending.GetCompletedValue() } != u64::MAX {
            return Err("owned loss fence did not report removal".into());
        }
        let reason = crate::d3d12_protocol::DeviceLossReason::from_hresult(reason.code().0 as u32)
            .ok_or("unsupported owned device-loss reason")?;
        drop(release);
        self.cancel_capture();
        Ok(reason)
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn pump_paused_windows(&self) -> Result<(), String> {
        for chain in &self
            .presentation
            .as_ref()
            .ok_or("paused renderer graph missing")?
            .chains
        {
            chain.window.pump()?;
        }
        Ok(())
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn removed_replacement_windows(&self) -> Result<[Rc<Window>; 2], String> {
        let presentation = self
            .presentation
            .as_ref()
            .ok_or("removed renderer graph missing")?;
        if self.capture_open
            || !self.capture.device_removed()
            || unsafe { presentation.device.GetDeviceRemovedReason() }.is_ok()
        {
            return Err(
                "removed replacement requires closed capture and confirmed owned removal".into(),
            );
        }
        Ok([
            presentation.chains[0].window.clone(),
            presentation.chains[1].window.clone(),
        ])
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn replacement_windows(&self) -> Result<[Rc<Window>; 2], String> {
        let presentation = self.presentation.as_ref().ok_or("missing renderer")?;
        if self.capture_open || self.failed || !self.capture.drain() || !drain(presentation) {
            return Err("replacement requires healthy drained render-only fixture".into());
        }
        Ok([
            presentation.chains[0].window.clone(),
            presentation.chains[1].window.clone(),
        ])
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn quiescent_removal_device(&self) -> Result<ID3D12Device5, String> {
        let presentation = self.presentation.as_ref().ok_or("missing renderer")?;
        if self.capture_open || self.failed || !self.capture.drain() || !drain(presentation) {
            return Err("device-removal probe requires a healthy drained owned renderer".into());
        }
        presentation.device.cast().map_err(error)
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn replace_render_only(
        warp: bool,
        width: u32,
        height: u32,
        windows: [Rc<Window>; 2],
    ) -> Result<Self, String> {
        let mut renderer = Self::renderer_with_windows(warp, width, height, false, windows)?;
        renderer.selected_color = if width == 640 {
            [0., 1., 0., 1.]
        } else {
            [0., 0., 1., 1.]
        };
        Ok(renderer)
    }
}
fn drain(presentation: &Presentation) -> bool {
    if presentation.uncertain {
        return false;
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let value = unsafe { presentation.fence.GetCompletedValue() };
        if value != u64::MAX && value >= presentation.last_signal {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
impl Drop for D3d12PresentFixture {
    fn drop(&mut self) {
        self.cancel_capture();
        let presentation = self.presentation.take().unwrap();
        #[cfg(feature = "d3d12-gpu-fixture")]
        let removed_gpu_releasable = self
            .gpu
            .as_ref()
            .is_none_or(|gpu| gpu.removed_graph_releasable());
        #[cfg(not(feature = "d3d12-gpu-fixture"))]
        let removed_gpu_releasable = true;
        #[cfg(feature = "d3d12-present-hook-fixture")]
        let no_hooks = self.present_hook.is_none();
        #[cfg(not(feature = "d3d12-present-hook-fixture"))]
        let no_hooks = true;
        if no_hooks
            && removed_gpu_releasable
            && self.capture.device_removed()
            && unsafe { presentation.device.GetDeviceRemovedReason() }.is_err()
        {
            return;
        }
        #[cfg(feature = "d3d12-gpu-fixture")]
        let gpu_drained = self.gpu.as_ref().is_none_or(|gpu| gpu.drain());
        #[cfg(feature = "d3d12-present-hook-fixture")]
        let gpu_drained = gpu_drained && self.present_hook.as_ref().is_none_or(|hook| hook.drain());
        #[cfg(not(feature = "d3d12-gpu-fixture"))]
        let gpu_drained = true;
        if gpu_drained && self.capture.drain() && drain(&presentation) {
            return;
        }
        eprintln!("owned D3D12 presentation completion uncertain; retaining windows/GPU objects until exit");
        self.capture.retain_until_exit();
        std::mem::forget(presentation);
    }
}
