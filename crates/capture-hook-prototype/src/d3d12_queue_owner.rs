//! Owner-thread-only, bounded COM ownership for successful owned factory calls.
//! Cooperative creation or single-use proof from the owned factory detour.
//! Not independent bootstrap or general-game discovery.
use crate::d3d12_present_fixture::Window;
use crate::d3d12_queue_registry::{
    Association, AssociationToken, QueueRegistry, ASSOCIATION_CAPACITY,
};
use std::rc::Rc;
use windows::core::{IUnknown, Interface};
use windows::Win32::Graphics::Direct3D12::{
    ID3D12CommandQueue, ID3D12Device, D3D12_COMMAND_LIST_TYPE_DIRECT,
};
use windows::Win32::Graphics::Dxgi::{IDXGIFactory4, IDXGISwapChain3, DXGI_SWAP_CHAIN_DESC1};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};

fn error(e: windows::core::Error) -> String {
    e.to_string()
}
fn identity(object: &impl Interface) -> Result<IUnknown, String> {
    object.cast().map_err(error)
}
fn address(object: &IUnknown) -> u64 {
    object.as_raw() as usize as u64
}

struct Entry {
    token: AssociationToken,
    chain: IUnknown,
    _queue_identity: IUnknown,
    _device_identity: IUnknown,
    queue: ID3D12CommandQueue,
    device: ID3D12Device,
    window: Rc<Window>,
}

pub(crate) struct NativeQueueOwner {
    registry: QueueRegistry,
    entries: [Option<Entry>; ASSOCIATION_CAPACITY],
    selected: Option<AssociationToken>,
    closed: bool,
}
impl Default for NativeQueueOwner {
    fn default() -> Self {
        Self {
            registry: QueueRegistry::default(),
            entries: std::array::from_fn(|_| None),
            selected: None,
            closed: false,
        }
    }
}
impl NativeQueueOwner {
    /// The wrapper itself performs creation: a same-device unrelated queue cannot
    /// be relabeled as the queue used to create an existing swap chain.
    pub(crate) fn create(
        &mut self,
        factory: &IDXGIFactory4,
        queue: &ID3D12CommandQueue,
        window: Rc<Window>,
        desc: &DXGI_SWAP_CHAIN_DESC1,
    ) -> Result<(IDXGISwapChain3, AssociationToken), String> {
        if self.closed {
            return Err("owned queue registry closed".into());
        }
        if self.entries.iter().all(Option::is_some) {
            return Err("owned queue registry full".into());
        }
        validate_window(&window)?;
        if unsafe { queue.GetDesc() }.Type != D3D12_COMMAND_LIST_TYPE_DIRECT {
            return Err("swap-chain capture requires a DIRECT queue".into());
        }
        let chain: IDXGISwapChain3 =
            unsafe { factory.CreateSwapChainForHwnd(queue, window.0, desc, None, None) }
                .map_err(error)?
                .cast()
                .map_err(error)?;
        let token = self.register(&chain, queue, window)?;
        Ok((chain, token))
    }

    #[cfg(feature = "d3d12-factory-hook-fixture")]
    pub(crate) fn claim_intercepted(
        &mut self,
        chain: &IDXGISwapChain3,
        window: Rc<Window>,
    ) -> Result<AssociationToken, String> {
        let observed = crate::d3d12_factory_hook::claim(chain)?;
        let (created, queue, hwnd) = observed.into_parts();
        if hwnd != window.0 .0 as usize
            || address(&identity(&created)?) != address(&identity(chain)?)
        {
            return Err("intercepted chain/window mismatch".into());
        }
        self.register(&created, &queue, window)
    }

