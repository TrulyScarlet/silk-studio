//! Monitor capture through Desktop Duplication. All DXGI objects belong to one
//! worker; the encoder receives immutable leases on eight reusable GPU textures.
//! The event queue is bounded, and congestion drops frames rather than blocking.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use capture_api::{Result, VideoCapture, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent};
use encoder_api::{GpuFrameContext, GpuFrameResolveError, GpuFrameResolver, GpuFrameResource};
use gpu_windows::{GpuDeviceContext, GpuTextureSlot};
use media_types::{
    FramePayload, GpuFrameHandle, PixelFormat, StreamId, TimeBase, VideoFrame, VideoSourceInfo,
};
use windows::core::Interface;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::{Common::*, *};
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject,
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS,
};

use crate::dxgi_cursor::Cursor;
use crate::dxgi_pool::LeasePool;

const POOL_SIZE: usize = 8;
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1 << 63);

pub(super) fn backend(error: impl std::fmt::Display) -> VideoCaptureError {
    VideoCaptureError::Backend {
        details: format!("DXGI: {error}"),
    }
}

struct Shared {
    gpu: Arc<GpuDeviceContext>,
    pool: LeasePool<GpuTextureSlot>,
    active: AtomicBool,
    terminal: Mutex<Option<Result<VideoCaptureEvent>>>,
}

impl GpuFrameResolver for Shared {
    fn resolve(
        &self,
        handle: GpuFrameHandle,
    ) -> std::result::Result<Arc<GpuFrameResource>, GpuFrameResolveError> {
        if !self.active.load(Ordering::Acquire) {
            return Err(GpuFrameResolveError::ContextUnavailable {
                details: "DXGI capture stopped".into(),
            });
        }
        self.pool
            .resolve(handle.0)
            .map(|slot| slot as Arc<GpuFrameResource>)
            .ok_or(GpuFrameResolveError::StaleHandle { handle })
    }
    fn session_resource(&self) -> Option<Arc<GpuFrameResource>> {
        Some(self.gpu.clone())
    }
}

/// `VideoCapture` handle; native resources stay on the dedicated worker thread.
pub struct DxgiDisplayCapture {
    stream: StreamId,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    events: Option<Receiver<(VideoCaptureEvent, Arc<GpuTextureSlot>)>>,
    delivered: Option<Arc<GpuTextureSlot>>,
    shared: Option<Arc<Shared>>,
}

impl DxgiDisplayCapture {
    pub fn new(stream: StreamId) -> Self {
        Self {
            stream,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            events: None,
            shared: None,
            delivered: None,
        }
    }
}

impl VideoCapture for DxgiDisplayCapture {
    fn enumerate_sources(&self) -> Result<Vec<VideoSourceInfo>> {
        crate::enumerate_displays()
    }

    fn start(&mut self, config: VideoCaptureConfig) -> Result<()> {
        if self.worker.is_some() {
            return Err(backend("capture already started"));
        }
        if !matches!(config.target_fps, 30 | 60 | 120) {
            return Err(VideoCaptureError::InvalidConfiguration {
                reason: "target_fps must be 30, 60, or 120".into(),
            });
        }
        let (ready_tx, ready_rx) = sync_channel(1);
        let (event_tx, event_rx) = sync_channel(2);
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&self.stop);
        let stream = self.stream;
        self.worker = Some(
            std::thread::Builder::new()
                .name("silk-dxgi-capture".into())
                .spawn(move || {
                    let mut session = match Session::open(&config.source_id) {
                        Ok(session) => session,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                            return;
                        }
                    };
                    let shared = Arc::clone(&session.shared);
                    if ready_tx.send(Ok(Arc::clone(&shared))).is_err() {
                        return;
                    }
                    if let Err(error) = session.run(stream, config.target_fps, &stop, &event_tx) {
                        let event = if matches!(
                            error,
                            VideoCaptureError::DeviceLost | VideoCaptureError::SourceUnavailable
                        ) {
                            Ok(VideoCaptureEvent::SourceLost)
                        } else {
                            Err(error)
                        };
                        if let Ok(mut terminal) = shared.terminal.lock() {
                            *terminal = Some(event);
                        }
                    }
                })
                .map_err(backend)?,
        );
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(shared)) => {
                self.shared = Some(shared);
                self.events = Some(event_rx);
                diagnostics::info(
                    "dxgi",
                    "Desktop Duplication active (8 GPU slots, cursor enabled)",
                );
                Ok(())
            }
            result => {
                let error = match result {
                    Ok(Err(error)) => error,
                    _ => backend("capture startup timed out or worker exited"),
                };
                self.stop()?;
                Err(error)
            }
        }
    }

    fn next_event(&mut self) -> Result<VideoCaptureEvent> {
        let events = self
            .events
            .as_ref()
            .ok_or_else(|| backend("capture not started"))?;
        // The worker emits repeated snapshots on static desktops, so the engine
        // can service stop and save commands even when the image never changes.
        match events.recv_timeout(Duration::from_secs(6)) {
            Ok((event, lease)) => {
                self.delivered = Some(lease);
                Ok(event)
            }
            Err(_) => self
                .shared
                .as_ref()
                .and_then(|s| s.terminal.lock().ok()?.take())
                .unwrap_or(Err(VideoCaptureError::EndOfStream)),
        }
    }

    fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
        if let Some(shared) = self.shared.take() {
            shared.active.store(false, Ordering::Release);
            shared.pool.clear();
        }
        self.events = None;
        self.delivered = None;
        Ok(())
    }

    fn gpu_frame_context(&self) -> Option<GpuFrameContext> {
        self.shared
            .as_ref()
            .map(|shared| GpuFrameContext::new(shared.clone()))
    }
}

