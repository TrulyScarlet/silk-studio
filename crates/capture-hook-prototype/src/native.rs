//! Real Present detour for the explicitly opted-in, self-owned fixture only.
//! Not safe to attach to a game: caller quiescence is a prototype precondition.

use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex, Weak};

use gpu_windows::{GpuDeviceContext, GpuTextureSlot};
use minhook::MinHook;
use windows::core::{Interface, HRESULT};
use windows::Win32::Foundation::{FILETIME, HWND};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11Multithread, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain, DXGI_PRESENT, DXGI_PRESENT_TEST};
use windows::Win32::System::Performance::QueryPerformanceCounter;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};

use crate::admission_gate::AdmissionGate;
use crate::{
    FrameHeader, FrameSlots, SessionIdentity, TargetIdentity, MAX_HEIGHT, MAX_WIDTH,
    PROTOCOL_VERSION, SLOT_COUNT,
};

type Present = unsafe extern "system" fn(*mut c_void, u32, DXGI_PRESENT) -> HRESULT;
static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ATTACHED: AtomicBool = AtomicBool::new(false);
static LIFECYCLE: Mutex<()> = Mutex::new(());
// Outside-Present lifetime accounting: at most one ring's worth of old leases
// may survive while allocating the next three-surface ring.
static RETIRED: Mutex<Vec<Weak<GpuTextureSlot>>> = Mutex::new(Vec::new());

thread_local! {
    static STATE: RefCell<Option<TapState>> = const { RefCell::new(None) };
}

#[derive(Default)]
pub struct TapStats {
    pub intercepted: AtomicU64,
    pub published: AtomicU64,
    pub dropped: AtomicU64,
    pub invalidated: AtomicBool,
    pub capture_gate: Arc<AdmissionGate>,
    pub forwarded_originals: AtomicU64,
    pub original_present_failures: AtomicU64,
    pub reported_present_failures: AtomicU64,
}

impl TapStats {
    /// Request media cancellation without disabling/freeing the detour.
    /// A drained capture gate is NOT a trampoline/DLL-unload safety barrier.
    pub fn cancel_capture(&self) {
        self.capture_gate.close();
        self.invalidated.store(true, Ordering::Release);
    }
}

pub struct CapturedFrame {
    pub header: FrameHeader,
    /// Consumers must retain this Arc for as long as they use the GPU texture.
    pub surface: Arc<GpuTextureSlot>,
}

pub(crate) struct TapState {
    chain: IDXGISwapChain,
    device: Arc<GpuDeviceContext>,
    dimensions: (u32, u32),
    slots: FrameSlots<GpuTextureSlot>,
    sender: SyncSender<CapturedFrame>,
    session: SessionIdentity,
    sequence: u64,
    stats: Arc<TapStats>,
    #[cfg(feature = "fixture-probes")]
    probe: Option<Arc<crate::fixture_probe::FixtureProbe>>,
}

impl TapState {
    pub(crate) fn capture(&mut self) {
        if self.stats.invalidated.load(Ordering::Relaxed) {
            return;
        }
        let Some(_permit) = self.stats.capture_gate.try_enter() else {
            return;
        };
        #[cfg(feature = "fixture-probes")]
        if self.probe.as_ref().is_some_and(|probe| !probe.checkpoint()) {
            self.invalidate();
            return;
        }
        if self.stats.capture_gate.is_closed() {
            return;
        }
        // No allocation, logging, encoding, disk access or consumer waits here.
        let buffer: ID3D11Texture2D = match unsafe { self.chain.GetBuffer(0) } {
            Ok(buffer) => buffer,
            Err(_) => {
                self.invalidate();
                return;
            }
        };
        let mut description = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            buffer.GetDesc(&mut description);
        }
        if description.Width != self.dimensions.0
            || description.Height != self.dimensions.1
            || description.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            || description.SampleDesc.Count != 1
        {
            self.invalidate();
            return;
        }
        let Some((slot, surface)) = self.slots.try_acquire() else {
            self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let mut ticks = 0;
        if unsafe { QueryPerformanceCounter(&mut ticks) }.is_err() {
            self.invalidate();
            return;
        }
        let Some(sequence) = self.sequence.checked_add(1) else {
            self.invalidate();
            return;
        };
        self.sequence = sequence;
        unsafe {
            self.device
                .context()
                .CopyResource(surface.texture(), &buffer);
        }
        let frame = CapturedFrame {
            header: FrameHeader {
                version: PROTOCOL_VERSION,
                session: self.session,
                slot: slot as u32,
                sequence,
                qpc_ticks: ticks,
                width: description.Width,
                height: description.Height,
            },
            surface,
        };
        // Cancellation can overlap GPU work. The consumer closes its own
        // admission and discards pre-stop queued frames; no unhook occurs here.
        if self.stats.capture_gate.is_closed() {
            return;
        }
        match self.sender.try_send(frame) {
            Ok(()) => {
                self.stats.published.fetch_add(1, Ordering::Relaxed);
            }
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => self.invalidate(),
        }
    }

