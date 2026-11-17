//! WGC display capture session: object ownership, frame pump, resize and
//! loss handling (spec VID-002…008, §13.2, ADR 0005).
//!
//! Ownership model: the dedicated session thread creates the COM/WGC/D3D
//! objects and owns their session lifecycle and cleanup. The free-threaded
//! callback receives cloned pool/device/context references through explicit
//! safety wrappers; the public [`WgcDisplayCapture`] holds only channels,
//! flags, and the thread handle, which makes it `Send` as required by the
//! `VideoCapture` trait.
//!
//! `FrameArrived` fires on the frame pool's internal worker thread. The
//! handler copies each surface into a private GPU texture immediately (the
//! immediate context runs with `ID3D11Multithread` protection) and
//! publishes a lease under a unique handle. Queued/delivered/encoder owners prevent
//! reuse; saturation drops frames rather than overwriting. Active/retired surfaces
//! share a fixed budget across resize and restart (S2 exit criterion).
//!
//! No code is injected into any process: WGC reads composited monitor
//! content through the OS compositor (VID-011).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::{factory, IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureAccess, GraphicsCaptureAccessKind,
    GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

use capture_api::{
    Result as CaptureResult, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent,
};
use encoder_api::{GpuFrameContext, GpuFrameResolveError, GpuFrameResolver, GpuFrameResource};
use gpu_windows::{GpuDeviceContext, GpuTextureSlot};
use media_clock::MediaTimeline;
use media_types::{FramePayload, GpuFrameHandle, PixelFormat, StreamId, TimeBase, VideoFrame};

use crate::device::create_capture_device;
use crate::dxgi_pool::{LeasePool, SurfaceBudget};
use crate::enumerate::resolve_source;

const FRAME_POOL_BUFFERS: i32 = 2;
const STAGING_RING_SIZE: usize = 8;
const MAX_STAGING_SURFACES: usize = 16;
/// Bounded event queue; overflow drops the newest frame (§14.1 backpressure
/// step 4) rather than ever blocking the compositor callback.
// At most sixteen live publications across two eight-slot epochs. Reserve one
// format event per epoch too, so full frame queues cannot lose a resize boundary.
const EVENT_QUEUE_BOUND: usize = MAX_STAGING_SURFACES + MAX_STAGING_SURFACES / STAGING_RING_SIZE;
const NEXT_EVENT_TICK_MS: u64 = 50;
/// `TimeSpan.Duration` is expressed in Windows 100-nanosecond ticks.
const WGC_TIMESTAMP_HZ: u32 = 10_000_000;
/// A long compositor pause is reported as a timestamp discontinuity rather
/// than silently creating a bad clip timeline.
const MAX_TIMESTAMP_JUMP_MS: i64 = 60_000;

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

/// See [`GpuTextureSlot`] safety contract.
struct SendContext(windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext);
unsafe impl Send for SendContext {}

/// See [`GpuTextureSlot`] safety contract. WinRT objects are additionally
/// agile by default, so cross-thread use is doubly sanctioned here.
struct SendWinrtDevice(windows::Graphics::DirectX::Direct3D11::IDirect3DDevice);

impl SendWinrtDevice {
    fn get(&self) -> windows::Graphics::DirectX::Direct3D11::IDirect3DDevice {
        self.0.clone()
    }
}

unsafe impl Send for SendWinrtDevice {}

enum InternalEvent {
    Frame {
        handle: GpuFrameHandle,
        width: u32,
        height: u32,
        source_ticks: i64,
        publication: Arc<GpuTextureSlot>,
    },
    FormatChanged {
        width: u32,
        height: u32,
        pool: Arc<LeasePool<GpuTextureSlot>>,
    },
}

#[derive(Default)]
struct SessionStats {
    frames_copied: u64,
    frames_dropped_queue_full: u64,
    resizes: u64,
    frames_dropped_pool_full: u64,
}

/// State shared with the callbacks; `Send` because every field is a lock,
/// an atomic, or a [`GpuTextureSlot`].
struct SharedState {
    stats: Mutex<SessionStats>,
    pool: Mutex<Option<Arc<LeasePool<GpuTextureSlot>>>>,
    budget: Arc<Mutex<SurfaceBudget<GpuTextureSlot>>>,
    window_target: Option<String>,
    device_context: Arc<GpuDeviceContext>,
    queue_alive: AtomicBool,
    failure: OnceLock<String>,
}