    fn register(
        &mut self,
        chain: &IDXGISwapChain3,
        queue: &ID3D12CommandQueue,
        window: Rc<Window>,
    ) -> Result<AssociationToken, String> {
        if self.closed {
            return Err("owned queue registry closed".into());
        }
        let index = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or("owned queue registry full")?;
        validate_window(&window)?;
        if unsafe { queue.GetDesc() }.Type != D3D12_COMMAND_LIST_TYPE_DIRECT {
            return Err("swap-chain capture requires a DIRECT queue".into());
        }
        let mut queue_device: Option<ID3D12Device> = None;
        unsafe { queue.GetDevice(&mut queue_device) }.map_err(error)?;
        let device = queue_device.ok_or("queue returned null device")?;
        let queue_identity = identity(queue)?;
        let device_identity = identity(&device)?;
        let actual_window = unsafe { chain.GetHwnd() }.map_err(error)?;
        let chain_device: ID3D12Device = unsafe { chain.GetDevice() }.map_err(error)?;
        if actual_window != window.0
            || address(&identity(&chain_device)?) != address(&device_identity)
        {
            return Err("created swap-chain window/device mismatch".into());
        }
        validate_window(&window)?;
        let chain_identity = identity(chain)?;
        let token = self
            .registry
            .observe(Association {
                chain: address(&chain_identity),
                queue: address(&queue_identity),
                device: address(&device_identity),
                window: window.0 .0 as usize as u64,
            })
            .map_err(|e| format!("owned queue association rejected: {e:?}"))?;
        self.entries[index] = Some(Entry {
            token,
            chain: chain_identity,
            _queue_identity: queue_identity,
            _device_identity: device_identity,
            queue: queue.clone(),
            device,
            window,
        });
        Ok(token)
    }

    pub(crate) fn select(&mut self, token: AssociationToken) -> Result<(), String> {
        self.registry
            .select(token)
            .map_err(|e| format!("owned queue selection rejected: {e:?}"))?;
        self.selected = Some(token);
        Ok(())
    }

    pub(crate) fn selected(
        &self,
        chain: &IDXGISwapChain3,
    ) -> Result<(ID3D12Device, ID3D12CommandQueue), String> {
        let id = identity(chain)?;
        self.registry
            .selected_for(address(&id))
            .map_err(|e| format!("owned queue resolution rejected: {e:?}"))?;
        let entry = self
            .entries
            .iter()
            .flatten()
            .find(|entry| address(&entry.chain) == address(&id))
            .ok_or("owned queue association missing")?;
        validate_window(&entry.window)?;
        Ok((entry.device.clone(), entry.queue.clone()))
    }

    pub(crate) fn retire(&mut self, token: AssociationToken) -> Result<(), String> {
        self.registry
            .retire(token)
            .map_err(|e| format!("owned queue retirement rejected: {e:?}"))?;
        if self.selected == Some(token) {
            self.close();
        } else {
            for slot in &mut self.entries {
                if slot.as_ref().is_some_and(|entry| entry.token == token) {
                    *slot = None;
                }
            }
        }
        Ok(())
    }

    fn close(&mut self) {
        // Metadata becomes terminal before releasing retained native lifetimes.
        self.registry.close();
        self.closed = true;
        self.selected = None;
        self.entries.fill_with(|| None);
    }
}
impl Drop for NativeQueueOwner {
    fn drop(&mut self) {
        self.close();
    }
}