    fn invalidate(&self) {
        self.stats.cancel_capture();
    }
}

/// Allocate media state outside Present. Does not install or remove any hook.
/// # Safety
/// The caller must own a valid chain lease and arrange protected immediate-context
/// use; this does not make arbitrary D3D11 application lifecycles safe.
#[cfg(feature = "resident-hooks")]
pub(crate) unsafe fn prepare_capture(
    chain: &IDXGISwapChain,
    session: SessionIdentity,
) -> Result<(TapState, Receiver<CapturedFrame>, Arc<TapStats>), String> {
    let description = chain.GetDesc().map_err(|error| error.to_string())?;
    if own_window_identity(description.OutputWindow)? != session.target
        || crate::SessionGate::new(session).is_none()
    {
        return Err("target identity or session generation did not match".into());
    }
    let buffer: ID3D11Texture2D = chain.GetBuffer(0).map_err(|error| error.to_string())?;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    buffer.GetDesc(&mut desc);
    if desc.Width == 0
        || desc.Height == 0
        || desc.Width > MAX_WIDTH
        || desc.Height > MAX_HEIGHT
        || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
        || desc.SampleDesc.Count != 1
    {
        return Err("prototype requires bounded, single-sampled BGRA8 D3D11 backbuffers".into());
    }
    let device: ID3D11Device = chain.GetDevice().map_err(|error| error.to_string())?;
    if device.GetCreationFlags()
        & windows::Win32::Graphics::Direct3D11::D3D11_CREATE_DEVICE_SINGLETHREADED.0
        != 0
    {
        return Err("single-threaded D3D11 devices are unsupported".into());
    }
    let context = device
        .GetImmediateContext()
        .map_err(|error| error.to_string())?;
    let multithread: ID3D11Multithread = context.cast().map_err(|error| error.to_string())?;
    let _ = multithread.SetMultithreadProtected(true);
    let device = Arc::new(GpuDeviceContext::new(device, context));
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
    desc.CPUAccessFlags = 0;
    desc.MiscFlags = 0;
    let mut surfaces = Vec::with_capacity(SLOT_COUNT);
    for _ in 0..SLOT_COUNT {
        let mut texture = None;
        device
            .device()
            .CreateTexture2D(&desc, None, Some(&mut texture))
            .map_err(|error| error.to_string())?;
        surfaces.push(GpuTextureSlot::new(
            texture.ok_or("D3D11 returned a null capture texture")?,
            device.clone(),
        ));
    }
    let surfaces: [GpuTextureSlot; SLOT_COUNT] =
        surfaces.try_into().map_err(|_| "invalid slot count")?;
    let stats = Arc::new(TapStats::default());
    let (sender, receiver) = sync_channel(SLOT_COUNT);
    Ok((
        TapState {
            chain: chain.clone(),
            device,
            dimensions: (desc.Width, desc.Height),
            slots: FrameSlots::new(surfaces),
            sender,
            session,
            sequence: 0,
            stats: stats.clone(),
            #[cfg(feature = "fixture-probes")]
            probe: None,
        },
        receiver,
        stats,
    ))
}

#[cfg(feature = "resident-hooks")]
impl TapState {
    pub(crate) fn matches(&self, chain: *mut c_void) -> bool {
        self.chain.as_raw() == chain
    }
    pub(crate) fn present_failed(&self) {
        self.invalidate();
    }
}