impl SharedState {
    fn new(
        device_context: Arc<GpuDeviceContext>,
        budget: Arc<Mutex<SurfaceBudget<GpuTextureSlot>>>,
        window_target: Option<String>,
    ) -> Self {
        Self {
            stats: Mutex::new(SessionStats::default()),
            pool: Mutex::new(None),
            budget,
            window_target,
            device_context,
            queue_alive: AtomicBool::new(true),
            failure: OnceLock::new(),
        }
    }

    fn publish_frame(&self, tx: &SyncSender<InternalEvent>, event: InternalEvent) {
        if let Err(std::sync::mpsc::TrySendError::Full(_)) = tx.try_send(event) {
            if let Ok(mut stats) = self.stats.lock() {
                stats.frames_dropped_queue_full += 1;
            }
        }
    }

    fn fail(&self, details: String) {
        let _ = self.failure.set(details);
        self.queue_alive.store(false, Ordering::Release);
    }
    fn validate_capture(&self) -> CaptureResult<()> {
        unsafe { self.device_context.device().GetDeviceRemovedReason() }
            .map_err(|_| VideoCaptureError::DeviceLost)?;
        if let Some(target) = &self.window_target {
            let hwnd = crate::enumerate::resolve_window(target)?;
            crate::enumerate::check_window_capture_affinity(hwnd)?;
        }
        Ok(())
    }

    fn release_staged(&self) {
        if let Ok(mut pool) = self.pool.lock() {
            if let Some(pool) = pool.take() {
                pool.clear();
            }
        }
    }
}

/// Resolves one active pool's handles to retained slots. The same Arc is held by
/// events and encoder leases, so the producer cannot overwrite a live publication.
struct WgcGpuFrameResolver {
    shared: Arc<SharedState>,
}

impl GpuFrameResolver for WgcGpuFrameResolver {
    fn resolve(
        &self,
        handle: GpuFrameHandle,
    ) -> std::result::Result<Arc<GpuFrameResource>, GpuFrameResolveError> {
        if !self.shared.queue_alive.load(Ordering::SeqCst) {
            return Err(GpuFrameResolveError::ContextUnavailable {
                details: "capture session is no longer active".into(),
            });
        }

        let guard =
            self.shared
                .pool
                .lock()
                .map_err(|_| GpuFrameResolveError::ContextUnavailable {
                    details: "capture staging registry is unavailable".into(),
                })?;
        let Some(slot) = guard.as_ref().and_then(|pool| pool.resolve(handle.0)) else {
            return Err(GpuFrameResolveError::StaleHandle { handle });
        };
        Ok(slot)
    }

    fn session_resource(&self) -> Option<Arc<GpuFrameResource>> {
        self.shared
            .queue_alive
            .load(Ordering::Acquire)
            .then(|| self.shared.device_context.clone() as Arc<GpuFrameResource>)
    }
}

enum Control {
    Stop,
}

/// Native Windows Graphics Capture display backend implementing the pull
/// model over the callback-driven frame pool. `next_event` waits up to
/// ~[`NEXT_EVENT_TICK_MS`] per iteration so `stop()`/Drop is honored from
/// another thread without long blocking.
pub struct WgcDisplayCapture {
    events: Option<Receiver<InternalEvent>>,
    control: Option<SyncSender<Control>>,
    stop_flag: Arc<AtomicBool>,
    shared: Option<Arc<SharedState>>,
    session_thread: Option<JoinHandle<()>>,
    stream_id: StreamId,
    min_frame_interval_ms: i64,
    last_delivered_pts: Option<i64>,
    timeline: MediaTimeline,
    last_source_pts: Option<i64>,
    cursor_capture: bool,
    delivered: Option<Arc<GpuTextureSlot>>,
    budget: Arc<Mutex<SurfaceBudget<GpuTextureSlot>>>,
}

impl WgcDisplayCapture {
    pub fn new(stream_id: StreamId) -> Self {
        Self {
            events: None,
            control: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
            shared: None,
            session_thread: None,
            stream_id,
            min_frame_interval_ms: 0,
            last_delivered_pts: None,
            timeline: MediaTimeline::new(
                TimeBase::from_hz(WGC_TIMESTAMP_HZ),
                MAX_TIMESTAMP_JUMP_MS,
            ),
            last_source_pts: None,
            cursor_capture: true,
            delivered: None,
            budget: Arc::new(Mutex::new(SurfaceBudget::new(MAX_STAGING_SURFACES))),
        }
    }

