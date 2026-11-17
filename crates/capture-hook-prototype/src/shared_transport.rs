//! Cooperative fixture GPU sharing. All operations are outside Present.

use std::marker::PhantomData;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpu_windows::{GpuDeviceContext, GpuTextureSlot};
use windows::core::Interface;
use windows::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_SAME_ACCESS, FILETIME, HANDLE, HMODULE, HWND,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11Device1, ID3D11Multithread, ID3D11Texture2D,
    D3D11_BIND_RENDER_TARGET, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIDevice, IDXGIFactory1, IDXGIKeyedMutex, IDXGIResource1,
    DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE,
};
use windows::Win32::System::Performance::QueryPerformanceFrequency;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessId, GetProcessTimes};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};

use crate::native::CapturedFrame;
use crate::transport::{keyed_mutex_acquired, Hello, Message, SlotLedger};
use crate::{FrameHeader, SessionGate, SessionIdentity, SLOT_COUNT};

fn error(error: windows::core::Error) -> String {
    error.to_string()
}

pub fn adapter_luid(device: &ID3D11Device) -> Result<u64, String> {
    let dxgi: IDXGIDevice = device.cast().map_err(error)?;
    let adapter = unsafe { dxgi.GetAdapter() }.map_err(error)?;
    let desc = unsafe { adapter.GetDesc() }.map_err(error)?;
    Ok((u64::from(desc.AdapterLuid.HighPart as u32) << 32) | u64::from(desc.AdapterLuid.LowPart))
}

fn acquire(mutex: &IDXGIKeyedMutex, key: u64, timeout_ms: u32) -> Result<(), String> {
    // Bypass generated .ok(): WAIT_TIMEOUT and WAIT_ABANDONED are positive.
    let status = unsafe { (mutex.vtable().AcquireSync)(mutex.as_raw(), key, timeout_ms) };
    if keyed_mutex_acquired(status.0) {
        Ok(())
    } else {
        Err(format!(
            "shared texture ownership was not acquired (HRESULT {:#x})",
            status.0
        ))
    }
}

struct ExportSurface {
    texture: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
    handle: OwnedHandle,
}

pub struct ProducerRing {
    device: Arc<GpuDeviceContext>,
    surfaces: [ExportSurface; SLOT_COUNT],
    hello: Hello,
    ledger: SlotLedger,
    failed: bool,
}

impl ProducerRing {
    pub fn new(
        device: Arc<GpuDeviceContext>,
        session: SessionIdentity,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let ledger = SlotLedger::new(session).ok_or("invalid shared-ring identity")?;
        let mut frequency = 0;
        unsafe { QueryPerformanceFrequency(&mut frequency) }.map_err(error)?;
        let mut hello = Hello {
            session,
            adapter_luid: adapter_luid(device.device())?,
            qpc_frequency: frequency as u64,
            width,
            height,
            handles: [4, 8, 12],
        };
        // Use the same bounded field validation as the wire reader before allocating.
        Message::decode(&Message::Hello(hello).encode())
            .map_err(|error| format!("invalid sharing parameters: {error:?}"))?;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            MiscFlags: (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0
                | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0) as u32,
            ..Default::default()
        };
        let mut surfaces = Vec::with_capacity(SLOT_COUNT);
        for _ in 0..SLOT_COUNT {
            let mut texture = None;
            unsafe {
                device
                    .device()
                    .CreateTexture2D(&desc, None, Some(&mut texture))
            }
            .map_err(error)?;
            let texture = texture.ok_or("null shared texture")?;
            let resource: IDXGIResource1 = texture.cast().map_err(error)?;
            let mutex: IDXGIKeyedMutex = texture.cast().map_err(error)?;
            let handle = unsafe {
                resource.CreateSharedHandle(
                    None,
                    DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0,
                    None,
                )
            }
            .map_err(error)?;
            // CreateSharedHandle returns a new, non-inherited, unnamed NT handle.
            let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
            surfaces.push(ExportSurface {
                texture,
                mutex,
                handle,
            });
        }
        let surfaces: [ExportSurface; SLOT_COUNT] = surfaces
            .try_into()
            .map_err(|_| "invalid shared-ring slot count")?;
        hello.handles = surfaces
            .each_ref()
            .map(|surface| surface.handle.as_raw_handle() as u64);
        Ok(Self {
            device,
            surfaces,
            hello,
            ledger,
            failed: false,
        })
    }

    pub fn hello(&self) -> Hello {
        self.hello
    }

    pub fn export(&mut self, frame: &CapturedFrame) -> Result<Option<FrameHeader>, String> {
        if self.failed {
            return Err("shared producer transport is closed".into());
        }
        let result = self.export_inner(frame);
        if result.is_err() {
            self.failed = true;
            self.ledger.close();
        }
        result
    }

    fn export_inner(&mut self, frame: &CapturedFrame) -> Result<Option<FrameHeader>, String> {
        if frame.header.session != self.hello.session
            || (frame.header.width, frame.header.height) != (self.hello.width, self.hello.height)
        {
            self.ledger.close();
            return Err("capture identity or dimensions changed; shared ring must retire".into());
        }
        let Some(slot) = self.ledger.available() else {
            return Ok(None);
        };
        let surface = &self.surfaces[slot];
        acquire(&surface.mutex, 0, 0)?;
        unsafe {
            self.device
                .context()
                .CopyResource(&surface.texture, frame.surface.texture());
            self.device.context().Flush();
        }
        unsafe { surface.mutex.ReleaseSync(1) }.map_err(error)?;
        let header = FrameHeader {
            slot: slot as u32,
            ..frame.header
        };
        self.ledger
            .publish(header)
            .map_err(|error| format!("shared-ring publication rejected: {error:?}"))?;
        Ok(Some(header))
    }

    pub fn acknowledge(
        &mut self,
        session: SessionIdentity,
        slot: u32,
        sequence: u64,
    ) -> Result<(), String> {
        let result = self
            .ledger
            .acknowledge(session, slot, sequence)
            .map_err(|error| format!("shared-ring acknowledgement rejected: {error:?}"));
        if result.is_err() {
            self.failed = true;
            self.ledger.close();
        }
        result
    }
}

