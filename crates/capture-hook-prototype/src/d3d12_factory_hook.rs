//! Resident factory detour in the exact explicitly opted-in owned child only.
//! Bounded COM observations, no capture/GPU submissions or general attachment.
use crate::d3d12_queue_registry::ASSOCIATION_CAPACITY;
use minhook::MinHook;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::Mutex;
use windows::core::{w, IUnknown, Interface, HRESULT, PCWSTR};
use windows::Win32::Foundation::{E_FAIL, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D12::ID3D12CommandQueue;
use windows::Win32::Graphics::Dxgi::{
    IDXGIFactory4, IDXGISwapChain3, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FULLSCREEN_DESC,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowThreadProcessId};

type Create = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    HWND,
    *const DXGI_SWAP_CHAIN_DESC1,
    *const DXGI_SWAP_CHAIN_FULLSCREEN_DESC,
    *mut c_void,
    *mut *mut c_void,
) -> HRESULT;
static ATTEMPTED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
type LegacyCreate = unsafe extern "system" fn(
    *mut c_void,
    *mut c_void,
    *const DXGI_SWAP_CHAIN_DESC,
    *mut *mut c_void,
) -> HRESULT;
static LEGACY_ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static FORWARDED: AtomicU64 = AtomicU64::new(0);
static OBSERVED: AtomicU64 = AtomicU64::new(0);
static CLAIMED: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static CREATION_ERRORS: AtomicU64 = AtomicU64::new(0);
static OFFERS: Mutex<[Option<ObservedCreation>; ASSOCIATION_CAPACITY]> =
    Mutex::new([const { None }; ASSOCIATION_CAPACITY]);

/// Only this module can construct proof of a successful intercepted creation.
pub(crate) struct ObservedCreation {
    chain: IDXGISwapChain3,
    queue: ID3D12CommandQueue,
    window: usize,
}
impl ObservedCreation {
    pub(crate) fn into_parts(self) -> (IDXGISwapChain3, ID3D12CommandQueue, usize) {
        (self.chain, self.queue, self.window)
    }
}

fn lost_observation() {
    FAILED.store(true, Ordering::Release);
    DROPPED.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "system" fn detour(
    factory: *mut c_void,
    device: *mut c_void,
    window: HWND,
    desc: *const DXGI_SWAP_CHAIN_DESC1,
    fullscreen: *const DXGI_SWAP_CHAIN_FULLSCREEN_DESC,
    output: *mut c_void,
    chain: *mut *mut c_void,
) -> HRESULT {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original.is_null() {
        return E_FAIL;
    }
    let original: Create = std::mem::transmute(original);
    let result = original(factory, device, window, desc, fullscreen, output, chain);
    FORWARDED.fetch_add(1, Ordering::Relaxed);
    if result.is_err() {
        CREATION_ERRORS.fetch_add(1, Ordering::Relaxed);
        return result;
    }
    observe(device, window, chain);
    result
}

unsafe extern "system" fn legacy_detour(
    factory: *mut c_void,
    device: *mut c_void,
    desc: *const DXGI_SWAP_CHAIN_DESC,
    chain: *mut *mut c_void,
) -> HRESULT {
    let original = LEGACY_ORIGINAL.load(Ordering::Acquire);
    if original.is_null() {
        return E_FAIL;
    }
    let original: LegacyCreate = std::mem::transmute(original);
    let result = original(factory, device, desc, chain);
    FORWARDED.fetch_add(1, Ordering::Relaxed);
    if result.is_err() {
        CREATION_ERRORS.fetch_add(1, Ordering::Relaxed);
        return result;
    }
    if !desc.is_null() {
        observe(device, (*desc).OutputWindow, chain);
    }
    result
}

unsafe fn observe(device: *mut c_void, window: HWND, chain: *mut *mut c_void) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    // Only the owned fixture's own class/process is observable. Never inspect an
    // external HWND; all observations are bounded references, never GPU copies.
    let mut pid = 0;
    GetWindowThreadProcessId(window, Some(&mut pid));
    if pid != std::process::id() {
        return;
    }
    let mut class = [0u16; 96];
    let length = GetClassNameW(window, &mut class);
    if length <= 0 || &class[..length as usize] != w!("SilkOwnedD3d12PresentationFixture").as_wide()
    {
        return;
    }
    if device.is_null() || chain.is_null() || (*chain).is_null() {
        lost_observation();
        return;
    }
    // Acquire bounded storage first. Never wait for controller work in a detour.
    let Ok(mut offers) = OFFERS.try_lock() else {
        lost_observation();
        return;
    };
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let Some(slot) = offers.iter_mut().find(|slot| slot.is_none()) else {
        lost_observation();
        return;
    };
    let Some(device) = IUnknown::from_raw_borrowed(&device) else {
        lost_observation();
        return;
    };
    let Some(created) = IUnknown::from_raw_borrowed(&*chain) else {
        lost_observation();
        return;
    };
    match (
        device.cast::<ID3D12CommandQueue>(),
        created.cast::<IDXGISwapChain3>(),
    ) {
        (Ok(queue), Ok(chain)) => {
            *slot = Some(ObservedCreation {
                chain,
                queue,
                window: window.0 as usize,
            });
            OBSERVED.fetch_add(1, Ordering::Relaxed);
        }
        _ => lost_observation(),
    }
    // Observer rejection never changes DXGI's HRESULT or caller-owned output.
}

