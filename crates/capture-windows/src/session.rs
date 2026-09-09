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
//! publishes it under a unique handle in a bounded registry; oldest
//! entries are evicted once the ring is full, so GPU memory stays constant
//! regardless of consumer speed (S2 exit criterion).
//!
//! No code is injected into any process: WGC reads composited monitor
//! content through the OS compositor (VID-011).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::{factory, IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
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
use crate::enumerate::resolve_source;

const FRAME_POOL_BUFFERS: i32 = 2;
const STAGING_RING_SIZE: usize = 8;
/// Bounded event queue; overflow drops the newest frame (§14.1 backpressure
/// step 4) rather than ever blocking the compositor callback.
const EVENT_QUEUE_BOUND: usize = 64;
const NEXT_EVENT_TICK_MS: u64 = 50;
/// `TimeSpan.Duration` is expressed in Windows 100-nanosecond ticks.
const WGC_TIMESTAMP_HZ: u32 = 10_000_000;
/// A long compositor pause is reported as a timestamp discontinuity rather
/// than silently creating a bad clip timeline.
const MAX_TIMESTAMP_JUMP_MS: i64 = 60_000;

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

/// See [`GpuTextureSlot`] safety contract.
struct SendDevice(windows::Win32::Graphics::Direct3D11::ID3D11Device);
unsafe impl Send for SendDevice {}

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

#[derive(Debug)]
enum InternalEvent {
    Frame {
        handle: GpuFrameHandle,
        width: u32,
        height: u32,
        source_ticks: i64,
    },
    FormatChanged {
        width: u32,
        height: u32,
    },
    SourceClosed,
    Failed {
        details: String,
    },
}

#[derive(Default)]
struct SessionStats {
    frames_copied: u64,
    frames_dropped_queue_full: u64,
    resizes: u64,
}

/// State shared with the callbacks; `Send` because every field is a lock,
/// an atomic, or a [`GpuTextureSlot`].
struct SharedState {
    stats: Mutex<SessionStats>,
    registry: Mutex<(VecDeque<u64>, HashMap<u64, GpuTextureSlot>)>,
    device_context: Arc<GpuDeviceContext>,
    queue_alive: AtomicBool,
}

impl SharedState {
    fn new(device_context: Arc<GpuDeviceContext>) -> Self {
        Self {
            stats: Mutex::new(SessionStats::default()),
            registry: Mutex::new((VecDeque::new(), HashMap::new())),
            device_context,
            queue_alive: AtomicBool::new(true),
        }
    }

    fn publish_frame(&self, tx: &SyncSender<InternalEvent>, event: InternalEvent) {
        if let Err(std::sync::mpsc::TrySendError::Full(_)) = tx.try_send(event) {
            if let Ok(mut stats) = self.stats.lock() {
                stats.frames_dropped_queue_full += 1;
            }
        }
    }

    fn insert_staged(&self, texture: ID3D11Texture2D) -> GpuFrameHandle {
        let handle = GpuFrameHandle(NEXT_HANDLE.fetch_add(1, Ordering::SeqCst));
        if let Ok(mut guard) = self.registry.lock() {
            let (order, map) = &mut *guard;
            map.insert(
                handle.0,
                GpuTextureSlot::new(texture, Arc::clone(&self.device_context)),
            );
            order.push_back(handle.0);
            while order.len() > STAGING_RING_SIZE {
                if let Some(oldest) = order.pop_front() {
                    map.remove(&oldest);
                }
            }
        }
        handle
    }

    fn release_staged(&self) {
        if let Ok(mut guard) = self.registry.lock() {
            guard.0.clear();
            guard.1.clear();
        }
    }
}