    /// Configure whether the WGC compositor includes the pointer. This must
    /// be set before `start`; the shared `VideoCapture` trait will gain the
    /// setting when application configuration is plumbed in.
    pub fn set_cursor_capture(&mut self, enabled: bool) -> CaptureResult<()> {
        if self.running() {
            return Err(VideoCaptureError::Backend {
                details: "cursor policy cannot change while capture is running".into(),
            });
        }
        self.cursor_capture = enabled;
        Ok(())
    }

    /// `(frames_copied, frames_dropped_queue_full, resizes)` snapshot for
    /// soak tests and diagnostics.
    #[allow(dead_code)] // consumed by the capture-probe example and soaks
    pub fn stats_snapshot(&self) -> Option<(u64, u64, u64)> {
        let shared = self.shared.as_ref()?;
        let stats = shared.stats.lock().ok()?;
        Some((
            stats.frames_copied,
            stats.frames_dropped_queue_full + stats.frames_dropped_pool_full,
            stats.resizes,
        ))
    }

    /// Return the encoder-side resolver for the active session. The context
    /// must be installed before the first GPU frame is encoded and should be
    /// discarded after the encoder drains or capture stops.
    pub fn gpu_frame_context(&self) -> Option<GpuFrameContext> {
        if !self.running() {
            return None;
        }
        let shared = Arc::clone(self.shared.as_ref()?);
        if !shared.queue_alive.load(Ordering::Acquire) {
            return None;
        }
        Some(GpuFrameContext::new(Arc::new(WgcGpuFrameResolver {
            shared,
        })))
    }

    fn running(&self) -> bool {
        self.events.is_some()
    }
}

impl Drop for WgcDisplayCapture {
    fn drop(&mut self) {
        let _ = capture_api::VideoCapture::stop(self);
    }
}

impl capture_api::VideoCapture for WgcDisplayCapture {
    fn enumerate_sources(&self) -> CaptureResult<Vec<media_types::VideoSourceInfo>> {
        crate::enumerate_displays()
    }

    fn start(&mut self, config: VideoCaptureConfig) -> CaptureResult<()> {
        if self.running() {
            return Err(VideoCaptureError::Backend {
                details: "capture already started".into(),
            });
        }
        if !matches!(config.target_fps, 30 | 60 | 120) {
            return Err(VideoCaptureError::InvalidConfiguration {
                reason: format!(
                    "target_fps must be 30, 60, or 120, got {}",
                    config.target_fps
                ),
            });
        }
        if !crate::is_wgc_supported() {
            return Err(VideoCaptureError::SourceUnavailable);
        }
        // Validate before spawning so an invalid display id is reported from
        // start() rather than becoming an asynchronous worker warning.
        if config.source_id.starts_with("hwnd:") || config.source_id.starts_with("pid:") {
            let hwnd = crate::enumerate::resolve_window(&config.source_id)?;
            crate::enumerate::check_window_capture_affinity(hwnd)?;
        } else {
            resolve_source(&config.source_id)?;
        }

        // VID-004 shaping: decimate delivery when targeting 30 or 60 fps from a
        // higher-cadence compositor; 120 fps passes everything through.
        self.min_frame_interval_ms = match config.target_fps {
            30 => 31,
            60 => 15,
            120 => 0,
            _ => 0,
        };
        self.last_delivered_pts = None;
        self.timeline =
            MediaTimeline::new(TimeBase::from_hz(WGC_TIMESTAMP_HZ), MAX_TIMESTAMP_JUMP_MS);
        self.last_source_pts = None;

        // Create and protect the device before publishing the resolver. The
        // encoder worker may configure itself as soon as start() returns.
        let capturing_device = create_capture_device()?;
        let device_context = Arc::new(GpuDeviceContext::new(
            capturing_device.device,
            capturing_device.context,
        ));
        let (event_tx, event_rx) = sync_channel::<InternalEvent>(EVENT_QUEUE_BOUND);
        // Stop is a single control command; keep the channel bounded so the
        // capture session has no unbounded queue even during repeated teardown.
        let (control_tx, control_rx) = sync_channel::<Control>(1);
        let stop_flag = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(SharedState::new(
            Arc::clone(&device_context),
            Arc::clone(&self.budget),
            (config.source_id.starts_with("hwnd:") || config.source_id.starts_with("pid:"))
                .then(|| config.source_id.clone()),
        ));

        let thread_shared = Arc::clone(&shared);
        let source_id = config.source_id.clone();
        let cursor_capture = self.cursor_capture;

        let builder = std::thread::Builder::new();
        let handle = builder
            .name("silk-wgc-session".to_string())
            .spawn(move || {
                run_session(
                    &source_id,
                    event_tx,
                    control_rx,
                    &thread_shared,
                    cursor_capture,
                    device_context,
                );
            })
            .map_err(|err| VideoCaptureError::Backend {
                details: format!("failed spawning session thread: {err}"),
            })?;

        self.events = Some(event_rx);
        self.control = Some(control_tx);
        self.stop_flag = stop_flag;
        self.shared = Some(shared);
        self.session_thread = Some(handle);
        diagnostics::info(
            "wgc",
            &format!("capture starting (source '{}')", config.source_id),
        );
        Ok(())
    }