pub(crate) struct OwnedFactoryHook;
impl OwnedFactoryHook {
    pub(crate) fn validate_opt_in() -> Result<(), String> {
        crate::d3d12_owned_hook::OwnedPresentHook::validate_opt_in()?;
        if !std::env::args().any(|arg| arg == "--factory-hook") {
            return Err("factory hook requires explicit owned-child opt-in".into());
        }
        Ok(())
    }
    pub(crate) unsafe fn install(factory: &IDXGIFactory4) -> Result<Self, String> {
        Self::validate_opt_in()?;
        if ATTEMPTED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("one resident factory hook attempt per owned process".into());
        }
        let target = factory.vtable().base__.base__.CreateSwapChainForHwnd as *mut c_void;
        let mut module = HMODULE::default();
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR(target.cast()),
            &mut module,
        )
        .map_err(|e| e.to_string())?;
        let original =
            MinHook::create_hook(target, detour as *mut c_void).map_err(|e| e.to_string())?;
        ORIGINAL.store(original, Ordering::Release);
        let legacy_target =
            factory.vtable().base__.base__.base__.base__.CreateSwapChain as *mut c_void;
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR(legacy_target.cast()),
            &mut module,
        )
        .map_err(|e| e.to_string())?;
        let legacy_original = MinHook::create_hook(legacy_target, legacy_detour as *mut c_void)
            .map_err(|e| e.to_string())?;
        LEGACY_ORIGINAL.store(legacy_original, Ordering::Release);
        ACTIVE.store(true, Ordering::Release);
        if let Err(e) = MinHook::enable_hook(target) {
            ACTIVE.store(false, Ordering::Release);
            return Err(e.to_string());
        }
        if let Err(e) = MinHook::enable_hook(legacy_target) {
            ACTIVE.store(false, Ordering::Release);
            return Err(e.to_string());
        }
        Ok(Self)
    }
    pub(crate) fn counts(&self) -> (u64, u64, u64, u64, u64) {
        (
            FORWARDED.load(Ordering::Acquire),
            OBSERVED.load(Ordering::Acquire),
            CLAIMED.load(Ordering::Acquire),
            DROPPED.load(Ordering::Acquire),
            CREATION_ERRORS.load(Ordering::Acquire),
        )
    }
    pub(crate) fn failed(&self) -> bool {
        FAILED.load(Ordering::Acquire)
    }
    pub(crate) fn close(&self) {
        ACTIVE.store(false, Ordering::Release);
        if let Ok(mut offers) = OFFERS.lock() {
            offers.fill_with(|| None);
        }
    }
}
impl Drop for OwnedFactoryHook {
    fn drop(&mut self) {
        self.close();
    }
}
pub(crate) fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}
#[cfg(feature = "d3d12-device-replace-fixture")]
pub(crate) fn discard_pending_for_replacement() -> Result<(), String> {
    let mut offers = OFFERS
        .lock()
        .map_err(|_| "factory observation state poisoned")?;
    if !active() || FAILED.load(Ordering::Acquire) {
        return Err("factory observation closed/failed before replacement".into());
    }
    offers.fill_with(|| None);
    Ok(())
}
pub(crate) fn claim(chain: &IDXGISwapChain3) -> Result<ObservedCreation, String> {
    if !active() || FAILED.load(Ordering::Acquire) {
        return Err("factory observation unavailable/terminal; no cooperative fallback".into());
    }
    let id: IUnknown = chain.cast().map_err(|e| e.to_string())?;
    let mut offers = OFFERS
        .lock()
        .map_err(|_| "factory observation state poisoned")?;
    // Recheck after acquisition: cancellation/lost observation cannot reopen.
    if !active() || FAILED.load(Ordering::Acquire) {
        return Err("factory observation closed/failed".into());
    }
    for slot in offers.iter_mut() {
        if let Some(offer) = slot {
            let other: IUnknown = offer.chain.cast().map_err(|e| e.to_string())?;
            if other.as_raw() == id.as_raw() {
                CLAIMED.fetch_add(1, Ordering::Relaxed);
                return Ok(slot.take().unwrap());
            }
        }
    }
    Err("no intercepted creation for exact chain; no cooperative fallback".into())
}