impl Drop for DxgiDisplayCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct Session {
    duplication: IDXGIOutputDuplication,
    shared: Arc<Shared>,
    desktop: ID3D11Texture2D,
    cursor: Cursor,
    width: u32,
    height: u32,
}

// Every successful AcquireNextFrame has a ReleaseFrame, including error paths.
struct Acquired<'a>(&'a IDXGIOutputDuplication);
impl Drop for Acquired<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.0.ReleaseFrame();
        }
    }
}

// A per-worker high-resolution timer avoids the ~15.6 ms default scheduler
// interval limiting 120 fps capture. No global timer-resolution change or spin.
struct FrameTimer(HANDLE);
impl FrameTimer {
    fn new() -> Result<Self> {
        unsafe {
            CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )
        }
        .map(Self)
        .map_err(backend)
    }
    fn wait(&self, duration: Duration) -> Result<()> {
        let ticks = -((duration.as_nanos() / 100).max(1) as i64);
        unsafe {
            SetWaitableTimer(self.0, &ticks, 0, None, None, false).map_err(backend)?;
            if WaitForSingleObject(self.0, 100) != WAIT_OBJECT_0 {
                return Err(backend("frame timer wait failed"));
            }
        }
        Ok(())
    }
}
impl Drop for FrameTimer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl Session {
    fn open(source: &str) -> Result<Self> {
        let (output, _) = crate::enumerate::resolve_source(source)?;
        let desc = unsafe { output.GetDesc() }.map_err(backend)?;
        if !matches!(
            desc.Rotation,
            DXGI_MODE_ROTATION_IDENTITY | DXGI_MODE_ROTATION_UNSPECIFIED
        ) {
            return Err(backend("rotated display requires Windows Graphics Capture"));
        }
        let adapter: IDXGIAdapter = unsafe { output.GetParent() }.map_err(backend)?;
        let device = crate::device::create_output_device(&adapter)?;
        let gpu = Arc::new(GpuDeviceContext::new(device.device, device.context));
        let output: IDXGIOutput1 = output.cast().map_err(backend)?;
        let duplication = unsafe { output.DuplicateOutput(gpu.device()) }.map_err(backend)?;
        let mode = unsafe { duplication.GetDesc() };
        if mode.ModeDesc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return Err(backend("unsupported desktop pixel format"));
        }
        let (width, height) = (mode.ModeDesc.Width, mode.ModeDesc.Height);
        let desktop = texture(&gpu, width, height, false)?;
        let mut slots = Vec::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            slots.push(GpuTextureSlot::new(
                texture(&gpu, width, height, false)?,
                Arc::clone(&gpu),
            ));
        }
        let shared = Arc::new(Shared {
            gpu,
            pool: LeasePool::new(slots),
            active: AtomicBool::new(true),
            terminal: Mutex::new(None),
        });
        Ok(Self {
            duplication,
            shared,
            desktop,
            cursor: Cursor::default(),
            width,
            height,
        })
    }

    fn run(
        &mut self,
        stream: StreamId,
        fps: u32,
        stop: &AtomicBool,
        events: &SyncSender<(VideoCaptureEvent, Arc<GpuTextureSlot>)>,
    ) -> Result<()> {
        let timer = FrameTimer::new()?;
        let started = Instant::now();
        let mut origin = None;
        let mut has_desktop = false;
        let mut delivered = 0_u64;
        let mut dropped = 0_u64;
        while !stop.load(Ordering::Acquire) {
            let sampled = Instant::now();
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            match unsafe {
                self.duplication
                    .AcquireNextFrame(0, &mut info, &mut resource)
            } {
                Ok(()) => {
                    let _frame = Acquired(&self.duplication);
                    let source: ID3D11Texture2D = resource
                        .ok_or_else(|| backend("empty desktop resource"))?
                        .cast()
                        .map_err(backend)?;
                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                    unsafe {
                        source.GetDesc(&mut desc);
                    }
                    if (desc.Width, desc.Height) != (self.width, self.height) {
                        return Err(VideoCaptureError::DeviceLost);
                    }
                    // A pointer-only notification can arrive before the first
                    // desktop presentation. Keep the last clean desktop until
                    // DXGI reports an actual image update.
                    if info.LastPresentTime != 0 {
                        unsafe {
                            self.shared
                                .gpu
                                .context()
                                .CopyResource(&self.desktop, &source);
                        }
                        has_desktop = true;
                    }
                    self.cursor.update(&self.duplication, &info)?;
                }
                Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => {}
                Err(error)
                    if matches!(
                        error.code(),
                        DXGI_ERROR_ACCESS_LOST
                            | DXGI_ERROR_DEVICE_REMOVED
                            | DXGI_ERROR_DEVICE_RESET
                            | DXGI_ERROR_SESSION_DISCONNECTED
                    ) =>
                {
                    return Err(VideoCaptureError::DeviceLost)
                }
                Err(error) => return Err(backend(error)),
            }
            if has_desktop {
                let origin = *origin.get_or_insert(sampled);
                if let Some((index, slot)) = self.shared.pool.acquire() {
                    unsafe {
                        self.shared
                            .gpu
                            .context()
                            .CopyResource(slot.texture(), &self.desktop);
                    }
                    self.cursor
                        .draw(&self.shared.gpu, slot.texture(), self.width, self.height)?;
                    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
                    self.shared.pool.publish(index, handle);
                    let frame = VideoFrame {
                        stream_id: stream,
                        width: self.width,
                        height: self.height,
                        pixel_format: PixelFormat::Bgra8,
                        pts: sampled.duration_since(origin).as_millis() as i64,
                        time_base: TimeBase::MILLISECOND,
                        payload: FramePayload::Gpu(GpuFrameHandle(handle)),
                    };
                    if events
                        .try_send((VideoCaptureEvent::Frame(frame), slot))
                        .is_ok()
                    {
                        delivered += 1;
                    } else {
                        dropped += 1;
                    }
                } else {
                    dropped += 1;
                }
            } else if started.elapsed() > Duration::from_secs(5) {
                return Err(backend("no desktop frame arrived within five seconds"));
            }
            let elapsed = started.elapsed();
            let next = Duration::from_nanos(next_tick_ns(elapsed.as_nanos() as u64, fps));
            timer.wait(next.saturating_sub(elapsed))?;
        }
        diagnostics::info(
            "dxgi",
            &format!("capture stopped: delivered={delivered}, dropped={dropped}"),
        );
        Ok(())
    }
}