    fn next_event(&mut self) -> CaptureResult<VideoCaptureEvent> {
        self.delivered.take();
        let Some(events) = &mut self.events else {
            return Err(VideoCaptureError::Backend {
                details: "capture not started".into(),
            });
        };

        loop {
            if self.stop_flag.load(Ordering::SeqCst) {
                return Err(VideoCaptureError::EndOfStream);
            }
            if let Some(shared) = &self.shared {
                if let Err(error) = shared.validate_capture() {
                    shared.queue_alive.store(false, Ordering::Release);
                    while events.try_recv().is_ok() {}
                    shared.release_staged();
                    return Err(error);
                }
                if !shared.queue_alive.load(Ordering::Acquire) {
                    while events.try_recv().is_ok() {}
                    shared.release_staged();
                    return match shared.failure.get() {
                        Some(details) => Err(VideoCaptureError::Backend {
                            details: details.clone(),
                        }),
                        None => Ok(VideoCaptureEvent::SourceLost),
                    };
                }
            }
            let event = events.recv_timeout(Duration::from_millis(NEXT_EVENT_TICK_MS));
            if let Some(shared) = &self.shared {
                if let Err(error) = shared.validate_capture() {
                    shared.queue_alive.store(false, Ordering::Release);
                    shared.release_staged();
                    while events.try_recv().is_ok() {}
                    return Err(error);
                }
            }
            if self
                .shared
                .as_ref()
                .is_some_and(|shared| !shared.queue_alive.load(Ordering::Acquire))
            {
                continue;
            }
            match event {
                Ok(InternalEvent::Frame {
                    handle,
                    width,
                    height,
                    source_ticks,
                    publication,
                }) => {
                    // WGC retains its compositor clock through idle/minimize gaps.
                    // Keep that origin: resetting to zero stalls cadence shaping
                    // against the last delivered timestamp and loses audio alignment.
                    let pts_ms = self
                        .timeline
                        .to_shared_allow_forward_gap(source_ticks)
                        .map_err(|discontinuity| VideoCaptureError::Backend {
                            details: discontinuity.to_string(),
                        })?;
                    let pts_ms = if let Some(last) = self.last_source_pts {
                        pts_ms.max(last)
                    } else {
                        pts_ms
                    };
                    self.last_source_pts = Some(pts_ms);
                    // Shaping compares frame PTS (compositor cadence).
                    if self.min_frame_interval_ms > 0 {
                        if let Some(last) = self.last_delivered_pts {
                            if pts_ms - last < self.min_frame_interval_ms {
                                continue;
                            }
                        }
                        self.last_delivered_pts = Some(pts_ms);
                    }
                    self.delivered = Some(publication);
                    return Ok(VideoCaptureEvent::Frame(VideoFrame {
                        stream_id: self.stream_id,
                        width,
                        height,
                        pixel_format: PixelFormat::Bgra8,
                        pts: pts_ms,
                        time_base: TimeBase::MILLISECOND,
                        payload: FramePayload::Gpu(handle),
                    }));
                }
                Ok(InternalEvent::FormatChanged {
                    width,
                    height,
                    pool,
                }) => {
                    if let Some(shared) = &self.shared {
                        let mut current =
                            shared.pool.lock().map_err(|_| VideoCaptureError::Backend {
                                details: "WGC resolver state poisoned".into(),
                            })?;
                        if let Some(old) = current.replace(pool) {
                            old.clear();
                        }
                    }
                    return Ok(VideoCaptureEvent::FormatChanged { width, height });
                }
                Err(RecvTimeoutError::Timeout) => return Ok(VideoCaptureEvent::Idle),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(VideoCaptureError::EndOfStream);
                }
            }
        }
    }

    fn stop(&mut self) -> CaptureResult<()> {
        if !self.running() {
            return Ok(()); // idempotent
        }
        self.stop_flag.store(true, Ordering::SeqCst);
        self.delivered.take();
        if let Some(shared) = &self.shared {
            shared.queue_alive.store(false, Ordering::SeqCst);
        }
        if let Some(control) = &self.control {
            let _ = control.send(Control::Stop);
        }
        if let Some(handle) = self.session_thread.take() {
            let _ = handle.join();
        }
        // `shared` intentionally survives stop so soak/probe callers can
        // read final counters; it is replaced on the next start().
        self.events = None;
        self.control = None;
        if let Some(shared) = &self.shared {
            shared.release_staged();
        }
        diagnostics::info("wgc", "capture stopped");
        Ok(())
    }

    fn gpu_frame_context(&self) -> Option<GpuFrameContext> {
        WgcDisplayCapture::gpu_frame_context(self)
    }
}

