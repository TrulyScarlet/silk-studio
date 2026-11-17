//! Resident ResizeBuffers1 observation for an exact opted-in owned fixture.
//! Successful calls with explicit queues only; no last-submitted-queue heuristic.
use crate::late_queue_contract::{
    validate_present_queues, PresentQueueIdentity, MAX_PRESENT_QUEUES,
};
use crate::TargetIdentity;
use minhook::MinHook;
use std::ffi::c_void;
#[cfg(feature = "d3d12-auto-resize-fixture")]
use std::sync::atomic::AtomicU8;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Mutex;
use windows::core::{IUnknown, Interface, HRESULT, PCWSTR};
use windows::Win32::Foundation::{E_FAIL, HMODULE};
use windows::Win32::Graphics::Direct3D12::{
    ID3D12CommandQueue, ID3D12Device, D3D12_COMMAND_LIST_TYPE_DIRECT,
};
use windows::Win32::Graphics::Dxgi::{Common::DXGI_FORMAT, IDXGISwapChain3};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
};

type Resize = unsafe extern "system" fn(
    *mut c_void,
    u32,
    u32,
    u32,
    DXGI_FORMAT,
    u32,
    *const u32,
    *const *mut c_void,
) -> HRESULT;
static ATTEMPTED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
struct State {
    target: TargetIdentity,
    offer: Option<(IDXGISwapChain3, ID3D12CommandQueue)>,
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    selected: Option<(IDXGISwapChain3, ID3D12CommandQueue)>,
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    automatic: Option<crate::automatic_resize_gate::AutomaticResizeGate>,
}
static STATE: Mutex<Option<State>> = Mutex::new(None);
#[cfg(feature = "d3d12-auto-resize-fixture")]
static AUTOMATIC: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "d3d12-auto-resize-fixture")]
static AUTO_PHASE: AtomicU8 = AtomicU8::new(0);
#[cfg(feature = "d3d12-auto-resize-fixture")]
pub(crate) fn automatic_capture_allowed() -> bool {
    !AUTOMATIC.load(Ordering::Acquire)
        || (AUTO_PHASE.load(Ordering::Acquire) == 1 && !FAILED.load(Ordering::Acquire))
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
pub(crate) fn automatic_media_suspended() -> bool {
    AUTOMATIC.load(Ordering::Acquire) && !automatic_capture_allowed()
}
#[cfg(feature = "d3d12-auto-resize-fixture")]
pub(crate) struct AutomaticResizeOffer {
    pub chain: IDXGISwapChain3,
    pub queue: ID3D12CommandQueue,
    pub width: u32,
    pub height: u32,
    pub epoch: u64,
}

fn identity(value: &impl Interface) -> Result<u64, String> {
    let canonical: IUnknown = value.cast().map_err(|e| e.to_string())?;
    Ok(canonical.as_raw() as usize as u64)
}
fn fail() {
    FAILED.store(true, Ordering::Release);
    ACTIVE.store(false, Ordering::Release);
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    AUTO_PHASE.store(5, Ordering::Release);
}

#[cfg(feature = "d3d12-auto-resize-fixture")]
unsafe fn prepare_automatic(raw: *mut c_void) -> Result<bool, String> {
    if !AUTOMATIC.load(Ordering::Acquire) {
        return Ok(false);
    }
    let chain = IDXGISwapChain3::from_raw_borrowed(&raw).ok_or("null automatic resize chain")?;
    let Ok(window) = chain.GetHwnd() else {
        return Ok(false);
    };
    {
        let mut state = STATE
            .try_lock()
            .map_err(|_| "automatic resize metadata contended")?;
        let state = state.as_mut().ok_or("automatic observer closed")?;
        if window.0 as usize as u64 != state.target.window {
            return Ok(false);
        }
        if crate::native::own_window_identity(window)? != state.target {
            return Err("automatic target changed".into());
        }
        let Some(gate) = state.automatic.as_mut() else {
            return Ok(false);
        }; // initial discovery
        gate.prepare()
            .map_err(|e| format!("automatic resize phase: {e:?}"))?;
        AUTO_PHASE.store(2, Ordering::Release);
    }
    crate::d3d12_owned_hook::suspend_observed_media()?;
    Ok(true)
}

unsafe extern "system" fn detour(
    raw: *mut c_void,
    count: u32,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    flags: u32,
    masks: *const u32,
    queues: *const *mut c_void,
) -> HRESULT {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original.is_null() {
        return E_FAIL;
    }
    let original: Resize = std::mem::transmute(original);
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    let automatic_prepared = if ACTIVE.load(Ordering::Acquire) {
        match prepare_automatic(raw) {
            Ok(prepared) => prepared,
            Err(_) => {
                fail();
                false
            }
        }
    } else {
        false
    };
    let result = original(raw, count, width, height, format, flags, masks, queues);
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    if result != HRESULT(0) && automatic_prepared {
        fail();
    }
    if result != HRESULT(0) || !ACTIVE.load(Ordering::Acquire) {
        return result;
    }
    let observed = (|| -> Result<(), String> {
        let chain = IDXGISwapChain3::from_raw_borrowed(&raw).ok_or("null resize chain")?;
        let Ok(window) = chain.GetHwnd() else {
            return Ok(());
        };
        // Never block a resize callback on controller work. A lost observation
        // closes discovery rather than guessing later which queue was supplied.
        let mut state = STATE.try_lock().map_err(|_| "late observation contended")?;
        let Some(state) = state.as_mut() else {
            return Err("late observation closed".into());
        };
        if window.0 as usize as u64 != state.target.window {
            return Ok(());
        }
        if !ACTIVE.load(Ordering::Acquire) {
            return Ok(());
        }
        if crate::native::own_window_identity(window)? != state.target {
            return Err("late target lifetime changed".into());
        }
        if !(2..=MAX_PRESENT_QUEUES as u32).contains(&count) || masks.is_null() || queues.is_null()
        {
            return Err("late resize requires bounded explicit queue arrays".into());
        }
        if state.offer.is_some() {
            return Err("overlapping late resize offer".into());
        }
        if chain.GetDesc1().map_err(|e| e.to_string())?.BufferCount != count {
            return Err("late resize buffer count does not match supplied queues".into());
        }
        let device: ID3D12Device = chain.GetDevice().map_err(|e| e.to_string())?;
        let device_id = identity(&device)?;
        let masks = std::slice::from_raw_parts(masks, count as usize);
        let queues = std::slice::from_raw_parts(queues, count as usize);
        let mut metadata = [PresentQueueIdentity {
            queue: 0,
            device: 0,
            direct: false,
            queue_node_mask: 0,
        }; MAX_PRESENT_QUEUES];
        let mut selected = None;
        for (index, raw_queue) in queues.iter().enumerate() {
            let unknown = IUnknown::from_raw_borrowed(raw_queue).ok_or("null present queue")?;
            let queue: ID3D12CommandQueue = unknown.cast().map_err(|e| e.to_string())?;
            let mut queue_device = None::<ID3D12Device>;
            queue
                .GetDevice(&mut queue_device)
                .map_err(|e| e.to_string())?;
            let queue_device = queue_device.ok_or("null queue device")?;
            let desc = queue.GetDesc();
            metadata[index] = PresentQueueIdentity {
                queue: identity(&queue)?,
                device: identity(&queue_device)?,
                direct: desc.Type == D3D12_COMMAND_LIST_TYPE_DIRECT,
                queue_node_mask: desc.NodeMask,
            };
            if selected.is_none() {
                selected = Some(queue);
            }
        }
        validate_present_queues(
            count,
            device_id,
            device.GetNodeCount(),
            masks,
            &metadata[..count as usize],
        )
        .map_err(|e| format!("late queue policy: {e:?}"))?;
        let selected = selected.ok_or("missing selected queue")?;
        #[cfg(feature = "d3d12-auto-resize-fixture")]
        if let Some((old_chain, old_queue)) = state.selected.as_ref() {
            if identity(old_chain)? != identity(chain)?
                || identity(old_queue)? != identity(&selected)?
            {
                return Err("automatic resize cannot change selected chain or queue".into());
            }
            let desc = chain.GetDesc1().map_err(|e| e.to_string())?;
            state
                .automatic
                .as_mut()
                .ok_or("missing automatic gate")?
                .offer(desc.Width, desc.Height)
                .map_err(|e| format!("automatic resize offer: {e:?}"))?;
            AUTO_PHASE.store(3, Ordering::Release);
        }
        state.offer = Some((chain.clone(), selected));
        Ok(())
    })();
    if observed.is_err() {
        fail();
    }
    result // Never change DXGI's return value or caller-owned output.
}

pub struct OwnedLateResizeHook;
impl OwnedLateResizeHook {
    /// # Safety
    /// `probe` must be an independently created live owned probe chain.
    pub unsafe fn install(probe: &IDXGISwapChain3, target: TargetIdentity) -> Result<Self, String> {
        crate::d3d12_bootstrap::validate_owned_attached_process()?;
        if !std::env::args().any(|arg| arg == "--late-d3d12")
            || !target.is_valid()
            || target.pid != std::process::id()
        {
            return Err("explicit exact owned late attachment required".into());
        }
        if ATTEMPTED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("one resident late resize observer per process".into());
        }
        let entry = probe.vtable().ResizeBuffers1 as *mut c_void;
        let mut module = HMODULE::default();
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR(entry.cast()),
            &mut module,
        )
        .map_err(|e| e.to_string())?;
        let original =
            MinHook::create_hook(entry, detour as *mut c_void).map_err(|e| e.to_string())?;
        ORIGINAL.store(original, Ordering::Release);
        *STATE.lock().map_err(|_| "late state poisoned")? = Some(State {
            target,
            offer: None,
            #[cfg(feature = "d3d12-auto-resize-fixture")]
            selected: None,
            #[cfg(feature = "d3d12-auto-resize-fixture")]
            automatic: None,
        });
        #[cfg(feature = "d3d12-auto-resize-fixture")]
        AUTOMATIC.store(
            std::env::args().any(|arg| arg == "--auto-resize-owned"),
            Ordering::Release,
        );
        ACTIVE.store(true, Ordering::Release);
        if let Err(error) = MinHook::enable_hook(entry) {
            fail();
            return Err(error.to_string());
        }
        Ok(Self)
    }
    pub fn claim(&self) -> Result<Option<(IDXGISwapChain3, ID3D12CommandQueue)>, String> {
        if FAILED.load(Ordering::Acquire) {
            return Err("late resize observation failed; no queue inference permitted".into());
        }
        let mut state = STATE.lock().map_err(|_| "late state poisoned")?;
        if FAILED.load(Ordering::Acquire) {
            return Err("late resize observation failed".into());
        }
        let state = state.as_mut().ok_or("late observer closed")?;
        let offer = state.offer.take();
        if let Some((_chain, _queue)) = &offer {
            #[cfg(feature = "d3d12-auto-resize-fixture")]
            if AUTOMATIC.load(Ordering::Acquire) {
                state.selected = Some((_chain.clone(), _queue.clone()));
                state.automatic = crate::automatic_resize_gate::AutomaticResizeGate::new(1);
                AUTO_PHASE.store(1, Ordering::Release);
            } else {
                ACTIVE.store(false, Ordering::Release);
            }
            #[cfg(not(feature = "d3d12-auto-resize-fixture"))]
            ACTIVE.store(false, Ordering::Release);
        }
        Ok(offer)
    }
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    pub(crate) fn take_automatic_offer() -> Result<Option<AutomaticResizeOffer>, String> {
        if FAILED.load(Ordering::Acquire) {
            return Err("automatic resize observation failed".into());
        }
        if AUTO_PHASE.load(Ordering::Acquire) != 3 {
            return Ok(None);
        }
        let mut state = STATE.lock().map_err(|_| "automatic metadata poisoned")?;
        let state = state.as_mut().ok_or("automatic observer closed")?;
        let (chain, queue) = state.offer.take().ok_or("automatic offer missing")?;
        let epoch = state
            .automatic
            .as_mut()
            .ok_or("automatic gate missing")?
            .claim()
            .map_err(|e| format!("automatic claim: {e:?}"))?;
        AUTO_PHASE.store(4, Ordering::Release);
        let desc = unsafe { chain.GetDesc1() }.map_err(|e| e.to_string())?;
        Ok(Some(AutomaticResizeOffer {
            chain,
            queue,
            width: desc.Width,
            height: desc.Height,
            epoch,
        }))
    }
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    pub(crate) fn approve_automatic(epoch: u64) -> Result<(), String> {
        if FAILED.load(Ordering::Acquire) {
            return Err("automatic observation failed before approval".into());
        }
        let mut state = STATE.lock().map_err(|_| "automatic metadata poisoned")?;
        state
            .as_mut()
            .ok_or("automatic state closed")?
            .automatic
            .as_mut()
            .ok_or("automatic gate missing")?
            .approve(epoch)
            .map_err(|e| format!("automatic approval: {e:?}"))?;
        AUTO_PHASE.store(1, Ordering::Release);
        if FAILED.load(Ordering::Acquire) {
            AUTO_PHASE.store(5, Ordering::Release);
            return Err("automatic observation failed during approval".into());
        }
        Ok(())
    }
    pub(crate) fn failed() -> bool {
        FAILED.load(Ordering::Acquire)
    }
    pub fn close(&self) {
        ACTIVE.store(false, Ordering::Release);
        #[cfg(feature = "d3d12-auto-resize-fixture")]
        AUTO_PHASE.store(5, Ordering::Release);
        if let Ok(mut state) = STATE.lock() {
            state.take();
        }
    }
}
impl Drop for OwnedLateResizeHook {
    fn drop(&mut self) {
        self.close();
    }
}
