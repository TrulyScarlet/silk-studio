//! One-shot, resident Present forwarding in the explicitly owned D3D12 child only.
use crate::d3d12_gpu_bridge::{D3d12GpuBridge, GpuInteropStats};
use minhook::MinHook;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use windows::core::{Interface, HRESULT, PCWSTR};
use windows::Win32::Foundation::{E_FAIL, HMODULE};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain3, DXGI_PRESENT, DXGI_PRESENT_TEST};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
};

type Present = unsafe extern "system" fn(*mut c_void, u32, DXGI_PRESENT) -> HRESULT;
static INSTALLED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
struct InitialArmGate {
    used: AtomicBool,
    cancelled: AtomicBool,
}
impl InitialArmGate {
    const fn new() -> Self {
        Self {
            used: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }
    fn claim(&self) -> bool {
        !self.cancelled()
            && self
                .used
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.used.store(true, Ordering::Release);
    }
}
static INITIAL_ARM: InitialArmGate = InitialArmGate::new();
static FAILED: AtomicBool = AtomicBool::new(false);
static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static SELECTED: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "d3d12-device-replace-fixture")]
static ENTRY: AtomicUsize = AtomicUsize::new(0);
static STATE: Mutex<Option<D3d12GpuBridge>> = Mutex::new(None);
static SELECTED_FORWARDS: AtomicU64 = AtomicU64::new(0);
static UNRELATED_FORWARDS: AtomicU64 = AtomicU64::new(0);
static CAPTURED: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
fn capture_allowed() -> bool {
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    {
        crate::d3d12_late_resize::automatic_capture_allowed()
    }
    #[cfg(not(feature = "d3d12-auto-resize-fixture"))]
    {
        true
    }
}

#[cfg(feature = "d3d12-auto-resize-fixture")]
pub(crate) fn suspend_observed_media() -> Result<(), String> {
    ACTIVE.store(false, Ordering::Release);
    INITIAL_ARM.cancel();
    // Local GPU access only: never wait for parent retirement, frontend or disk
    // work on the application's resize thread.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    let mut state = loop {
        match STATE.try_lock() {
            Ok(state) => break state,
            Err(std::sync::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(1))
            }
            Err(_) => {
                return Err("observed resize could not acquire local GPU state by deadline".into())
            }
        }
    };
    let gpu = state
        .as_mut()
        .ok_or("observed resize has no active media")?;
    gpu.cancel();
    gpu.release_sources()?;
    state.take();
    Ok(())
}