struct ImportSurface {
    surface: GpuTextureSlot,
    mutex: IDXGIKeyedMutex,
    leased: AtomicBool,
}

pub struct ConsumerRing {
    surfaces: [Arc<ImportSurface>; SLOT_COUNT],
    gate: SessionGate,
    hello: Hello,
    valid: Arc<AtomicBool>,
    _owner: PhantomData<Rc<()>>,
}

pub struct ImportedFrame {
    pub header: FrameHeader,
    surface: Arc<ImportSurface>,
    valid: Arc<AtomicBool>,
    released: bool,
    _owner: PhantomData<Rc<()>>,
}

impl ImportedFrame {
    pub fn surface(&self) -> &GpuTextureSlot {
        &self.surface.surface
    }
    fn finish(&mut self) -> Result<(), String> {
        if self.released {
            return Ok(());
        }
        self.released = true;
        unsafe {
            self.surface.surface.device_context().context().Flush();
        }
        if let Err(error) = unsafe { self.surface.mutex.ReleaseSync(0) } {
            self.valid.store(false, Ordering::Release);
            return Err(error.to_string());
        }
        self.surface.leased.store(false, Ordering::Release);
        Ok(())
    }
    /// ACK may be sent only after this returns successfully.
    pub fn release(mut self) -> Result<FrameHeader, String> {
        self.finish()?;
        Ok(self.header)
    }
}
impl Drop for ImportedFrame {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// # Safety
/// Borrow the retained handle of the owned child whose private pipe supplied
/// this offer. Never reopen a PID supplied by an untrusted peer.
pub unsafe fn validate_selected_child(
    process: HANDLE,
    pid: u32,
    session: SessionIdentity,
) -> Result<(), String> {
    if crate::SessionGate::new(session).is_none() {
        return Err("invalid target selection".into());
    }
    let window = HWND(session.target.window as usize as *mut _);
    let mut window_pid = 0;
    if !IsWindow(Some(window)).as_bool() {
        return Err("producer window no longer exists".into());
    }
    GetWindowThreadProcessId(window, Some(&mut window_pid));
    if GetProcessId(process) != pid
        || session.target.pid != pid
        || window_pid != pid
        || creation_time(process)? != session.target.creation_time
    {
        return Err("producer identity does not match the launched process".into());
    }
    Ok(())
}

fn creation_time(process: HANDLE) -> Result<u64, String> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(error)?;
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

fn consumer_device(luid: u64, warp: bool) -> Result<Arc<GpuDeviceContext>, String> {
    let mut selected: Option<IDXGIAdapter> = None;
    if !warp {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(error)?;
        let mut index = 0;
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
            let desc = unsafe { adapter.GetDesc1() }.map_err(error)?;
            if ((u64::from(desc.AdapterLuid.HighPart as u32) << 32)
                | u64::from(desc.AdapterLuid.LowPart))
                == luid
            {
                selected = Some(adapter.cast().map_err(error)?);
                break;
            }
            index += 1;
        }
        if selected.is_none() {
            return Err("producer adapter is not available to the consumer".into());
        }
    }
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            selected.as_ref(),
            if warp {
                D3D_DRIVER_TYPE_WARP
            } else {
                D3D_DRIVER_TYPE_UNKNOWN
            },
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(
                D3D11_CREATE_DEVICE_BGRA_SUPPORT.0
                    | if warp {
                        0
                    } else {
                        D3D11_CREATE_DEVICE_VIDEO_SUPPORT.0
                    },
            ),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(error)?;
    let device = device.ok_or("null consumer device")?;
    let context = context.ok_or("null consumer context")?;
    if adapter_luid(&device)? != luid {
        return Err("producer/consumer adapter LUID mismatch".into());
    }
    let multithread: ID3D11Multithread = context.cast().map_err(error)?;
    unsafe {
        let _ = multithread.SetMultithreadProtected(true);
    }
    Ok(Arc::new(GpuDeviceContext::new(device, context)))
}