fn validate_window(window: &Window) -> Result<(), String> {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window.0, Some(&mut pid));
    }
    if !unsafe { IsWindow(Some(window.0)) }.as_bool() || pid != std::process::id() {
        return Err("association window is no longer owned/live".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d3d12_fixture::D3d12Fixture;
    use windows::Win32::Graphics::Direct3D12::{
        D3D12_COMMAND_LIST_TYPE_COMPUTE, D3D12_COMMAND_QUEUE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGISwapChain1, DXGI_SWAP_EFFECT_FLIP_DISCARD,
        DXGI_USAGE_RENDER_TARGET_OUTPUT,
    };

    fn descriptor() -> DXGI_SWAP_CHAIN_DESC1 {
        DXGI_SWAP_CHAIN_DESC1 {
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
        }
    }

    fn lifecycle(warp: bool) {
        let fixture = D3d12Fixture::new(warp, 318, 178).unwrap();
        let (device, queue) = fixture.device_queue();
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.unwrap();
        let mut owner = NativeQueueOwner::default();
        let invalid = Rc::new(Window(Default::default()));
        assert!(owner
            .create(&factory, &queue, invalid, &descriptor())
            .is_err());
        let window = Rc::new(Window::new(true, 318, 178).unwrap());
        let weak = Rc::downgrade(&window);
        let hwnd = window.0;
        let compute: ID3D12CommandQueue = unsafe {
            device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_COMPUTE,
                ..Default::default()
            })
        }
        .unwrap();
        assert!(owner
            .create(&factory, &compute, window.clone(), &descriptor())
            .unwrap_err()
            .contains("DIRECT"));
        let mut bad = descriptor();
        bad.BufferCount = 0;
        assert!(owner
            .create(&factory, &queue, window.clone(), &bad)
            .is_err());
        assert!(owner.entries.iter().all(Option::is_none));

        let (selected, token) = owner
            .create(&factory, &queue, window.clone(), &descriptor())
            .unwrap();
        drop(window);
        assert!(weak.upgrade().is_some()); // native owner holds the actual HWND guard
        assert!(owner.selected(&selected).is_err()); // no automatic first chain
        owner.select(token).unwrap();
        owner.select(token).unwrap();
        let alias: IDXGISwapChain1 = selected.cast().unwrap();
        let alias: IDXGISwapChain3 = alias.cast().unwrap();
        let (resolved_device, resolved_queue) = owner.selected(&alias).unwrap();
        assert_eq!(
            address(&identity(&resolved_device).unwrap()),
            address(&identity(&device).unwrap())
        );
        assert_eq!(
            address(&identity(&resolved_queue).unwrap()),
            address(&identity(&queue).unwrap())
        );

        let mut others = Vec::new();
        let other_queue: ID3D12CommandQueue = unsafe {
            device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                ..Default::default()
            })
        }
        .unwrap();
        assert_ne!(
            address(&identity(&queue).unwrap()),
            address(&identity(&other_queue).unwrap())
        );
        for _ in 1..ASSOCIATION_CAPACITY {
            let w = Rc::new(Window::new(false, 318, 178).unwrap());
            let pair = owner
                .create(&factory, &other_queue, w, &descriptor())
                .unwrap();
            assert!(owner.selected(&pair.0).is_err());
            assert!(owner.select(pair.1).is_err());
            others.push(pair);
        }
        let extra = Rc::new(Window::new(false, 318, 178).unwrap());
        let count = Rc::strong_count(&extra);
        assert!(owner
            .create(&factory, &queue, extra.clone(), &descriptor())
            .unwrap_err()
            .contains("full"));
        assert_eq!(Rc::strong_count(&extra), count);
        let (retired_chain, retired_token) = others.pop().unwrap();
        owner.retire(retired_token).unwrap();
        assert!(owner.select(retired_token).is_err());
        drop(retired_chain);
        owner
            .create(&factory, &queue, extra, &descriptor())
            .unwrap();
        assert!(owner.selected(&selected).is_ok());
        let (_, still_selected) = owner.selected(&selected).unwrap();
        assert_eq!(
            address(&identity(&still_selected).unwrap()),
            address(&identity(&queue).unwrap())
        );
        owner.retire(token).unwrap();
        assert!(owner.selected(&selected).is_err());
        assert!(owner.select(token).is_err());
        assert!(weak.upgrade().is_none());
        assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
        let extra = Rc::new(Window::new(false, 318, 178).unwrap());
        assert!(owner
            .create(&factory, &queue, extra, &descriptor())
            .unwrap_err()
            .contains("closed"));
        println!("PASS: owned native queue association {}: DIRECT validation, failed creation leaves no entry, canonical identity/explicit selection, unrelated-chain rejection, eight-slot bounds, retirement and retained HWND guard lifetime. Cooperative factory wrapper only; no interception/discovery.", if warp {"WARP"} else {"hardware"});
    }

    #[test]
    #[ignore = "creates only owned HWNDs and real WARP D3D12/DXGI objects"]
    fn queue_owner_warp() {
        lifecycle(true);
    }

    #[test]
    #[ignore = "creates only owned HWNDs and real hardware D3D12/DXGI objects"]
    fn queue_owner_hardware() {
        lifecycle(false);
    }
}