unsafe extern "system" fn detour(raw: *mut c_void, interval: u32, flags: DXGI_PRESENT) -> HRESULT {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original.is_null() {
        return E_FAIL;
    }
    let selected = raw as usize == SELECTED.load(Ordering::Acquire);
    if selected
        && ACTIVE.load(Ordering::Acquire)
        && capture_allowed()
        && flags.0 & DXGI_PRESENT_TEST.0 == 0
    {
        match STATE.try_lock() {
            Ok(mut state) => {
                let result = (|| {
                    if !ACTIVE.load(Ordering::Acquire) || !capture_allowed() {
                        return Ok(false);
                    }
                    let gpu = state.as_mut().ok_or(())?;
                    // The selected raw interface is exactly the owned chain3
                    // whose Present vtable was supplied at installation.
                    let chain = IDXGISwapChain3::from_raw_borrowed(&raw).ok_or(())?;
                    let target: ID3D12Resource = chain
                        .GetBuffer(chain.GetCurrentBackBufferIndex())
                        .map_err(|_| ())?;
                    gpu.submit(&target).map_err(|_| ())
                })();
                match result {
                    Ok(true) => {
                        CAPTURED.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(false) => {
                        DROPPED.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(()) => {
                        FAILED.store(true, Ordering::Release);
                        ACTIVE.store(false, Ordering::Release);
                    }
                }
            }
            Err(_) => {
                DROPPED.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    let original: Present = std::mem::transmute(original);
    let result = original(raw, interval, flags);
    if selected {
        SELECTED_FORWARDS.fetch_add(1, Ordering::Relaxed);
    } else {
        UNRELATED_FORWARDS.fetch_add(1, Ordering::Relaxed);
    }
    if selected && result.is_err() {
        FAILED.store(true, Ordering::Release);
        ACTIVE.store(false, Ordering::Release);
    }
    result
}
pub(crate) struct OwnedPresentHook;
impl OwnedPresentHook {
    pub(crate) fn validate_opt_in() -> Result<(), String> {
        if !std::env::args().any(|a| a == "--allow-owned-d3d12")
            || !std::env::args().any(|a| a == "--present-hook")
            || !std::env::current_exe().ok().is_some_and(|p| {
                p.file_name().is_some_and(|n| {
                    n.to_string_lossy()
                        .eq_ignore_ascii_case("silk-d3d12-owned-fixture.exe")
                })
            })
        {
            return Err("Present detour restricted to explicit owned D3D12 fixture child".into());
        }
        Ok(())
    }
    pub(crate) unsafe fn install(
        chain: &IDXGISwapChain3,
        gpu: D3d12GpuBridge,
    ) -> Result<Self, String> {
        Self::install_with_media(chain, Some(gpu))
    }
    #[cfg(feature = "d3d12-bootstrap-fixture")]
    pub(crate) unsafe fn install_forwarding(chain: &IDXGISwapChain3) -> Result<Self, String> {
        Self::install_with_media(chain, None)
    }
    unsafe fn install_with_media(
        chain: &IDXGISwapChain3,
        gpu: Option<D3d12GpuBridge>,
    ) -> Result<Self, String> {
        Self::validate_opt_in()?;
        if INSTALLED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("one D3D12 resident hook attempt per owned process".into());
        }
        let target = chain.vtable().base__.base__.base__.Present as *mut c_void;
        #[cfg(feature = "d3d12-device-replace-fixture")]
        ENTRY.store(target as usize, Ordering::Release);
        let mut pinned = HMODULE::default();
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR(target.cast()),
            &mut pinned,
        )
        .map_err(|e| e.to_string())?;
        let original =
            MinHook::create_hook(target, detour as *mut c_void).map_err(|e| e.to_string())?;
        ORIGINAL.store(original, Ordering::Release);
        SELECTED.store(chain.as_raw() as usize, Ordering::Release);
        let armed = gpu.is_some();
        *STATE.lock().map_err(|_| "owned Present state poisoned")? = gpu;
        if armed && !INITIAL_ARM.claim() {
            return Err("initial Present activation already closed".into());
        }
        ACTIVE.store(armed, Ordering::Release);
        if let Err(e) = MinHook::enable_hook(target) {
            ACTIVE.store(false, Ordering::Release);
            STATE
                .lock()
                .map_err(|_| "owned Present state poisoned")?
                .take();
            return Err(e.to_string());
        }
        Ok(Self)
    }
    #[cfg(feature = "d3d12-bootstrap-fixture")]
    pub(crate) fn arm_initial(
        &self,
        chain: &IDXGISwapChain3,
        gpu: D3d12GpuBridge,
    ) -> Result<(), String> {
        let mut state = STATE.lock().map_err(|_| "owned Present state poisoned")?;
        if self.failed()
            || INITIAL_ARM.cancelled()
            || ACTIVE.load(Ordering::Acquire)
            || state.is_some()
            || SELECTED.load(Ordering::Acquire) != chain.as_raw() as usize
            || self.counts().0 < 12
            || !INITIAL_ARM.claim()
        {
            return Err(
                "initial capture requires healthy one-shot forwarding warmup on the selected chain"
                    .into(),
            );
        }
        *state = Some(gpu);
        ACTIVE.store(true, Ordering::Release);
        if INITIAL_ARM.cancelled() {
            ACTIVE.store(false, Ordering::Release);
            if let Some(gpu) = state.as_mut() {
                gpu.cancel();
            }
            return Err("initial capture canceled during activation".into());
        }
        Ok(())
    }
    pub(crate) fn access<R>(
        &self,
        f: impl FnOnce(&mut D3d12GpuBridge) -> Result<R, String>,
    ) -> Result<R, String> {
        let mut state = STATE
            .lock()
            .map_err(|_| "owned D3D12 hook state poisoned")?;
        f(state.as_mut().ok_or("owned D3D12 hook media retired")?)
    }
    pub(crate) fn failed(&self) -> bool {
        FAILED.load(Ordering::Acquire)
    }
    #[cfg(feature = "d3d12-rearm-fixture")]
    pub(crate) fn release_media_for_resize(&self) -> Result<(), String> {
        self.cancel();
        self.access(|gpu| gpu.release_sources())?;
        STATE
            .lock()
            .map_err(|_| "owned Present state poisoned")?
            .take();
        Ok(())
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn pause_for_pending_loss(&self) {
        INITIAL_ARM.cancel();
        ACTIVE.store(false, Ordering::Release);
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn retired_remote_pending_loss(
        &self,
        session: crate::SessionIdentity,
        device: &windows::Win32::Graphics::Direct3D12::ID3D12Device,
        chain: &IDXGISwapChain3,
    ) -> Result<(), String> {
        if ACTIVE.load(Ordering::Acquire) || self.failed() {
            return Err(
                "pending-loss probe cannot run with active/failed Present admission".into(),
            );
        }
        self.access(|gpu| {
            gpu.approve_remote_retirement(session)?;
            gpu.probe_pending_removal(device, chain)
        })
    }
    #[cfg(feature = "d3d12-rearm-fixture")]
    pub(crate) fn stage_resize(
        &self,
        chain: &IDXGISwapChain3,
        gpu: D3d12GpuBridge,
    ) -> Result<(), String> {
        if ACTIVE.load(Ordering::Acquire)
            || self.failed()
            || SELECTED.load(Ordering::Acquire) != chain.as_raw() as usize
        {
            return Err("resize cannot replace active/failed/other-chain hook".into());
        }
        let mut state = STATE.lock().map_err(|_| "owned Present state poisoned")?;
        if state.is_some() {
            return Err("old resize media was not released".into());
        }
        *state = Some(gpu);
        Ok(())
    }
    #[cfg(feature = "d3d12-rearm-fixture")]
    pub(crate) fn activate_resize(&self) -> Result<(), String> {
        if self.failed()
            || STATE
                .lock()
                .map_err(|_| "owned Present state poisoned")?
                .is_none()
        {
            return Err("no valid staged resize media".into());
        }
        ACTIVE.store(true, Ordering::Release);
        Ok(())
    }
    pub(crate) fn counts(&self) -> (u64, u64, u64, u64) {
        (
            SELECTED_FORWARDS.load(Ordering::Acquire),
            UNRELATED_FORWARDS.load(Ordering::Acquire),
            CAPTURED.load(Ordering::Acquire),
            DROPPED.load(Ordering::Acquire),
        )
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn stage_device_replacement(
        &self,
        chain: &IDXGISwapChain3,
        gpu: D3d12GpuBridge,
        window: u64,
    ) -> Result<(), String> {
        if ACTIVE.load(Ordering::Acquire)
            || self.failed()
            || chain.vtable().base__.base__.base__.Present as usize != ENTRY.load(Ordering::Acquire)
            || unsafe { chain.GetHwnd() }.map_err(|e| e.to_string())?.0 as usize as u64 != window
        {
            return Err("replacement hook entry/window mismatch or active media".into());
        }
        let mut state = STATE.lock().map_err(|_| "owned Present state poisoned")?;
        if state.is_some() {
            return Err("old replacement media was not released".into());
        }
        SELECTED.store(chain.as_raw() as usize, Ordering::Release);
        *state = Some(gpu);
        Ok(())
    }
    pub(crate) fn gpu_stats(&self) -> Option<GpuInteropStats> {
        self.access(|g| Ok(g.stats())).ok()
    }
    pub(crate) fn cancel(&self) {
        INITIAL_ARM.cancel();
        ACTIVE.store(false, Ordering::Release);
        let _ = self.access(|g| {
            g.cancel();
            Ok(())
        });
    }
    pub(crate) fn drain(&self) -> bool {
        self.access(|g| Ok(g.drain())).unwrap_or(false)
    }
}
impl Drop for OwnedPresentHook {
    fn drop(&mut self) {
        self.cancel();
        if let Ok(mut state) = STATE.lock() {
            state.take();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_media_activation_is_one_shot_and_cancel_is_terminal() {
        let gate = InitialArmGate::new();
        assert!(gate.claim());
        assert!(!gate.claim());
        gate.cancel();
        assert!(gate.cancelled());
        assert!(!gate.claim());
        let cancelled = InitialArmGate::new();
        cancelled.cancel();
        assert!(!cancelled.claim());
        assert!(cancelled.cancelled());
    }
    #[test]
    fn cancellation_after_initial_claim_is_observable_before_media_admission() {
        let gate = InitialArmGate::new();
        assert!(gate.claim());
        // Same ordering as a cancellation between claim and ACTIVE.store: the
        // activation path must recheck this sticky flag after publishing ACTIVE.
        gate.cancel();
        assert!(gate.cancelled());
        assert!(!gate.claim());
    }
    #[test]
    fn test_process_cannot_opt_in_to_owned_child_hook() {
        assert!(OwnedPresentHook::validate_opt_in().is_err());
        assert!(!INSTALLED.load(Ordering::Acquire));
    }
}