/// Select only by explicit owned target metadata, never by a renderer COM handoff.
#[cfg(feature = "d3d12-bootstrap-fixture")]
pub(crate) fn claim_window(window: u64) -> Result<Option<ObservedCreation>, String> {
    if !active() || FAILED.load(Ordering::Acquire) {
        return Err("factory observation unavailable/terminal".into());
    }
    let mut offers = OFFERS
        .lock()
        .map_err(|_| "factory observation state poisoned")?;
    if !active() || FAILED.load(Ordering::Acquire) {
        return Err("factory observation closed/failed".into());
    }
    let mut found = None;
    for (index, slot) in offers.iter().enumerate() {
        if slot
            .as_ref()
            .is_some_and(|offer| offer.window as u64 == window)
            && found.replace(index).is_some()
        {
            return Err("ambiguous observed chains for selected HWND".into());
        }
    }
    Ok(found.map(|index| {
        CLAIMED.fetch_add(1, Ordering::Relaxed);
        offers[index].take().unwrap()
    }))
}

/// Bounded diagnostics only, executed as a separate exact owned fixture child.
/// No renderer or capture session and no arbitrary process/window input.
pub fn run_owned_probe(warp: bool) -> Result<(), String> {
    use crate::d3d12_fixture::D3d12Fixture;
    use crate::d3d12_present_fixture::Window;
    use crate::d3d12_queue_owner::NativeQueueOwner;
    use std::rc::Rc;
    use windows::Win32::Graphics::Direct3D12::{
        D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    };
    OwnedFactoryHook::validate_opt_in()?;
    let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.map_err(|e| e.to_string())?;
    let hook = unsafe { OwnedFactoryHook::install(&factory) }?;
    if unsafe { OwnedFactoryHook::install(&factory) }.is_ok() {
        return Err("duplicate factory installation succeeded".into());
    }
    let fixture = D3d12Fixture::new(warp, 318, 178)?;
    let (device, queue) = fixture.device_queue();
    let other_queue: ID3D12CommandQueue = unsafe {
        device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        })
    }
    .map_err(|e| e.to_string())?;
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: 318,
        Height: 178,
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
    let window = Rc::new(Window::new(true, 318, 178)?);
    let legacy = std::env::args().any(|argument| argument == "--legacy-factory");
    let create = |queue: &ID3D12CommandQueue,
                  window: &Window,
                  desc: &DXGI_SWAP_CHAIN_DESC1|
     -> Result<IDXGISwapChain3, String> {
        if legacy {
            let legacy_desc = DXGI_SWAP_CHAIN_DESC {
                BufferDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_MODE_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
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
            unsafe { factory.CreateSwapChain(queue, &legacy_desc, &mut chain) }
                .ok()
                .map_err(|e| e.to_string())?;
            chain
                .ok_or("legacy probe creation returned no chain")?
                .cast()
                .map_err(|e| e.to_string())
        } else {
            unsafe { factory.CreateSwapChainForHwnd(queue, window.0, desc, None, None) }
                .map_err(|e| e.to_string())?
                .cast()
                .map_err(|e| e.to_string())
        }
    };
    let mut invalid = desc;
    invalid.BufferCount = 0;
    if create(&queue, &window, &invalid).is_ok() {
        return Err("invalid native creation unexpectedly succeeded".into());
    }
    if hook.counts() != (1, 0, 0, 0, 1) {
        return Err("failed creation was not transparently forwarded".into());
    }
    let raw_create = |queue: &ID3D12CommandQueue,
                      window: &Window|
     -> Result<IDXGISwapChain3, String> { create(queue, window, &desc) };
    // A different class in this owned process forwards without observation.
    let ignored_window = Window(
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("Silk ignored factory probe"),
                windows::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW,
                20,
                20,
                318,
                178,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(|e| e.to_string())?,
    );
    let _ignored_chain = raw_create(&queue, &ignored_window)?;
    if hook.counts() != (2, 0, 0, 0, 1) {
        return Err("unrelated window class was observed".into());
    }
    let selected = raw_create(&queue, &window)?;
    let mut owner = NativeQueueOwner::default();
    let token = owner.claim_intercepted(&selected, window.clone())?;
    if owner.claim_intercepted(&selected, window.clone()).is_ok() {
        return Err("creation offer could be claimed twice".into());
    }
    owner.select(token)?;
    let unrelated_window = Rc::new(Window::new(false, 318, 178)?);
    let unrelated = raw_create(&other_queue, &unrelated_window)?;
    owner.claim_intercepted(&unrelated, unrelated_window.clone())?;
    if owner.selected(&unrelated).is_ok() {
        return Err("unrelated factory chain selected".into());
    }
    let (_, discovered) = owner.selected(&selected)?;
    let expected: IUnknown = queue.cast().map_err(|e| e.to_string())?;
    let actual: IUnknown = discovered.cast().map_err(|e| e.to_string())?;
    if expected.as_raw() != actual.as_raw() {
        return Err("factory observation selected wrong same-device queue".into());
    }
    if hook.counts() != (4, 2, 2, 0, 1) {
        return Err("actual interception counts did not match observed creations".into());
    }
    let mut retained = Vec::with_capacity(ASSOCIATION_CAPACITY + 2);
    for _ in 0..ASSOCIATION_CAPACITY {
        let window = Rc::new(Window::new(false, 318, 178)?);
        let chain = raw_create(&queue, &window)?;
        retained.push((window, chain));
    }
    let overflow_window = Rc::new(Window::new(false, 318, 178)?);
    let overflow_chain = raw_create(&queue, &overflow_window)?;
    if !hook.failed() || hook.counts().3 != 1 || claim(&overflow_chain).is_ok() {
        return Err("full observer did not fail closed while forwarding creation".into());
    }
    // Deliberate same-thread contention: DXGI must return normally, never wait
    // for the controller's offer lock. Lost observation stays terminal.
    let blocked_window = Rc::new(Window::new(false, 318, 178)?);
    let lock = OFFERS.lock().map_err(|_| "factory offer state poisoned")?;
    let blocked_chain = raw_create(&queue, &blocked_window)?;
    drop(lock);
    if hook.counts().3 != 2 {
        return Err("contended observer did not drop".into());
    }
    if claim(&blocked_chain).is_ok() {
        return Err("lost observation was recovered unsafely".into());
    }
    hook.close();
    let before = hook.counts();
    let stopped_window = Rc::new(Window::new(false, 318, 178)?);
    let stopped_chain = raw_create(&queue, &stopped_window)?;
    let after = hook.counts();
    if after.0 != before.0 + 1 || after.1 != before.1 || claim(&stopped_chain).is_ok() {
        return Err("factory cancellation stopped forwarding or reopened observations".into());
    }
    owner.retire(token)?;
    if owner.selected(&selected).is_ok() {
        return Err("retired selected factory association resolved".into());
    }
    println!("PASS: actual owned factory detour {}: original failed/successful calls; two distinct same-device queues; canonical selected association; duplicate claim rejection; eight pending offers; overflow/contention preserve successful DXGI return but fail observation closed; cancellation keeps original creation working; resident forwarding until exit. counts={after:?}", if warp {"WARP"} else {"hardware"});
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_process_cannot_install_factory_hook() {
        assert!(OwnedFactoryHook::validate_opt_in().is_err());
        assert!(!ATTEMPTED.load(Ordering::Acquire));
    }
}