// ---------------------------------------------------------------------------
// Session thread
// ---------------------------------------------------------------------------

fn run_session(
    source_id: &str,
    event_tx: SyncSender<InternalEvent>,
    control_rx: Receiver<Control>,
    shared: &Arc<SharedState>,
    cursor_capture: bool,
    device_context: Arc<GpuDeviceContext>,
) {
    if let Err(err) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        let details = format!("RoInitialize failed: {err}");
        diagnostics::error("wgc", &details);
        shared.fail(details);
        return;
    }

    match run_session_inner(
        source_id,
        &event_tx,
        shared,
        cursor_capture,
        &device_context,
    ) {
        Ok(setup) => {
            // Park until Stop arrives, keeping all objects alive here so
            // callbacks keep firing. Drop order releases platform resources
            // deterministically on this thread.
            if let Ok(Control::Stop) = control_rx.recv() {
                // fallthrough to cleanup via drop
            }
            drop(setup);
            diagnostics::info("wgc", "session thread closing");
        }
        Err(err) => {
            diagnostics::error("wgc", &format!("session failed: {err}"));
            shared.fail(err.to_string());
        }
    }
    unsafe {
        RoUninitialize();
    }
}

struct SessionSetup {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    item: GraphicsCaptureItem,
    frame_token: Option<i64>,
    closed_token: Option<i64>,
}
impl Drop for SessionSetup {
    fn drop(&mut self) {
        if let Some(token) = self.frame_token.take() {
            let _ = self.pool.RemoveFrameArrived(token);
        }
        if let Some(token) = self.closed_token.take() {
            let _ = self.item.RemoveClosed(token);
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// Map `windows::core::Result` errors into our backend error at `?` sites.
trait IntoCapture<T> {
    fn cap(self) -> CaptureResult<T>;
    fn cap_at(self, stage: &'static str) -> CaptureResult<T>;
}

impl<T> IntoCapture<T> for windows::core::Result<T> {
    fn cap(self) -> CaptureResult<T> {
        self.map_err(|err| VideoCaptureError::Backend {
            details: err.to_string(),
        })
    }
    fn cap_at(self, stage: &'static str) -> CaptureResult<T> {
        self.map_err(|err| VideoCaptureError::Backend {
            details: format!("{stage} failed: {err}"),
        })
    }
}

fn allocate_staging_pool(
    shared: &SharedState,
    width: u32,
    height: u32,
) -> CaptureResult<Option<Arc<LeasePool<GpuTextureSlot>>>> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width.max(1),
        Height: height.max(1),
        MipLevels: 1,
        ArraySize: 1,
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut budget = shared
        .budget
        .lock()
        .map_err(|_| VideoCaptureError::Backend {
            details: "WGC surface budget poisoned".into(),
        })?;
    let pool = budget.allocate(STAGING_RING_SIZE, || {
        let mut texture: Option<ID3D11Texture2D> = None;
        unsafe {
            shared
                .device_context
                .device()
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .map_err(|err| VideoCaptureError::Backend {
                    details: format!("CreateTexture2D for staging pool failed: {err}"),
                })?;
        }
        let texture = texture.ok_or_else(|| VideoCaptureError::Backend {
            details: "CreateTexture2D returned None".to_string(),
        })?;
        Ok(GpuTextureSlot::new(texture, shared.device_context.clone()))
    })?;
    Ok(pool.map(|pool| Arc::new(LeasePool::from_shared(pool))))
}

fn run_session_inner(
    source_id: &str,
    event_tx: &SyncSender<InternalEvent>,
    shared: &Arc<SharedState>,
    cursor_capture: bool,
    device_context: &Arc<GpuDeviceContext>,
) -> CaptureResult<SessionSetup> {
    let device = device_context.device().clone();
    let context = device_context.context().clone();

    // Wrap the D3D11 device for WinRT (IDirect3DDevice).
    let dxgi_device = device
        .cast::<windows::Win32::Graphics::Dxgi::IDXGIDevice>()
        .cap()?;
    let inspectable =
        unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }.map_err(|err| {
            VideoCaptureError::Backend {
                details: format!("CreateDirect3D11DeviceFromDXGIDevice failed: {err}"),
            }
        })?;
    let d3d_device: windows::Graphics::DirectX::Direct3D11::IDirect3DDevice =
        inspectable.cast().cap()?;

    let interop: IGraphicsCaptureItemInterop = factory::<GraphicsCaptureItem, _>().cap()?;
    let item: GraphicsCaptureItem = if source_id.starts_with("hwnd:")
        || source_id.starts_with("pid:")
    {
        let hwnd = crate::enumerate::resolve_window(source_id)?;
        crate::enumerate::check_window_capture_affinity(hwnd)?;
        unsafe { interop.CreateForWindow(hwnd) }.cap_at("GraphicsCaptureItem.CreateForWindow")?
    } else {
        let (_output, hmonitor_isize) = resolve_source(source_id)?;
        let hmonitor = HMONITOR(hmonitor_isize as *mut core::ffi::c_void);
        unsafe { interop.CreateForMonitor(hmonitor) }
            .cap_at("GraphicsCaptureItem.CreateForMonitor")?
    };

    let initial_size = item.Size().cap_at("GraphicsCaptureItem.Size")?;
    // Item.Size can include bounds that differ from the first ContentSize.
    // Allocate only for observed content, avoiding a disposable eight-surface
    // startup pool that would consume a retained-session budget during restart.
    let staging_pool = Arc::new(Mutex::new(None::<Arc<LeasePool<GpuTextureSlot>>>));

    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &d3d_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        FRAME_POOL_BUFFERS,
        initial_size,
    )
    .cap_at("Direct3D11CaptureFramePool.CreateFreeThreaded")?;
    let session = pool
        .CreateCaptureSession(&item)
        .cap_at("CreateCaptureSession")?;
    // From here, any setup failure unregisters handlers and breaks pool/handler
    // reference cycles rather than leaking a failed session's GPU allocation.
    let mut setup = SessionSetup {
        pool: pool.clone(),
        session: session.clone(),
        item: item.clone(),
        frame_token: None,
        closed_token: None,
    };

    // Frame pump: drain whatever is available each wake-up, copying each
    // surface into a pre-allocated GPU texture from the staging ring under a unique handle.
    let handler_shared = Arc::clone(shared);
    let handler_size = Arc::new(Mutex::new(None::<windows::Graphics::SizeInt32>));
    // Pre-allocated texture pool is reused across frames (0 per-frame allocations).
    let copy_context = SendContext(context.clone());
    // Owned handle: Recreate takes the WinRT device by reference.
    let handler_device = SendWinrtDevice(d3d_device.clone());
    let handler_pool = pool.clone();
    let handler_tx = event_tx.clone();
    let handler_staging_pool = Arc::clone(&staging_pool);
    let frame_handler =
        TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_sender, _| {
            // Use our owned handle rather than the callback's guard ref.
            let pool = handler_pool.clone();
            let context = &copy_context.0;
            let winrt_device = handler_device.get();
            if !handler_shared.queue_alive.load(Ordering::SeqCst) {
                return Ok(());
            }

            loop {
                let Ok(frame) = pool.TryGetNextFrame() else {
                    break;
                };
                let Ok(content) = frame.ContentSize() else {
                    continue;
                };
                if let Some(target) = &handler_shared.window_target {
                    match crate::enumerate::resolve_window(target) {
                        Ok(hwnd) => {
                            if unsafe { windows::Win32::UI::WindowsAndMessaging::IsIconic(hwnd) }
                                .as_bool()
                            {
                                // Some windows publish a tiny taskbar-sized
                                // surface while minimized. It is not a new game
                                // format and may be unsupported by hardware MFTs.
                                let _ = frame.Close();
                                continue;
                            }
                            let mut client = windows::Win32::Foundation::RECT::default();
                            if unsafe {
                                windows::Win32::UI::WindowsAndMessaging::GetClientRect(
                                    hwnd,
                                    &mut client,
                                )
                            }
                            .is_ok()
                                && obsolete_window_content(content.Width, content.Height, &client)
                            {
                                // Minimized publications can arrive after restore.
                                // Do not configure an obsolete taskbar thumbnail
                                // when the live client is already larger.
                                let _ = frame.Close();
                                continue;
                            }
                        }
                        Err(_) => {
                            let _ = frame.Close();
                            handler_shared.queue_alive.store(false, Ordering::Release);
                            return Ok(());
                        }
                    }
                }
                if content.Width <= 0 || content.Height <= 0 {
                    continue;
                }

                // Resize path (VID-006): recreate the staging pool and WGC pool at the new
                // content size and notify the pipeline. Break afterwards;
                // the next FrameArrived continues on the new pool.
                {
                    let mut guard = match handler_size.lock() {
                        Ok(guard) => guard,
                        Err(_) => return Ok(()),
                    };
                    if guard.as_ref() != Some(&content) {
                        let is_resize = guard.is_some();
                        let new_width = content.Width.max(1) as u32;
                        let new_height = content.Height.max(1) as u32;
                        let new_pool =
                            match allocate_staging_pool(&handler_shared, new_width, new_height) {
                                Ok(Some(pool)) => pool,
                                Ok(None) => {
                                    // Rapid restore/resize can overtake consumer
                                    // retirement. Keep the bounded budget and
                                    // retry on a later frame, without publishing
                                    // pixels under the wrong format or failing
                                    // an otherwise healthy capture session.
                                    let _ = frame.Close();
                                    break;
                                }
                                Err(error) => {
                                    handler_shared.fail(error.to_string());
                                    return Ok(());
                                }
                            };
                        let _ = frame.Close();
                        let resized = pool.Recreate(
                            &winrt_device,
                            DirectXPixelFormat::B8G8R8A8UIntNormalized,
                            FRAME_POOL_BUFFERS,
                            content,
                        );
                        if let Ok(mut stats) = handler_shared.stats.lock() {
                            stats.resizes += u64::from(is_resize);
                        }
                        if let Err(error) = resized {
                            handler_shared.fail(format!("WGC frame pool resize failed: {error}"));
                            return Ok(());
                        }
                        *guard = Some(content);
                        if handler_tx
                            .try_send(InternalEvent::FormatChanged {
                                width: content.Width.max(0) as u32,
                                height: content.Height.max(0) as u32,
                                pool: new_pool.clone(),
                            })
                            .is_err()
                        {
                            handler_shared
                                .fail("WGC format transition queue full/disconnected".into());
                            return Ok(());
                        }
                        if let Ok(mut pool_guard) = handler_staging_pool.lock() {
                            *pool_guard = Some(new_pool);
                        } else {
                            handler_shared.fail("WGC staging pool poisoned".into());
                            return Ok(());
                        }
                        drop(guard);
                        break;
                    }
                }

                let texture: ID3D11Texture2D = match frame.Surface() {
                    Ok(surface) => match surface.cast::<IDirect3DDxgiInterfaceAccess>() {
                        Ok(access) => match unsafe { access.GetInterface() } {
                            Ok(texture) => texture,
                            Err(_) => continue,
                        },
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };

                // Copy into next pre-allocated texture from the staging ring (zero per-frame allocations).
                let staging = {
                    let pool_guard = match handler_staging_pool.lock() {
                        Ok(guard) => guard,
                        Err(_) => continue,
                    };
                    let Some(pool) = pool_guard.as_ref() else {
                        continue;
                    };
                    pool.clone()
                };
                let Some((index, publication)) = staging.acquire() else {
                    if let Ok(mut stats) = handler_shared.stats.lock() {
                        stats.frames_dropped_pool_full += 1;
                    }
                    continue;
                };

                unsafe {
                    context.CopyResource(publication.texture(), &texture);
                }

                let handle =
                    match NEXT_HANDLE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                        value.checked_add(1)
                    }) {
                        Ok(value) => GpuFrameHandle(value),
                        Err(_) => {
                            handler_shared.fail("WGC frame handle exhausted".into());
                            return Ok(());
                        }
                    };
                staging.publish(index, handle.0);
                if let Ok(mut stats) = handler_shared.stats.lock() {
                    stats.frames_copied += 1;
                }

                let Ok(source_ticks) = frame.SystemRelativeTime() else {
                    continue;
                };

                handler_shared.publish_frame(
                    &handler_tx,
                    InternalEvent::Frame {
                        handle,
                        width: content.Width.max(0) as u32,
                        height: content.Height.max(0) as u32,
                        source_ticks: source_ticks.Duration,
                        publication,
                    },
                );

                // `frame` drops here, releasing the pool surface reference.
            }
            Ok(())
        });
    setup.frame_token = Some(pool.FrameArrived(&frame_handler).cap()?);

    // Monitor disappearance surfaces once as SourceLost (VID-007); the
    // engine decides recovery vs clean stop.
    let closed_shared = shared.clone();
    let closed_handler =
        TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
            closed_shared.queue_alive.store(false, Ordering::Release);
            Ok(())
        });
    setup.closed_token = Some(item.Closed(&closed_handler).cap()?);
    session.SetIsCursorCaptureEnabled(cursor_capture).cap()?;
    // Windows requires consent before it will honor IsBorderRequired=false.
    // Negotiate before StartCapture, never on the real-time frame callback.
    match GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)
        .and_then(|operation| operation.join())
    {
        Ok(windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus::Allowed) => {
            if let Err(error) = session.SetIsBorderRequired(false) {
                diagnostics::warn("wgc", &format!("borderless capture unavailable: {error}"));
            }
        }
        Ok(status) => diagnostics::warn(
            "wgc",
            &format!("borderless capture permission not granted ({status:?}); Windows may show its capture border"),
        ),
        Err(error) => diagnostics::warn(
            "wgc",
            &format!("borderless capture permission unavailable: {error}; retaining Windows capture indicator"),
        ),
    }
    // Window bounds used during enumeration can include non-content borders.
    shared.validate_capture()?;
    // First observed ContentSize publishes the format before any staged frame.
    session
        .StartCapture()
        .cap_at("GraphicsCaptureSession.StartCapture")?;
    diagnostics::info(
        "wgc",
        &format!(
            "session started ({}x{})",
            initial_size.Width, initial_size.Height
        ),
    );

    Ok(setup)
}