unsafe extern "system" fn detour(
    chain: *mut c_void,
    interval: u32,
    flags: DXGI_PRESENT,
) -> HRESULT {
    // Attach/detach quiescence guarantees the trampoline outlives every call.
    let original = ORIGINAL.load(Ordering::Acquire);
    if original.is_null() {
        return windows::Win32::Foundation::E_FAIL;
    }
    let _ = STATE.try_with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut() {
            if let Some(state) = state.as_mut().filter(|state| state.chain.as_raw() == chain) {
                state.stats.intercepted.fetch_add(1, Ordering::Relaxed);
                if flags.0 & DXGI_PRESENT_TEST.0 == 0 {
                    state.capture();
                }
            }
        }
    });
    let original: Present = std::mem::transmute(original);
    let result = original(chain, interval, flags);
    STATE
        .try_with(|cell| {
            if let Ok(state) = cell.try_borrow() {
                if let Some(state) = state.as_ref().filter(|state| state.chain.as_raw() == chain) {
                    state
                        .stats
                        .forwarded_originals
                        .fetch_add(1, Ordering::Relaxed);
                    if result.is_err() {
                        state
                            .stats
                            .original_present_failures
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    #[cfg(feature = "fixture-probes")]
                    let result = state
                        .probe
                        .as_ref()
                        .map(|probe| probe.take_present_result())
                        .filter(|value| *value != 0)
                        .map(HRESULT)
                        .unwrap_or(result);
                    if result.is_err() {
                        state
                            .stats
                            .reported_present_failures
                            .fetch_add(1, Ordering::Relaxed);
                        state.invalidate();
                    }
                    return result;
                }
            }
            result
        })
        .unwrap_or(result)
}

/// Deliberately !Send/!Sync: attach, rendering, and teardown are one-thread-only.
pub struct PresentTap {
    target: *mut c_void,
    detached: bool,
    _owner: PhantomData<Rc<()>>,
}

pub fn own_window_identity(window: HWND) -> Result<TargetIdentity, String> {
    let mut pid = 0;
    unsafe {
        if !IsWindow(Some(window)).as_bool() {
            return Err("target window does not exist".into());
        }
        GetWindowThreadProcessId(window, Some(&mut pid));
    }
    if pid != std::process::id() {
        return Err("prototype accepts only its own test application's window".into());
    }
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
    .map_err(|error| error.to_string())?;
    Ok(TargetIdentity {
        pid,
        creation_time: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        window: window.0 as u64,
    })
}