// Deadlines derive from the original epoch; fractional frame intervals do not
// accumulate millisecond rounding error, and a stall never causes a catch-up burst.
fn next_tick_ns(elapsed: u64, fps: u32) -> u64 {
    let frame = u128::from(elapsed) * u128::from(fps) / 1_000_000_000 + 1;
    (frame * 1_000_000_000).div_ceil(u128::from(fps)) as u64
}

pub(super) fn texture(
    gpu: &GpuDeviceContext,
    width: u32,
    height: u32,
    cpu: bool,
) -> Result<ID3D11Texture2D> {
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
        Usage: if cpu {
            D3D11_USAGE_STAGING
        } else {
            D3D11_USAGE_DEFAULT
        },
        BindFlags: if cpu {
            0
        } else {
            D3D11_BIND_RENDER_TARGET.0 as u32
        },
        CPUAccessFlags: if cpu {
            (D3D11_CPU_ACCESS_READ.0 | D3D11_CPU_ACCESS_WRITE.0) as u32
        } else {
            0
        },
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe {
        gpu.device()
            .CreateTexture2D(&desc, None, Some(&mut texture))
    }
    .map_err(backend)?;
    texture.ok_or_else(|| backend("texture creation returned no resource"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_cadence_has_no_drift_or_catchup_burst() {
        for fps in [30, 60, 120] {
            let mut stamp = 0;
            for _ in 0..fps * 600 {
                stamp = next_tick_ns(stamp, fps);
            }
            assert_eq!(stamp, 600_000_000_000);
            let stalled = 45_123_456_789;
            let resumed = next_tick_ns(stalled, fps);
            assert!(resumed > stalled && resumed - stalled <= 1_000_000_000 / u64::from(fps) + 1);
        }
    }
}
