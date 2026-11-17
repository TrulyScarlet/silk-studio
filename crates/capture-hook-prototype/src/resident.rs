//! One-shot resident forwarder. Never removes a hook or unloads executable code.
use crate::native::{prepare_capture, CapturedFrame, TapState, TapStats};
use crate::SessionIdentity;
use minhook::MinHook;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::core::{Interface, HRESULT};
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain, DXGI_PRESENT, DXGI_PRESENT_TEST};

type Present = unsafe extern "system" fn(*mut c_void, u32, DXGI_PRESENT) -> HRESULT;
static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static INSTALLED: AtomicBool = AtomicBool::new(false);
static DISCOVERING: AtomicBool = AtomicBool::new(false);
static MEDIA_OPEN: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<TapState>> = Mutex::new(None);
struct Discovery {
    session: SessionIdentity,
    sender: SyncSender<IDXGISwapChain>,
    claimed: AtomicBool,
}
static DISCOVERY: OnceLock<Discovery> = OnceLock::new();
pub static FORWARDED: AtomicU64 = AtomicU64::new(0);
pub static CAPTURE_BUSY: AtomicU64 = AtomicU64::new(0);

unsafe extern "system" fn detour(
    chain: *mut c_void,
    interval: u32,
    flags: DXGI_PRESENT,
) -> HRESULT {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original.is_null() {
        return windows::Win32::Foundation::E_FAIL;
    }
    if let Some(discovery) = DISCOVERY.get() {
        if DISCOVERING.load(Ordering::Acquire) && !discovery.claimed.load(Ordering::Acquire) {
            if let Some(borrowed) = IDXGISwapChain::from_raw_borrowed(&chain) {
                if borrowed
                    .GetDesc()
                    .is_ok_and(|desc| desc.OutputWindow.0 as u64 == discovery.session.target.window)
                    && discovery
                        .claimed
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    // COM AddRef + fixed-capacity try_send only; initialization
                    // and texture allocation are on the worker, not Present.
                    if discovery.sender.try_send(borrowed.clone()).is_err() {
                        discovery.claimed.store(false, Ordering::Release);
                    }
                }
            }
        }
    }
    if MEDIA_OPEN.load(Ordering::Acquire) && flags.0 & DXGI_PRESENT_TEST.0 == 0 {
        match STATE.try_lock() {
            Ok(mut state) => {
                if let Some(state) = state.as_mut().filter(|state| state.matches(chain)) {
                    state.capture();
                }
            }
            Err(_) => {
                CAPTURE_BUSY.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    let original: Present = std::mem::transmute(original);
    let result = original(chain, interval, flags);
    if result.is_err() {
        if let Ok(state) = STATE.try_lock() {
            if let Some(state) = state.as_ref().filter(|state| state.matches(chain)) {
                state.present_failed();
            }
        }
    }
    FORWARDED.fetch_add(1, Ordering::Relaxed);
    result
}

pub struct ResidentCapture {
    pub frames: Receiver<CapturedFrame>,
    pub stats: Arc<TapStats>,
}
impl ResidentCapture {
    /// # Safety
    /// The DLL and this module's statics MUST stay loaded until process exit.
    /// Call only in the explicitly owned, opted-in fixture. Installation relies
    /// on MinHook's thread management, not a claim of arbitrary-game safety.
    pub unsafe fn discover(
        probe: &IDXGISwapChain,
        session: SessionIdentity,
    ) -> Result<(Self, IDXGISwapChain), String> {
        if crate::SessionGate::new(session).is_none() {
            return Err("invalid resident selection".into());
        }
        if INSTALLED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("resident attachment is one-shot per process".into());
        }
        let (sender, receiver) = sync_channel(1);
        DISCOVERY
            .set(Discovery {
                session,
                sender,
                claimed: AtomicBool::new(false),
            })
            .map_err(|_| "resident discovery already initialized")?;
        let target = probe.vtable().Present as *mut c_void;
        let mut owner = windows::Win32::Foundation::HMODULE::default();
        windows::Win32::System::LibraryLoader::GetModuleHandleExW(
            windows::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                | windows::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_PIN,
            windows::core::PCWSTR(target.cast()),
            &mut owner,
        )
        .map_err(|error| error.to_string())?;
        let original = MinHook::create_hook(target, detour as *mut c_void)
            .map_err(|error| error.to_string())?;
        ORIGINAL.store(original, Ordering::Release);
        // Never remove/disable this hook, including on subsequent errors.
        DISCOVERING.store(true, Ordering::Release);
        MinHook::enable_hook(target).map_err(|error| error.to_string())?;
        let discovered = receiver.recv_timeout(Duration::from_secs(8));
        DISCOVERING.store(false, Ordering::Release);
        let chain =
            discovered.map_err(|error| format!("selected Present discovery failed: {error}"))?;
        let (state, frames, stats) = prepare_capture(&chain, session)?;
        *STATE.lock().map_err(|_| "resident media state poisoned")? = Some(state);
        MEDIA_OPEN.store(true, Ordering::Release);
        Ok((Self { frames, stats }, chain))
    }
    pub fn cancel(&self) -> bool {
        MEDIA_OPEN.store(false, Ordering::Release);
        self.stats.cancel_capture();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(mut state) = STATE.try_lock() {
                state.take();
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        // Keep state resident when a driver/callback does not drain. Never
        // reclaim its executable forwarder or trampoline on this evidence.
        false
    }
}
impl Drop for ResidentCapture {
    fn drop(&mut self) {
        self.cancel();
    }
}
