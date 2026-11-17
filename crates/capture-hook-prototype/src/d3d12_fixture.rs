//! Owned offscreen D3D12 renderer/readback experiment. No hooks or game targeting.
use crate::{fence_slots::FenceSlots, SLOT_COUNT};
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory4};

// One live fixture per process, including any terminal retained GPU allocation.
static ACTIVE: AtomicBool = AtomicBool::new(false);
struct Reservation {
    retain: bool,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.retain {
            ACTIVE.store(false, Ordering::Release);
        }
    }
}

struct Slot {
    target: ID3D12Resource,
    readback: ID3D12Resource,
    rtv: ID3D12DescriptorHeap,
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    consumer: Option<(u64, Weak<[u8]>)>,
    source_lease: Option<ID3D12Resource>,
    submitted_at: Option<Instant>,
}
struct Resources {
    // Retain the device and queue with every in-flight resource on uncertain exit.
    _device: ID3D12Device,
    queue: ID3D12CommandQueue,
    fence: ID3D12Fence,
    slots: Vec<Slot>,
}
pub struct FixtureFrame {
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    pub bgra: Arc<[u8]>,
    pub submitted_at: Instant,
}
pub struct D3d12Fixture {
    resources: Option<Resources>,
    ownership: FenceSlots,
    footprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
    total_bytes: usize,
    width: u32,
    height: u32,
    last_signal: u64,
    uncertain: bool,
    closed: bool,
    reservation: Reservation,
}
fn error(e: windows::core::Error) -> String {
    e.to_string()
}

