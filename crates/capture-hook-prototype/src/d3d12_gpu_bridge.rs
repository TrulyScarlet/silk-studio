//! In-process owned D3D12 -> D3D11 shared-texture/fence interop. No hooks or IPC.
use crate::d3d12_fixture::barrier;
use crate::local_gpu_bridge::{GpuPoolBudget, LocalGpuBridge};
use crate::publication_pool::Publication;
use crate::shared_fence_slots::SharedFenceSlots;
use crate::SLOT_COUNT;
use encoder_api::GpuFrameContext;
use gpu_windows::{GpuDeviceContext, GpuTextureSlot};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::{GENERIC_ALL, HANDLE, HMODULE};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIDevice, IDXGIFactory4,
};

fn error(e: windows::core::Error) -> String {
    e.to_string()
}
pub(crate) fn consumer_device(
    luid: windows::Win32::Foundation::LUID,
    warp: bool,
) -> Result<Arc<GpuDeviceContext>, String> {
    unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory1().map_err(error)?;
        let adapter: IDXGIAdapter = factory.EnumAdapterByLuid(luid).map_err(error)?;
        let mut device = None;
        let mut context = None;
        D3D11CreateDevice(
            if warp { None } else { Some(&adapter) },
            if warp {
                D3D_DRIVER_TYPE_WARP
            } else {
                D3D_DRIVER_TYPE_UNKNOWN
            },
            HMODULE::default(),
            if warp {
                D3D11_CREATE_DEVICE_BGRA_SUPPORT
            } else {
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT
            },
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|e| format!("D3D11 interop device creation failed: {e}"))?;
        let device = device.ok_or("null interop D3D11 device")?;
        let context = context.ok_or("null interop D3D11 context")?;
        let dxgi: IDXGIDevice = device.cast().map_err(error)?;
        let actual = dxgi
            .GetAdapter()
            .map_err(error)?
            .GetDesc()
            .map_err(error)?
            .AdapterLuid;
        if (actual.LowPart, actual.HighPart) != (luid.LowPart, luid.HighPart) {
            return Err("D3D11/D3D12 adapter mismatch".into());
        }
        let mt: ID3D11Multithread = context.cast().map_err(error)?;
        let _ = mt.SetMultithreadProtected(true);
        Ok(Arc::new(GpuDeviceContext::new(device, context)))
    }
}
pub(crate) fn handle<T: Interface>(
    device: &ID3D12Device,
    object: &T,
) -> Result<OwnedHandle, String> {
    let child: ID3D12DeviceChild = object.cast().map_err(error)?;
    let handle =
        unsafe { device.CreateSharedHandle(&child, None, GENERIC_ALL.0, None) }.map_err(error)?;
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}
fn open_fence(
    device12: &ID3D12Device,
    device11: &ID3D11Device5,
    fence: &ID3D12Fence,
) -> Result<ID3D11Fence, String> {
    let handle = handle(device12, fence)?;
    let mut opened = None;
    unsafe { device11.OpenSharedFence(HANDLE(handle.as_raw_handle()), &mut opened) }
        .map_err(error)?;
    opened.ok_or("null D3D11 shared fence".into())
}
struct Slot {
    // Rust drops fields in declaration order. Keep both devices/queues alive
    // until command lists, imported wrappers and shared resources are gone.
    list: ID3D12GraphicsCommandList,
    allocator: ID3D12CommandAllocator,
    source: Option<ID3D12Resource>,
    imported: GpuTextureSlot,
    texture: ID3D12Resource,
    submitted_at: Option<Instant>,
    qpc_ticks: i64,
}
struct Resources {
    slots: Vec<Slot>,
    output: Arc<LocalGpuBridge>,
    context4: ID3D11DeviceContext4,
    produced11: ID3D11Fence,
    consumed11: ID3D11Fence,
    retired11: ID3D11Fence,
    produced: ID3D12Fence,
    consumed: ID3D12Fence,
    queue: ID3D12CommandQueue,
    device11: Arc<GpuDeviceContext>,
}
pub struct GpuInteropFrame {
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    pub submitted_at: Instant,
    pub publication: Publication<GpuTextureSlot>,
}
#[derive(Default, Clone, Copy, Debug)]
pub struct GpuInteropStats {
    pub submitted: u64,
    pub gpu_copies_to_output: u64,
    pub dropped_local_full: u64,
    pub cpu_readbacks: u64,
}
pub(crate) struct D3d12GpuBridge {
    resources: Option<Resources>,
    ledger: SharedFenceSlots,
    dimensions: (u32, u32),
    last_produced: u64,
    last_consumed: u64,
    uncertain: bool,
    consumer_work_exposed: bool,
    consumer_uncertain: bool,
    closed: bool,
    stats: GpuInteropStats,
    #[cfg(feature = "d3d12-cross-process")]
    remote: Option<(crate::d3d12_protocol::GpuHello, [OwnedHandle; 5])>,
    #[cfg(feature = "d3d12-cross-process")]
    remote_retired: bool,
}
impl D3d12GpuBridge {
    pub(crate) fn new(
        device12: &ID3D12Device,
        queue: &ID3D12CommandQueue,
        width: u32,
        height: u32,
        budget: Arc<GpuPoolBudget>,
        warp: bool,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 || width > 1920 || height > 1080 {
            return Err("invalid owned GPU bridge dimensions".into());
        }
        unsafe {
            let device11 = consumer_device(device12.GetAdapterLuid(), warp)?;
            let context4: ID3D11DeviceContext4 = device11.context().cast().map_err(error)?;
            let device5: ID3D11Device5 = device11.device().cast().map_err(error)?;
            let device1: ID3D11Device1 = device11.device().cast().map_err(error)?;
            let output = LocalGpuBridge::new(device11.clone(), width, height, budget)?;
            let produced: ID3D12Fence = device12
                .CreateFence(0, D3D12_FENCE_FLAG_SHARED)
                .map_err(error)?;
            let consumed: ID3D12Fence = device12
                .CreateFence(0, D3D12_FENCE_FLAG_SHARED)
                .map_err(error)?;
            let produced11 = open_fence(device12, &device5, &produced)?;
            let consumed11 = open_fence(device12, &device5, &consumed)?;
            // Independent of D3D12 removal signaling on the shared fences.
            let mut retired11 = None;
            device5
                .CreateFence(0, D3D11_FENCE_FLAG_NONE, &mut retired11)
                .map_err(error)?;
            let retired11 = retired11.ok_or("null local consumer retirement fence")?;
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Width: u64::from(width),
                Height: height,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
                    | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
                ..Default::default()
            };
            let mut slots = Vec::with_capacity(SLOT_COUNT);
            for _ in 0..SLOT_COUNT {
                let mut texture = None;
                device12
                    .CreateCommittedResource(
                        &D3D12_HEAP_PROPERTIES {
                            Type: D3D12_HEAP_TYPE_DEFAULT,
                            ..Default::default()
                        },
                        D3D12_HEAP_FLAG_SHARED,
                        &desc,
                        D3D12_RESOURCE_STATE_COMMON,
                        None,
                        &mut texture,
                    )
                    .map_err(error)?;
                let texture: ID3D12Resource = texture.ok_or("null D3D12 shared texture")?;
                let shared = handle(device12, &texture)?;
                let imported: ID3D11Texture2D = device1
                    .OpenSharedResource1(HANDLE(shared.as_raw_handle()))
                    .map_err(error)?;
                let allocator: ID3D12CommandAllocator = device12
                    .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                    .map_err(error)?;
                let list: ID3D12GraphicsCommandList = device12
                    .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
                    .map_err(error)?;
                list.Close().map_err(error)?;
                slots.push(Slot {
                    texture,
                    imported: GpuTextureSlot::new(imported, device11.clone()),
                    allocator,
                    list,
                    source: None,
                    submitted_at: None,
                    qpc_ticks: 0,
                });
            }
            Ok(Self {
                resources: Some(Resources {
                    queue: queue.clone(),
                    produced,
                    consumed,
                    produced11,
                    consumed11,
                    retired11,
                    context4,
                    device11,
                    output,
                    slots,
                }),
                ledger: SharedFenceSlots::default(),
                dimensions: (width, height),
                last_produced: 0,
                last_consumed: 0,
                uncertain: false,
                consumer_work_exposed: false,
                consumer_uncertain: false,
                closed: false,
                stats: GpuInteropStats::default(),
                #[cfg(feature = "d3d12-cross-process")]
                remote: None,
                #[cfg(feature = "d3d12-cross-process")]
                remote_retired: false,
            })
        }
    }
    pub(crate) fn context(&self) -> GpuFrameContext {
        self.resources.as_ref().unwrap().output.context()
    }
    pub(crate) fn stats(&self) -> GpuInteropStats {
        self.stats
    }
    pub(crate) fn submit(&mut self, source: &ID3D12Resource) -> Result<bool, String> {
        if self.closed {
            return Err("GPU bridge closed".into());
        }
        let result = self.submit_inner(source);
        if result.is_err() {
            // May run in the owned Present callback. Close admission only;
            // controller-side cancellation retires the mutex-protected output.
            self.closed = true;
            self.ledger.close();
        }
        result
    }
    fn submit_inner(&mut self, source: &ID3D12Resource) -> Result<bool, String> {
        let desc = unsafe { source.GetDesc() };
        if (desc.Width, desc.Height) != (u64::from(self.dimensions.0), self.dimensions.1)
            || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            || desc.SampleDesc.Count != 1
        {
            return Err("owned GPU source format changed".into());
        }
        let resources = self.resources.as_mut().unwrap();
        let consumed = unsafe { resources.consumed.GetCompletedValue() };
        let Some((index, value)) = self.ledger.reserve(consumed)? else {
            return Ok(false);
        };
        let slot = &mut resources.slots[index];
        slot.source = Some(source.clone());
        unsafe {
            slot.allocator.Reset().map_err(error)?;
            slot.list.Reset(&slot.allocator, None).map_err(error)?;
            barrier(
                &slot.list,
                source,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            barrier(
                &slot.list,
                &slot.texture,
                D3D12_RESOURCE_STATE_COMMON,
                D3D12_RESOURCE_STATE_COPY_DEST,
            );
            slot.list.CopyResource(&slot.texture, source);
            barrier(
                &slot.list,
                &slot.texture,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_COMMON,
            );
            barrier(
                &slot.list,
                source,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_PRESENT,
            );
            slot.list.Close().map_err(error)?;
            let command: ID3D12CommandList = slot.list.cast().map_err(error)?;
            self.uncertain = true;
            slot.submitted_at = Some(Instant::now());
            windows::Win32::System::Performance::QueryPerformanceCounter(&mut slot.qpc_ticks)
                .map_err(error)?;
            resources.queue.ExecuteCommandLists(&[Some(command)]);
            resources
                .queue
                .Signal(&resources.produced, value)
                .map_err(error)?;
            self.last_produced = value;
            self.uncertain = false;
        }
        self.stats.submitted += 1;
        Ok(true)
    }
    pub(crate) fn poll(&mut self) -> Result<Option<GpuInteropFrame>, String> {
        #[cfg(feature = "d3d12-cross-process")]
        if self.remote.is_some() {
            return Err("remote ring cannot be consumed locally".into());
        }
        if self.closed {
            return Err("GPU bridge closed".into());
        }
        let result = self.poll_inner();
        if result.is_err() {
            self.cancel();
        }
        result
    }
    fn poll_inner(&mut self) -> Result<Option<GpuInteropFrame>, String> {
        let resources = self.resources.as_mut().unwrap();
        let produced = unsafe { resources.produced.GetCompletedValue() };
        let Some((index, sequence)) = self.ledger.ready(produced)? else {
            return Ok(None);
        };
        let slot = &resources.slots[index];
        // Producer fence is already complete: this GPU-side wait cannot create a
        // dependency on unsubmitted producer work or block the CPU render path.
        self.uncertain = true;
        let consumer_exposed = &mut self.consumer_work_exposed;
        let consumer_uncertain = &mut self.consumer_uncertain;
        let Some(publication) =
            resources
                .output
                .copy_imported_after(&slot.imported, || unsafe {
                    *consumer_exposed = true;
                    *consumer_uncertain = true;
                    resources
                        .context4
                        .Wait(&resources.produced11, sequence)
                        .map_err(error)
                })?
        else {
            self.uncertain = false;
            assert!(self.ledger.discard(index, sequence, produced));
            self.stats.dropped_local_full += 1;
            return Ok(None);
        };
        unsafe {
            resources
                .context4
                .Signal(&resources.consumed11, sequence)
                .map_err(error)?;
            resources
                .context4
                .Signal(&resources.retired11, sequence)
                .map_err(error)?;
            resources.device11.context().Flush();
        }
        self.last_consumed = sequence;
        self.consumer_uncertain = false;
        self.uncertain = false;
        assert!(self.ledger.consuming(index, sequence));
        self.stats.gpu_copies_to_output += 1;
        Ok(Some(GpuInteropFrame {
            sequence,
            width: self.dimensions.0,
            height: self.dimensions.1,
            submitted_at: slot
                .submitted_at
                .ok_or("missing GPU submission timestamp")?,
            publication,
        }))
    }
    pub(crate) fn cancel(&mut self) {
        self.closed = true;
        self.ledger.close();
        self.resources.as_ref().unwrap().output.retire();
    }
    pub(crate) fn drain(&self) -> bool {
        if self.uncertain {
            return false;
        }
        let resources = self.resources.as_ref().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let produced = unsafe { resources.produced.GetCompletedValue() };
            let consumed = unsafe { resources.consumed.GetCompletedValue() };
            if produced != u64::MAX
                && consumed != u64::MAX
                && produced >= self.last_produced
                && consumed >= self.last_consumed
            {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    pub(crate) fn removed_graph_releasable(&self) -> bool {
        #[cfg(feature = "d3d12-cross-process")]
        let remote = self.remote.is_some() && !self.remote_retired;
        #[cfg(not(feature = "d3d12-cross-process"))]
        let remote = false;
        let local_completed = unsafe {
            self.resources
                .as_ref()
                .unwrap()
                .retired11
                .GetCompletedValue()
        };
        let local_unretired = !local_consumer_retired(
            self.consumer_work_exposed,
            self.consumer_uncertain,
            self.last_consumed,
            local_completed,
        );
        if local_unretired || remote {
            return false;
        }
        let mut device: Option<ID3D12Device> = None;
        if unsafe {
            self.resources
                .as_ref()
                .unwrap()
                .queue
                .GetDevice(&mut device)
        }
        .is_err()
        {
            return false;
        }
        permits_removal_only_release(
            device.is_some_and(|device| unsafe { device.GetDeviceRemovedReason() }.is_err()),
            local_unretired,
            remote,
        )
    }
    pub(crate) fn release_sources(&mut self) -> Result<(), String> {
        if self.closed && self.removed_graph_releasable() {
            // Removed command lists cannot be Reset. Their owning graph is
            // dropped next; independent consumer retirement was proved above.
            for slot in &mut self.resources.as_mut().unwrap().slots {
                slot.source = None;
            }
            return Ok(());
        }
        if !self.closed || !self.drain() {
            return Err("GPU source release requires closed/drained bridge".into());
        }
        for slot in &mut self.resources.as_mut().unwrap().slots {
            unsafe {
                slot.allocator.Reset().map_err(error)?;
                slot.list.Reset(&slot.allocator, None).map_err(error)?;
                slot.list.Close().map_err(error)?;
            }
            slot.source = None;
        }
        Ok(())
    }
    #[cfg(feature = "d3d12-cross-process")]
    pub(crate) fn export_hello(
        &mut self,
        device: &ID3D12Device,
        session: crate::SessionIdentity,
    ) -> Result<crate::d3d12_protocol::GpuHello, String> {
        if self.closed || self.stats.submitted != 0 || self.remote.is_some() {
            return Err("remote export requires a fresh unused ring".into());
        }
        let resources = self.resources.as_ref().unwrap();
        let mut handles = Vec::with_capacity(5);
        for slot in &resources.slots {
            handles.push(handle(device, &slot.texture)?);
        }
        handles.push(handle(device, &resources.produced)?);
        handles.push(handle(device, &resources.consumed)?);
        let handles: [OwnedHandle; 5] = handles
            .try_into()
            .map_err(|_| "invalid export handle count")?;
        let luid = unsafe { device.GetAdapterLuid() };
        let mut frequency = 0;
        unsafe { windows::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency) }
            .map_err(error)?;
        let hello = crate::d3d12_protocol::GpuHello {
            session,
            adapter_luid: (u64::from(luid.HighPart as u32) << 32) | u64::from(luid.LowPart),
            qpc_frequency: frequency as u64,
            width: self.dimensions.0,
            height: self.dimensions.1,
            handles: handles.each_ref().map(|h| h.as_raw_handle() as u64),
        };
        hello
            .validate()
            .map_err(|e| format!("invalid export greeting: {e:?}"))?;
        self.remote = Some((hello, handles));
        Ok(hello)
    }
    #[cfg(feature = "d3d12-cross-process")]
    pub(crate) fn export_frame(&mut self) -> Result<Option<crate::FrameHeader>, String> {
        if self.closed || self.remote_retired {
            return Err("remote ring closed".into());
        }
        let session = self.remote.as_ref().ok_or("ring not exported")?.0.session;
        let resources = self.resources.as_ref().unwrap();
        let completed = unsafe { resources.produced.GetCompletedValue() };
        let Some((index, sequence)) = self.ledger.ready(completed)? else {
            return Ok(None);
        };
        assert!(self.ledger.publish_remote(index, sequence));
        Ok(Some(crate::FrameHeader {
            version: crate::PROTOCOL_VERSION,
            session,
            slot: index as u32,
            sequence,
            qpc_ticks: resources.slots[index].qpc_ticks,
            width: self.dimensions.0,
            height: self.dimensions.1,
        }))
    }
    #[cfg(feature = "d3d12-cross-process")]
    pub(crate) fn remote_ack(
        &mut self,
        session: crate::SessionIdentity,
        slot: u32,
        sequence: u64,
    ) -> Result<(), String> {
        if self.closed
            || self.remote_retired
            || self
                .remote
                .as_ref()
                .is_none_or(|(h, _)| h.session != session)
            || sequence <= self.last_consumed
            || !self.ledger.acknowledge_remote(slot as usize, sequence)
        {
            self.cancel();
            return Err("stale or mismatched remote GPU ACK".into());
        }
        // ACK may arrive before GPU completion; reserve still checks the fence.
        self.last_consumed = sequence;
        Ok(())
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn remote_primed(&self) -> bool {
        self.remote.is_some() && !self.remote_retired && self.last_consumed != 0
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn approve_remote_retirement(
        &mut self,
        session: crate::SessionIdentity,
    ) -> Result<(), String> {
        if self.last_consumed == 0 {
            return Err("pending-copy removal before any exact remote ACK is unsupported; no removal attempted".into());
        }
        if self.remote_retired
            || self
                .remote
                .as_ref()
                .is_none_or(|(hello, _)| hello.session != session)
        {
            return Err("remote retirement requires the exact exported session once".into());
        }
        self.remote_retired = true;
        Ok(())
    }
    #[cfg(feature = "d3d12-device-replace-fixture")]
    pub(crate) fn probe_pending_removal(
        &mut self,
        device: &ID3D12Device,
        chain: &windows::Win32::Graphics::Dxgi::IDXGISwapChain3,
    ) -> Result<(), String> {
        if !self.remote_retired || self.closed {
            return Err(
                "pending removal probe requires retired remote consumer and open private producer"
                    .into(),
            );
        }
        let removable: ID3D12Device5 = device.cast().map_err(error)?;
        // The controlled probe removes one deliberately pending copy, not
        // unrelated startup/render work. Prove the earlier queue tail retired
        // before introducing the gate; never replace this with a timing sleep.
        let prior: ID3D12Fence =
            unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.map_err(error)?;
        unsafe { self.resources.as_ref().unwrap().queue.Signal(&prior, 1) }.map_err(error)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let value = unsafe { prior.GetCompletedValue() };
            if value == u64::MAX {
                return Err("device removed before controlled pending-copy probe".into());
            }
            if value >= 1 {
                break;
            }
            if Instant::now() >= deadline {
                return Err("prior queue tail did not retire before controlled probe".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let gate: ID3D12Fence =
            unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.map_err(error)?;
        struct ReleaseWait(ID3D12Fence);
        impl Drop for ReleaseWait {
            fn drop(&mut self) {
                let _ = unsafe { self.0.Signal(1) };
            }
        }
        let release = ReleaseWait(gate.clone());
        let source: ID3D12Resource =
            unsafe { chain.GetBuffer(chain.GetCurrentBackBufferIndex()) }.map_err(error)?;
        unsafe { self.resources.as_ref().unwrap().queue.Wait(&gate, 1) }.map_err(error)?;
        if !self.submit(&source)? {
            return Err("pending removal probe had no free producer slot".into());
        }
        let completed = unsafe {
            self.resources
                .as_ref()
                .unwrap()
                .produced
                .GetCompletedValue()
        };
        if completed == u64::MAX || completed >= self.last_produced {
            return Err("pending capture-copy proof missing".into());
        }
        unsafe { removable.RemoveDevice() };
        if unsafe { device.GetDeviceRemovedReason() }.is_ok()
            || unsafe {
                self.resources
                    .as_ref()
                    .unwrap()
                    .produced
                    .GetCompletedValue()
            } != u64::MAX
        {
            return Err("owned pending-copy removal did not invalidate device".into());
        }
        self.cancel();
        drop(release);
        eprintln!("owned remote epoch retired; actual unpublished capture copy pending at explicit removal; no completion claim for UINT64_MAX");
        Ok(())
    }
}
fn permits_removal_only_release(removed: bool, consumer: bool, remote: bool) -> bool {
    removed && !consumer && !remote
}
fn local_consumer_retired(exposed: bool, uncertain: bool, last: u64, completed: u64) -> bool {
    !exposed || (!uncertain && last != 0 && completed != u64::MAX && completed >= last)
}
impl Drop for D3d12GpuBridge {
    fn drop(&mut self) {
        self.cancel();
        // Confirmed device removal permits releasing the invalid producer graph,
        // not interpreting UINT64_MAX as a completed copy. This narrow path is
        // unavailable with unretired local work or remote handle export.
        let releasable =
            self.removed_graph_releasable() || self.drain() || self.removed_graph_releasable();
        #[cfg(feature = "d3d12-cross-process")]
        if releasable && self.remote_retired {
            self.remote.take();
        }
        let resources = self.resources.take().unwrap();
        if !releasable {
            eprintln!("D3D12/D3D11 interop completion uncertain; retaining resources until exit");
            std::mem::forget(resources);
        }
    }
}

#[cfg(test)]
mod removal_tests {
    #[test]
    fn removal_only_release_requires_no_unretired_consumer_or_remote_exposure() {
        for removed in [false, true] {
            for consumer in [false, true] {
                for remote in [false, true] {
                    assert_eq!(
                        super::permits_removal_only_release(removed, consumer, remote),
                        removed && !consumer && !remote
                    );
                }
            }
        }
    }
    #[test]
    fn local_retirement_never_accepts_shared_removal_sentinel_or_uncertain_signal() {
        assert!(super::local_consumer_retired(false, false, 0, 0));
        for (uncertain, last, completed) in [
            (false, 1, 0),
            (false, 2, 1),
            (false, 1, u64::MAX),
            (false, 0, 5),
            (true, 1, 1),
        ] {
            assert!(!super::local_consumer_retired(
                true, uncertain, last, completed
            ));
        }
        assert!(super::local_consumer_retired(true, false, 1, 1));
        assert!(super::local_consumer_retired(true, false, 2, 3));
    }
}