// WGC includes non-client bounds, so it cannot be smaller than the current
// client. Empty clients occur during minimize transitions before IsIconic has
// necessarily caught up. Do not turn either transient into an encoder epoch.
fn obsolete_window_content(
    width: i32,
    height: i32,
    client: &windows::Win32::Foundation::RECT,
) -> bool {
    client.right <= client.left
        || client.bottom <= client.top
        || i64::from(width) < i64::from(client.right) - i64::from(client.left)
        || i64::from(height) < i64::from(client.bottom) - i64::from(client.top)
}

#[cfg(test)]
mod tests {
    #[test]
    fn minimize_and_stale_restore_content_never_become_window_formats() {
        use windows::Win32::Foundation::RECT;
        let restored = RECT {
            left: 0,
            top: 0,
            right: 784,
            bottom: 561,
        };
        assert!(super::obsolete_window_content(146, 21, &RECT::default()));
        assert!(super::obsolete_window_content(146, 21, &restored));
        assert!(!super::obsolete_window_content(786, 593, &restored));
        assert!(!super::obsolete_window_content(784, 561, &restored));
    }
    use super::*;
    #[test]
    fn setup_error_keeps_stage_and_hresult() {
        let result: windows::core::Result<()> = Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_INVALIDARG,
        ));
        let error = result
            .cap_at("GraphicsCaptureItem.CreateForWindow")
            .unwrap_err()
            .to_string();
        assert!(error.contains("GraphicsCaptureItem.CreateForWindow"));
        assert!(error.contains("0x80070057"));
    }
}