impl ConsumerRing {
    pub fn device_context(&self) -> Arc<GpuDeviceContext> {
        self.surfaces[0].surface.device_context().clone()
    }
    /// # Safety
    /// `process` must be the live borrowed process handle from the exact Child
    /// whose private stdout supplied hello, not a reopened PID or remote input.
    pub unsafe fn from_child(
        process: HANDLE,
        pid: u32,
        hello: Hello,
        warp: bool,
    ) -> Result<Self, String> {
        Message::decode(&Message::Hello(hello).encode())
            .map_err(|error| format!("invalid peer greeting: {error:?}"))?;
        validate_selected_child(process, pid, hello.session)?;
        let mut frequency = 0;
        QueryPerformanceFrequency(&mut frequency).map_err(error)?;
        if frequency as u64 != hello.qpc_frequency {
            return Err("producer QPC frequency mismatch".into());
        }
        let device = consumer_device(hello.adapter_luid, warp)?;
        let device1: ID3D11Device1 = device.device().cast().map_err(error)?;
        let mut surfaces = Vec::with_capacity(SLOT_COUNT);
        for remote_handle in hello.handles {
            let mut local_handle = HANDLE::default();
            DuplicateHandle(
                process,
                HANDLE(remote_handle as usize as *mut _),
                GetCurrentProcess(),
                &mut local_handle,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
            .map_err(error)?;
            let handle = OwnedHandle::from_raw_handle(local_handle.0);
            let texture: ID3D11Texture2D = device1
                .OpenSharedResource1(HANDLE(handle.as_raw_handle()))
                .map_err(error)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let shared_flags = (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0
                | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0) as u32;
            if desc.Width != hello.width
                || desc.Height != hello.height
                || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
                || desc.SampleDesc.Count != 1
                || desc.MipLevels != 1
                || desc.ArraySize != 1
                || desc.Usage != D3D11_USAGE_DEFAULT
                || desc.CPUAccessFlags != 0
                || desc.MiscFlags & shared_flags != shared_flags
            {
                return Err(
                    "shared texture descriptor did not match the approved bounded format".into(),
                );
            }
            let mutex = texture.cast().map_err(error)?;
            surfaces.push(Arc::new(ImportSurface {
                surface: GpuTextureSlot::new(texture, device.clone()),
                mutex,
                leased: AtomicBool::new(false),
            }));
            // OpenSharedResource1 retains the resource; the duplicated NT handle
            // is no longer needed and closes here on both success and error.
        }
        let surfaces = surfaces
            .try_into()
            .map_err(|_| "invalid imported slot count")?;
        Ok(Self {
            surfaces,
            gate: SessionGate::new(hello.session).ok_or("invalid peer session")?,
            hello,
            valid: Arc::new(AtomicBool::new(true)),
            _owner: PhantomData,
        })
    }

    pub fn receive(&mut self, header: FrameHeader) -> Result<ImportedFrame, String> {
        if !self.valid.load(Ordering::Acquire) {
            return Err("shared transport is closed".into());
        }
        let validate = (|| {
            self.gate
                .accept(header)
                .map_err(|error| format!("peer frame metadata rejected: {error:?}"))?;
            if (header.width, header.height) != (self.hello.width, self.hello.height) {
                return Err("peer changed dimensions without retiring its ring".into());
            }
            let surface = self.surfaces[header.slot as usize].clone();
            if surface
                .leased
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err("peer reused a retained shared slot".into());
            }
            acquire(&surface.mutex, 1, 1000)?;
            Ok(ImportedFrame {
                header,
                surface,
                valid: self.valid.clone(),
                released: false,
                _owner: PhantomData,
            })
        })();
        if validate.is_err() {
            self.valid.store(false, Ordering::Release);
            self.gate.close();
        }
        validate
    }
}