/// Resolves handles from one active WGC session. The staged texture is cloned
/// into the returned type-erased resource so a lease can outlive eviction from
/// the eight-entry registry while the encoder is using it.
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
                .registry
                .lock()
                .map_err(|_| GpuFrameResolveError::ContextUnavailable {
                    details: "capture staging registry is unavailable".into(),
                })?;
        let Some(slot) = guard.1.get(&handle.0) else {
            return Err(GpuFrameResolveError::StaleHandle { handle });
        };
        Ok(Arc::new(slot.clone()))
    }

    fn session_resource(&self) -> Option<Arc<GpuFrameResource>> {
        Some(self.shared.device_context.clone())
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
            stats.frames_dropped_queue_full,
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
        resolve_source(&config.source_id)?;

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
        let shared = Arc::new(SharedState::new(Arc::clone(&device_context)));

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
        let Some(events) = &mut self.events else {
            return Err(VideoCaptureError::Backend {
                details: "capture not started".into(),
            });
        };

        loop {
            if self.stop_flag.load(Ordering::SeqCst) {
                return Err(VideoCaptureError::EndOfStream);
            }
            match events.recv_timeout(Duration::from_millis(NEXT_EVENT_TICK_MS)) {
                Ok(InternalEvent::Frame {
                    handle,
                    width,
                    height,
                    source_ticks,
                }) => {
                    let pts_ms = match self.timeline.to_shared(source_ticks) {
                        Ok(pts) => pts,
                        Err(discontinuity) => {
                            if discontinuity.jump_ms > 0 {
                                // Forward jump (e.g. screen idle, locked, asleep, or monitor mode switch):
                                // Recalibrate video timeline so capture continues seamlessly!
                                self.timeline = MediaTimeline::new(
                                    TimeBase::from_hz(WGC_TIMESTAMP_HZ),
                                    MAX_TIMESTAMP_JUMP_MS,
                                );
                                self.timeline.to_shared(source_ticks).unwrap_or(0)
                            } else {
                                return Err(VideoCaptureError::Backend {
                                    details: discontinuity.to_string(),
                                });
                            }
                        }
                    };
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
                Ok(InternalEvent::FormatChanged { width, height }) => {
                    return Ok(VideoCaptureEvent::FormatChanged { width, height });
                }
                Ok(InternalEvent::SourceClosed) => {
                    return Ok(VideoCaptureEvent::SourceLost);
                }
                Ok(InternalEvent::Failed { details }) => {
                    return Err(VideoCaptureError::Backend { details });
                }
                Err(RecvTimeoutError::Timeout) => continue,
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
        let _ = event_tx.send(InternalEvent::Failed { details });
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
            let SessionSetup {
                pool,
                session,
                item,
                frame_token,
                closed_token,
            } = setup;
            if let Ok(Control::Stop) = control_rx.recv() {
                // fallthrough to cleanup via drop
            }
            let _ = pool.RemoveFrameArrived(frame_token);
            let _ = item.RemoveClosed(closed_token);
            let _ = session.Close();
            let _ = pool.Close();
            diagnostics::info("wgc", "session thread closing");
        }
        Err(err) => {
            diagnostics::error("wgc", &format!("session failed: {err}"));
            let _ = event_tx.send(InternalEvent::Failed {
                details: err.to_string(),
            });
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
    frame_token: i64,
    closed_token: i64,
}

/// Map `windows::core::Result` errors into our backend error at `?` sites.
trait IntoCapture<T> {
    fn cap(self) -> CaptureResult<T>;
}

impl<T> IntoCapture<T> for windows::core::Result<T> {
    fn cap(self) -> CaptureResult<T> {
        self.map_err(|err| VideoCaptureError::Backend {
            details: err.to_string(),
        })
    }
}

fn allocate_staging_pool(
    device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    width: u32,
    height: u32,
) -> CaptureResult<Vec<ID3D11Texture2D>> {
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
    let mut pool = Vec::with_capacity(STAGING_RING_SIZE);
    for _ in 0..STAGING_RING_SIZE {
        let mut texture: Option<ID3D11Texture2D> = None;
        unsafe {
            device
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .map_err(|err| VideoCaptureError::Backend {
                    details: format!("CreateTexture2D for staging pool failed: {err}"),
                })?;
        }
        if let Some(tex) = texture {
            pool.push(tex);
        } else {
            return Err(VideoCaptureError::Backend {
                details: "CreateTexture2D returned None".to_string(),
            });
        }
    }
    Ok(pool)
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
    let item: GraphicsCaptureItem = if let Some(hwnd_str) = source_id.strip_prefix("hwnd:") {
        let hwnd_val = hwnd_str.parse::<usize>().unwrap_or(0);
        let hwnd = windows::Win32::Foundation::HWND(hwnd_val as *mut core::ffi::c_void);
        unsafe { interop.CreateForWindow(hwnd) }.cap()?
    } else {
        let (_output, hmonitor_isize) = resolve_source(source_id)?;
        let hmonitor = HMONITOR(hmonitor_isize as *mut core::ffi::c_void);
        unsafe { interop.CreateForMonitor(hmonitor) }.cap()?
    };

    let initial_size = item.Size().cap()?;
    let initial_width = initial_size.Width.max(1) as u32;
    let initial_height = initial_size.Height.max(1) as u32;
    let initial_pool = allocate_staging_pool(&device, initial_width, initial_height)?;
    let staging_pool = Arc::new(Mutex::new(initial_pool));
    let ring_cursor = Arc::new(AtomicUsize::new(0));

    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &d3d_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        FRAME_POOL_BUFFERS,
        initial_size,
    )
    .cap()?;

    // Frame pump: drain whatever is available each wake-up, copying each
    // surface into a pre-allocated GPU texture from the staging ring under a unique handle.
    let handler_shared = Arc::clone(shared);
    let handler_size = Arc::new(Mutex::new(initial_size));
    // Pre-allocated texture pool is reused across frames (0 per-frame allocations).
    let texture_creator = SendDevice(device.clone());
    let copy_context = SendContext(context.clone());
    // Owned handle: Recreate takes the WinRT device by reference.
    let handler_device = SendWinrtDevice(d3d_device.clone());
    let handler_pool = pool.clone();
    let handler_tx = event_tx.clone();
    let handler_staging_pool = Arc::clone(&staging_pool);
    let handler_ring_cursor = Arc::clone(&ring_cursor);
    let frame_handler =
        TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_sender, _| {
            // Use our owned handle rather than the callback's guard ref.
            let pool = handler_pool.clone();
            let texture_creator = &texture_creator.0;
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
                    if guard.Width != content.Width || guard.Height != content.Height {
                        *guard = content;
                        let new_width = content.Width.max(1) as u32;
                        let new_height = content.Height.max(1) as u32;
                        if let Ok(new_pool) =
                            allocate_staging_pool(texture_creator, new_width, new_height)
                        {
                            if let Ok(mut pool_guard) = handler_staging_pool.lock() {
                                *pool_guard = new_pool;
                            }
                            handler_shared.release_staged();
                        }
                        let resized = pool.Recreate(
                            &winrt_device,
                            DirectXPixelFormat::B8G8R8A8UIntNormalized,
                            FRAME_POOL_BUFFERS,
                            content,
                        );
                        if let Ok(mut stats) = handler_shared.stats.lock() {
                            stats.resizes += 1;
                        }
                        let _ = handler_tx.try_send(InternalEvent::FormatChanged {
                            width: content.Width.max(0) as u32,
                            height: content.Height.max(0) as u32,
                        });
                        drop(guard);
                        if resized.is_err() {
                            break;
                        }
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
                let staged_texture = {
                    let pool_guard = match handler_staging_pool.lock() {
                        Ok(guard) => guard,
                        Err(_) => continue,
                    };
                    if pool_guard.is_empty() {
                        continue;
                    }
                    let index =
                        handler_ring_cursor.fetch_add(1, Ordering::Relaxed) % pool_guard.len();
                    pool_guard[index].clone()
                };

                unsafe {
                    context.CopyResource(&staged_texture, &texture);
                }

                let handle = handler_shared.insert_staged(staged_texture);
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
                    },
                );

                // `frame` drops here, releasing the pool surface reference.
            }
            Ok(())
        });
    let frame_token = pool.FrameArrived(&frame_handler).cap()?;

    // Monitor disappearance surfaces once as SourceLost (VID-007); the
    // engine decides recovery vs clean stop.
    let closed_tx = event_tx.clone();
    let closed_handler =
        TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
            let _ = closed_tx.try_send(InternalEvent::SourceClosed);
            Ok(())
        });
    let closed_token = item.Closed(&closed_handler).cap()?;

    let session = pool.CreateCaptureSession(&item).cap()?;
    session.SetIsCursorCaptureEnabled(cursor_capture).cap()?;
    let _ = session.SetIsBorderRequired(false);
    session.StartCapture().cap()?;
    diagnostics::info(
        "wgc",
        &format!(
            "session started ({}x{})",
            initial_size.Width, initial_size.Height
        ),
    );

    Ok(SessionSetup {
        pool,
        session,
        item,
        frame_token,
        closed_token,
    })
}