impl PresentTap {
    /// Test-only instrumentation. Installs only on the attach/render owner
    /// thread, before rendering resumes. This is never a production feature.
    #[cfg(feature = "fixture-probes")]
    pub fn install_fixture_probe(
        &mut self,
        probe: Arc<crate::fixture_probe::FixtureProbe>,
    ) -> Result<(), String> {
        STATE.with(|cell| {
            let mut state = cell
                .try_borrow_mut()
                .map_err(|_| "capture callback is busy")?;
            state.as_mut().ok_or("no attached capture state")?.probe = Some(probe);
            Ok(())
        })
    }
    /// Attach only to the controlled, self-owned D3D11 test swap chain.
    ///
    /// # Safety
    /// All calls to this implementation of Present in this process must be
    /// quiescent during attach/detach. All captured Presents must run on this
    /// thread, and no asynchronous Present call may outlive the tap. These
    /// preconditions are NOT established for arbitrary applications or games.
    pub unsafe fn attach(
        chain: &IDXGISwapChain,
        session: SessionIdentity,
    ) -> Result<(Self, Receiver<CapturedFrame>, Arc<TapStats>), String> {
        let _lifecycle = LIFECYCLE
            .lock()
            .map_err(|_| "hook lifecycle lock poisoned")?;
        if ATTACHED.load(Ordering::Acquire) {
            return Err("a prototype Present tap is already attached".into());
        }
        {
            let mut retired = RETIRED
                .lock()
                .map_err(|_| "retired surface lock poisoned")?;
            retired.retain(|surface| surface.strong_count() != 0);
            if retired.len() > SLOT_COUNT {
                return Err("old frame leases exhaust the bounded epoch budget; release them before reattaching".into());
            }
        }
        let description = chain.GetDesc().map_err(|error| error.to_string())?;
        if own_window_identity(description.OutputWindow)? != session.target
            || crate::SessionGate::new(session).is_none()
        {
            return Err("target identity or session generation did not match".into());
        }
        let buffer: ID3D11Texture2D = chain.GetBuffer(0).map_err(|error| error.to_string())?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        buffer.GetDesc(&mut desc);
        if desc.Width == 0
            || desc.Height == 0
            || desc.Width > MAX_WIDTH
            || desc.Height > MAX_HEIGHT
            || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            || desc.SampleDesc.Count != 1
        {
            return Err(
                "prototype requires bounded, single-sampled BGRA8 D3D11 backbuffers".into(),
            );
        }
        let device: ID3D11Device = chain.GetDevice().map_err(|error| error.to_string())?;
        let context = device
            .GetImmediateContext()
            .map_err(|error| error.to_string())?;
        let multithread: ID3D11Multithread = context.cast().map_err(|error| error.to_string())?;
        let _ = multithread.SetMultithreadProtected(true);
        let device = Arc::new(GpuDeviceContext::new(device, context));
        desc.Usage = D3D11_USAGE_DEFAULT;
        desc.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
        desc.CPUAccessFlags = 0;
        desc.MiscFlags = 0;
        let mut surfaces = Vec::with_capacity(SLOT_COUNT);
        for _ in 0..SLOT_COUNT {
            let mut texture = None;
            device
                .device()
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .map_err(|error| error.to_string())?;
            let texture = texture.ok_or("D3D11 returned a null capture texture")?;
            surfaces.push(GpuTextureSlot::new(texture, device.clone()));
        }
        let surfaces: [GpuTextureSlot; SLOT_COUNT] =
            surfaces.try_into().map_err(|_| "invalid slot count")?;
        if ATTACHED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("a prototype Present tap is already attached".into());
        }
        let target = chain.vtable().Present as *mut c_void;
        let original = match MinHook::create_hook(target, detour as *mut c_void) {
            Ok(original) => original,
            Err(error) => {
                ATTACHED.store(false, Ordering::Release);
                return Err(error.to_string());
            }
        };
        ORIGINAL.store(original, Ordering::Release);
        let stats = Arc::new(TapStats::default());
        let (sender, receiver) = sync_channel(SLOT_COUNT);
        STATE.with(|state| {
            *state.borrow_mut() = Some(TapState {
                chain: chain.clone(),
                device,
                dimensions: (desc.Width, desc.Height),
                slots: FrameSlots::new(surfaces),
                sender,
                session,
                sequence: 0,
                stats: stats.clone(),
                #[cfg(feature = "fixture-probes")]
                probe: None,
            })
        });
        if let Err(error) = MinHook::enable_hook(target) {
            STATE.with(|state| {
                state.borrow_mut().take();
            });
            // Preserve the reservation on cleanup failure; never allow reattach
            // over a trampoline whose lifecycle is uncertain.
            if MinHook::remove_hook(target).is_ok() {
                ORIGINAL.store(std::ptr::null_mut(), Ordering::Release);
                ATTACHED.store(false, Ordering::Release);
            }
            return Err(error.to_string());
        }
        Ok((
            Self {
                target,
                detached: false,
                _owner: PhantomData,
            },
            receiver,
            stats,
        ))
    }

    pub fn detach(&mut self) -> Result<(), String> {
        if self.detached {
            return Ok(());
        }
        let _lifecycle = LIFECYCLE
            .lock()
            .map_err(|_| "hook lifecycle lock poisoned")?;
        // Safety: guaranteed by attach's lifetime-wide caller preconditions.
        unsafe {
            MinHook::disable_hook(self.target).map_err(|error| error.to_string())?;
            MinHook::remove_hook(self.target).map_err(|error| error.to_string())?;
        }
        STATE.with(|state| {
            if let Some(state) = state.borrow_mut().take() {
                // No callback can run after successful hook removal. These weak
                // tokens do not retain GPU memory but account for old leases.
                if let Ok(mut retired) = RETIRED.lock() {
                    retired.extend(state.slots.retirement_tokens());
                }
            }
        });
        ORIGINAL.store(std::ptr::null_mut(), Ordering::Release);
        ATTACHED.store(false, Ordering::Release);
        self.detached = true;
        Ok(())
    }
}

impl Drop for PresentTap {
    fn drop(&mut self) {
        // If cleanup fails, keep state/trampoline reserved until process exit;
        // never free something that the detour might still reference.
        let _ = self.detach();
    }
}