impl D3d12Fixture {
    pub fn new(warp: bool, width: u32, height: u32) -> Result<Self, String> {
        // Small explicit test budget, not an arbitrary-game resolution contract.
        if width == 0 || height == 0 || width > 1920 || height > 1080 {
            return Err("owned D3D12 fixture requires dimensions within 1920x1080".into());
        }
        ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "owned D3D12 fixture is already active or terminally retained")?;
        let reservation = Reservation { retain: false };
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory1().map_err(error)?;
            let adapter: IDXGIAdapter = if warp {
                factory.EnumWarpAdapter()
            } else {
                factory.EnumAdapters(0)
            }
            .map_err(error)?;
            let mut device: Option<ID3D12Device> = None;
            D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device).map_err(error)?;
            let device = device.ok_or("null D3D12 device")?;
            let queue: ID3D12CommandQueue = device
                .CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                    Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                    ..Default::default()
                })
                .map_err(error)?;
            let fence: ID3D12Fence = device
                .CreateFence(0, D3D12_FENCE_FLAG_NONE)
                .map_err(error)?;
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
                Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                ..Default::default()
            };
            let mut footprint = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
            let mut total = 0;
            let mut rows = 0;
            let mut row_bytes = 0;
            device.GetCopyableFootprints(
                &desc,
                0,
                1,
                0,
                Some(&mut footprint),
                Some(&mut rows),
                Some(&mut row_bytes),
                Some(&mut total),
            );
            if rows != height || row_bytes != u64::from(width) * 4 || total > 16 * 1024 * 1024 {
                return Err("unexpected D3D12 footprint or readback budget".into());
            }
            let mut slots = Vec::with_capacity(SLOT_COUNT);
            for _ in 0..SLOT_COUNT {
                let mut target: Option<ID3D12Resource> = None;
                device
                    .CreateCommittedResource(
                        &D3D12_HEAP_PROPERTIES {
                            Type: D3D12_HEAP_TYPE_DEFAULT,
                            ..Default::default()
                        },
                        D3D12_HEAP_FLAG_NONE,
                        &desc,
                        D3D12_RESOURCE_STATE_RENDER_TARGET,
                        None,
                        &mut target,
                    )
                    .map_err(error)?;
                let target = target.ok_or("null D3D12 target")?;
                let buffer = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Width: total,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    ..Default::default()
                };
                let mut readback: Option<ID3D12Resource> = None;
                device
                    .CreateCommittedResource(
                        &D3D12_HEAP_PROPERTIES {
                            Type: D3D12_HEAP_TYPE_READBACK,
                            ..Default::default()
                        },
                        D3D12_HEAP_FLAG_NONE,
                        &buffer,
                        D3D12_RESOURCE_STATE_COPY_DEST,
                        None,
                        &mut readback,
                    )
                    .map_err(error)?;
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
                slots.push(Slot {
                    target,
                    readback: readback.ok_or("null readback buffer")?,
                    rtv,
                    allocator,
                    list,
                    consumer: None,
                    source_lease: None,
                    submitted_at: None,
                });
            }
            Ok(Self {
                resources: Some(Resources {
                    _device: device,
                    queue,
                    fence,
                    slots,
                }),
                ownership: FenceSlots::default(),
                footprint,
                total_bytes: total as usize,
                width,
                height,
                last_signal: 0,
                uncertain: false,
                closed: false,
                reservation,
            })
        }
    }

    fn reap(&mut self) {
        for (index, slot) in self
            .resources
            .as_mut()
            .unwrap()
            .slots
            .iter_mut()
            .enumerate()
        {
            if slot
                .consumer
                .as_ref()
                .is_some_and(|(_, weak)| weak.strong_count() == 0)
            {
                let (sequence, _) = slot.consumer.take().unwrap();
                assert!(self.ownership.release(index, sequence));
            }
        }
    }
    /// No fence wait or CPU map. Returns false when all three slots are occupied.
    pub fn submit_color(&mut self, rgba: [f32; 4]) -> Result<bool, String> {
        if self.closed {
            return Err("D3D12 fixture is closed".into());
        }
        if !rgba
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        {
            return Err("invalid fixture color".into());
        }
        self.submit(Some(rgba), None)
    }

    pub(crate) fn device_queue(&self) -> (ID3D12Device, ID3D12CommandQueue) {
        let resources = self.resources.as_ref().unwrap();
        (resources._device.clone(), resources.queue.clone())
    }

    /// Only the owned presentation fixture may call this. It creates the source
    /// on this device/queue, orders its render before this copy, and owns PRESENT.
    pub(crate) fn copy_owned_present_target(
        &mut self,
        target: &ID3D12Resource,
    ) -> Result<bool, String> {
        let desc = unsafe { target.GetDesc() };
        if desc.Width != u64::from(self.width)
            || desc.Height != self.height
            || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            || desc.SampleDesc.Count != 1
        {
            self.cancel();
            return Err("owned swap-chain format changed; capture is terminal".into());
        }
        self.submit(None, Some(target))
    }

    fn submit(
        &mut self,
        color: Option<[f32; 4]>,
        source: Option<&ID3D12Resource>,
    ) -> Result<bool, String> {
        if self.closed {
            return Err("D3D12 fixture is closed".into());
        }
        self.reap();
        let Some((index, value)) = self.ownership.reserve() else {
            return Ok(false);
        };
        let resources = self.resources.as_mut().unwrap();
        let slot = &mut resources.slots[index];
        slot.source_lease = source.cloned();
        let target = slot.source_lease.as_ref().unwrap_or(&slot.target);
        let resting_state = if source.is_some() {
            D3D12_RESOURCE_STATE_PRESENT
        } else {
            D3D12_RESOURCE_STATE_RENDER_TARGET
        };
        let result = unsafe {
            (|| {
                slot.allocator.Reset().map_err(error)?;
                slot.list.Reset(&slot.allocator, None).map_err(error)?;
                if let Some(rgba) = color {
                    slot.list.ClearRenderTargetView(
                        slot.rtv.GetCPUDescriptorHandleForHeapStart(),
                        &rgba,
                        None,
                    );
                }
                barrier(
                    &slot.list,
                    target,
                    resting_state,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                );
                let mut source = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: ManuallyDrop::new(Some(target.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        SubresourceIndex: 0,
                    },
                };
                let mut destination = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: ManuallyDrop::new(Some(slot.readback.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        PlacedFootprint: self.footprint,
                    },
                };
                slot.list
                    .CopyTextureRegion(&destination, 0, 0, 0, &source, None);
                ManuallyDrop::drop(&mut source.pResource);
                ManuallyDrop::drop(&mut destination.pResource);
                barrier(
                    &slot.list,
                    target,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                    resting_state,
                );
                slot.list.Close().map_err(error)?;
                let command: ID3D12CommandList = slot.list.cast().map_err(error)?;
                // Any error after execution must retain resources until proven complete.
                self.uncertain = true;
                slot.submitted_at = Some(Instant::now());
                resources.queue.ExecuteCommandLists(&[Some(command)]);
                resources
                    .queue
                    .Signal(&resources.fence, value)
                    .map_err(error)?;
                self.last_signal = value;
                self.uncertain = false;
                Ok::<_, String>(true)
            })()
        };
        if result.is_err() {
            self.cancel();
        }
        result
    }

    pub fn poll(&mut self) -> Result<Option<FixtureFrame>, String> {
        if self.closed {
            return Err("D3D12 fixture is closed".into());
        }
        let result = self.poll_inner();
        if result.is_err() {
            self.cancel();
        }
        result
    }
    fn poll_inner(&mut self) -> Result<Option<FixtureFrame>, String> {
        let resources = self.resources.as_mut().unwrap();
        let completed = unsafe { resources.fence.GetCompletedValue() };
        if completed == u64::MAX {
            return Err("D3D12 device removed; no publication".into());
        }
        let Some((index, sequence)) = self.ownership.ready(completed) else {
            return Ok(None);
        };
        let slot = &mut resources.slots[index];
        let mut data = std::ptr::null_mut();
        unsafe {
            slot.readback.Map(
                0,
                Some(&D3D12_RANGE {
                    Begin: 0,
                    End: self.total_bytes,
                }),
                Some(&mut data),
            )
        }
        .map_err(error)?;
        if data.is_null() {
            return Err("null mapped D3D12 readback".into());
        }
        let mut pixels = vec![0u8; self.width as usize * self.height as usize * 4];
        unsafe {
            let source = std::slice::from_raw_parts(data.cast::<u8>(), self.total_bytes);
            let row_bytes = self.width as usize * 4;
            for row in 0..self.height as usize {
                let begin = self.footprint.Offset as usize
                    + row * self.footprint.Footprint.RowPitch as usize;
                pixels[row * row_bytes..(row + 1) * row_bytes]
                    .copy_from_slice(&source[begin..begin + row_bytes]);
            }
            slot.readback
                .Unmap(0, Some(&D3D12_RANGE { Begin: 0, End: 0 }));
        }
        let bgra: Arc<[u8]> = pixels.into();
        assert!(self.ownership.lease(index, sequence));
        slot.consumer = Some((sequence, Arc::downgrade(&bgra)));
        Ok(Some(FixtureFrame {
            sequence,
            width: self.width,
            height: self.height,
            bgra,
            submitted_at: slot
                .submitted_at
                .ok_or("capture submission timestamp missing")?,
        }))
    }
    pub fn cancel(&mut self) {
        self.closed = true;
        self.ownership.close();
    }

    // Control/teardown only, never called in rendering or capture submission.
    pub(crate) fn drain(&self) -> bool {
        if self.uncertain {
            return false;
        }
        let resources = self.resources.as_ref().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let completed = unsafe { resources.fence.GetCompletedValue() };
            if completed != u64::MAX && completed >= self.last_signal {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub(crate) fn release_drained_sources(&mut self) -> Result<(), String> {
        if !self.closed || !self.drain() {
            return Err("capture must be closed and drained before source release".into());
        }
        for slot in &mut self.resources.as_mut().unwrap().slots {
            // ResizeBuffers also needs recorded source references retired.
            unsafe {
                slot.allocator.Reset().map_err(error)?;
                slot.list.Reset(&slot.allocator, None).map_err(error)?;
                slot.list.Close().map_err(error)?;
            }
            slot.source_lease = None;
        }
        Ok(())
    }

    pub(crate) fn retain_until_exit(&mut self) {
        self.cancel();
        self.uncertain = true;
    }
    pub(crate) fn device_removed(&self) -> bool {
        unsafe {
            self.resources
                .as_ref()
                .unwrap()
                ._device
                .GetDeviceRemovedReason()
        }
        .is_err()
    }
}

pub(crate) unsafe fn barrier(
    list: &ID3D12GraphicsCommandList,
    target: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) {
    let mut barrier = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: ManuallyDrop::new(Some(target.clone())),
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: before,
                StateAfter: after,
            }),
        },
    };
    list.ResourceBarrier(std::slice::from_ref(&barrier));
    ManuallyDrop::drop(&mut (*barrier.Anonymous.Transition).pResource);
}
impl Drop for D3d12Fixture {
    fn drop(&mut self) {
        self.cancel();
        let removed = self.device_removed();
        let resources = self.resources.take().unwrap();
        if removed {
            return;
        } // Invalid device graph: never a successful fence drain.
        let deadline = Instant::now() + Duration::from_secs(2);
        if !self.uncertain {
            loop {
                let completed = unsafe { resources.fence.GetCompletedValue() };
                if completed != u64::MAX && completed >= self.last_signal {
                    return;
                }
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        // Never free objects still potentially referenced by submitted GPU work.
        // Caller must treat this as terminal, not start an unbounded retry loop.
        eprintln!("D3D12 fixture completion uncertain: retaining GPU resources until process exit");
        self.reservation.retain = true;
        std::mem::forget(resources);
    }
}
