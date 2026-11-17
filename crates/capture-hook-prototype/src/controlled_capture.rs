//! Development-only VideoCapture adapter for the original cooperative renderer.
//! GPU bridge is explicit; CPU mode remains a development comparison path.

use crate::cooperative_peer::FixturePeer;
use crate::delivery_gate::DeliveryGate;
use crate::local_gpu_bridge::{GpuPoolBudget, LocalGpuBridge};
use crate::publication_pool::Publication;
use crate::shared_transport::ConsumerRing;
use crate::timeline::QpcTimeline;
use crate::transport::Message;
use capture_api::{Result, VideoCapture, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent};
use encoder_api::GpuFrameContext;
use gpu_windows::{BgraReadback, GpuTextureSlot};
use media_types::{
    FramePayload, GpuFrameHandle, PixelFormat, StreamId, TimeBase, VideoFrame, VideoSourceInfo,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const SOURCE_ID: &str = "fixture:d3d11";
const FRAME_QUEUE_BOUND: usize = 3;

#[derive(Clone)]
pub struct ControlledSource {
    pub executable: PathBuf,
    pub warp: bool,
    /// Test-only fault: terminate the owned fixture after this many frames.
    pub crash_after_frames: Option<u64>,
    pub bridge_mode: BridgeMode,
    pub producer_mode: ProducerMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProducerMode {
    Commanded,
    CooperativeDll,
    #[cfg(feature = "resident-hooks")]
    ResidentDll,
}

#[derive(Clone, Copy, Debug)]
pub struct HelperRearm {
    pub width: u32,
    pub height: u32,
    pub recreate: bool,
}
#[derive(Clone, Default)]
pub struct HelperControl {
    pending: Arc<Mutex<Option<HelperRearm>>>,
    latest: Arc<Mutex<Option<Arc<CaptureStats>>>>,
    allow_rearm: bool,
}
impl HelperControl {
    pub fn stats(&self) -> Option<Arc<CaptureStats>> {
        self.latest.lock().ok()?.clone()
    }
    pub fn rearm(&self, request: HelperRearm) -> std::result::Result<(), String> {
        if !self.allow_rearm {
            return Err("resident/commanded capture does not support cooperative rearm".into());
        }
        if request.width == 0
            || request.height == 0
            || request.width > crate::MAX_WIDTH
            || request.height > crate::MAX_HEIGHT
            || !request.width.is_multiple_of(2)
            || !request.height.is_multiple_of(2)
        {
            return Err("invalid bounded/even helper dimensions".into());
        }
        let mut pending = self.pending.try_lock().map_err(|_| "helper control busy")?;
        if pending.is_some() {
            return Err("helper rearm mailbox full".into());
        }
        *pending = Some(request);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeMode {
    CpuReadback,
    Gpu,
}

struct CapturedEvent {
    event: VideoCaptureEvent,
    publication: Option<Publication<GpuTextureSlot>>,
    epoch: u64,
    gpu_update: Option<Arc<LocalGpuBridge>>,
}

struct WorkerRequest {
    source: ControlledSource,
    stream: StreamId,
    fps: u32,
    budget: Arc<GpuPoolBudget>,
    control: HelperControl,
}
struct GpuSession(Option<Arc<LocalGpuBridge>>);
impl Drop for GpuSession {
    fn drop(&mut self) {
        if let Some(gpu) = &self.0 {
            gpu.retire();
        }
    }
}

#[derive(Default)]
pub struct CaptureStats {
    pub copied: AtomicU64,
    pub dropped_queue_full: AtomicU64,
    pub dropped_pool_full: AtomicU64,
    pub gpu_copies: AtomicU64,
    pub cpu_readbacks: AtomicU64,
    pub worker_exited: AtomicBool,
    pub retired_events: AtomicU64,
    pub dropped_cadence: AtomicU64,
    pub helper_epochs: AtomicU64,
    delivery: DeliveryGate,
    process: Mutex<Option<std::os::windows::io::OwnedHandle>>,
    error: Mutex<Option<String>>,
}

pub struct ControlledCapture {
    source: ControlledSource,
    stream: StreamId,
    events: Option<Receiver<CapturedEvent>>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    stats: Arc<CaptureStats>,
    gpu: Option<Arc<LocalGpuBridge>>,
    delivered: Option<Publication<GpuTextureSlot>>,
    budget: Arc<GpuPoolBudget>,
    control: HelperControl,
}

fn backend(details: impl Into<String>) -> VideoCaptureError {
    VideoCaptureError::Backend {
        details: details.into(),
    }
}

impl ControlledCapture {
    pub fn new(source: ControlledSource, stream: StreamId) -> Self {
        let control = HelperControl {
            allow_rearm: source.producer_mode == ProducerMode::CooperativeDll,
            ..Default::default()
        };
        Self {
            source,
            stream,
            events: None,
            worker: None,
            stop: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(CaptureStats::default()),
            gpu: None,
            delivered: None,
            budget: Arc::new(GpuPoolBudget::default()),
            control,
        }
    }
    pub fn stats(&self) -> Arc<CaptureStats> {
        self.stats.clone()
    }
    pub fn helper_control(&self) -> Option<HelperControl> {
        (self.source.producer_mode != ProducerMode::Commanded).then(|| self.control.clone())
    }
    fn check_terminal(&mut self) -> Result<()> {
        let dead = !self.stats.source_live();
        let error = self
            .stats
            .error
            .lock()
            .map_err(|_| backend("capture failure state poisoned"))?
            .clone();
        if dead || error.is_some() || self.stats.delivery.current() == 0 {
            if let Some(events) = self.events.take() {
                while events.try_recv().is_ok() {
                    self.stats.retired_events.fetch_add(1, Ordering::Relaxed);
                }
            }
            self.delivered.take();
            if let Some(gpu) = self.gpu.take() {
                gpu.retire();
            }
            return Err(backend(error.unwrap_or_else(|| {
                if dead {
                    "controlled producer exited; pending media discarded".into()
                } else {
                    "controlled capture ended".into()
                }
            })));
        }
        Ok(())
    }
}

impl CaptureStats {
    pub fn delivery_epoch(&self) -> u64 {
        self.delivery.current()
    }
    fn source_live(&self) -> bool {
        use std::os::windows::io::AsRawHandle;
        self.process
            .lock()
            .map(|process| {
                process.as_ref().is_none_or(|process| {
                    (unsafe {
                        windows::Win32::System::Threading::WaitForSingleObject(
                            windows::Win32::Foundation::HANDLE(process.as_raw_handle()),
                            0,
                        )
                    }) == windows::Win32::Foundation::WAIT_TIMEOUT
                })
            })
            .unwrap_or(false)
    }
}

impl VideoCapture for ControlledCapture {
    fn enumerate_sources(&self) -> Result<Vec<VideoSourceInfo>> {
        Ok(vec![VideoSourceInfo {
            id: SOURCE_ID.into(),
            name: "Controlled D3D11 hook fixture (development only)".into(),
            is_primary: false,
            width: 640,
            height: 480,
        }])
    }
    fn start(&mut self, config: VideoCaptureConfig) -> Result<()> {
        if self.worker.is_some() {
            return Err(backend("controlled capture already started"));
        }
        if config.source_id != SOURCE_ID || !matches!(config.target_fps, 30 | 60 | 120) {
            return Err(VideoCaptureError::InvalidConfiguration { reason: "only the controlled fixture source at 30/60/120 FPS is supported; no desktop fallback".into() });
        }
        self.stop = Arc::new(AtomicBool::new(false));
        self.stats = Arc::new(CaptureStats::default());
        *self
            .control
            .latest
            .lock()
            .map_err(|_| backend("helper stats poisoned"))? = Some(self.stats.clone());
        self.control
            .pending
            .lock()
            .map_err(|_| backend("helper control poisoned"))?
            .take();
        let stop = self.stop.clone();
        let stats = self.stats.clone();
        let source = self.source.clone();
        let stream = self.stream;
        let request = WorkerRequest {
            source,
            stream,
            fps: config.target_fps,
            budget: self.budget.clone(),
            control: self.control.clone(),
        };
        let (events_tx, events_rx) = sync_channel(FRAME_QUEUE_BOUND);
        let (ready_tx, ready_rx) = sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("silk-controlled-hook-capture".into())
            .spawn(move || {
                let result = run(request, &stop, &stats, &events_tx, &ready_tx);
                if let Err(error) = result {
                    if !stop.load(Ordering::Acquire) {
                        if let Ok(mut stored) = stats.error.lock() {
                            *stored = Some(error.clone());
                        }
                    }
                    let _ = ready_tx.try_send(Err(error));
                }
                stats.delivery.close();
                stats.worker_exited.store(true, Ordering::Release);
            })
            .map_err(|error| backend(error.to_string()))?;
        self.events = Some(events_rx);
        self.worker = Some(worker);
        match ready_rx.recv_timeout(Duration::from_secs(12)) {
            Ok(Ok(gpu)) => {
                self.gpu = gpu;
                Ok(())
            }
            result => {
                let message = match result {
                    Ok(Err(error)) => error,
                    Err(error) => format!("controlled startup failed: {error}"),
                    _ => unreachable!(),
                };
                let _ = self.stop();
                Err(backend(message))
            }
        }
    }
    fn next_event(&mut self) -> Result<VideoCaptureEvent> {
        // Previous encode has completed before the next pull. Existing resolved
        // Arc leases still prevent reuse independently of this publication.
        self.delivered.take();
        loop {
            if self.stop.load(Ordering::Acquire) {
                return Err(VideoCaptureError::EndOfStream);
            }
            self.check_terminal()?;
            let events = self.events.as_ref().ok_or(VideoCaptureError::EndOfStream)?;
            if let Some(error) = self
                .stats
                .error
                .lock()
                .map_err(|_| backend("capture failure state poisoned"))?
                .as_ref()
            {
                return Err(backend(error.clone()));
            }
            match events.recv_timeout(Duration::from_millis(25)) {
                Ok(event) => {
                    self.check_terminal()?;
                    if !self.stats.delivery.accepts(event.epoch) {
                        self.stats.retired_events.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if let Some(gpu) = event.gpu_update {
                        self.gpu = Some(gpu);
                    }
                    self.delivered = event.publication;
                    return Ok(event.event);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if let Some(error) = self
                        .stats
                        .error
                        .lock()
                        .map_err(|_| backend("capture failure state poisoned"))?
                        .as_ref()
                    {
                        return Err(backend(error.clone()));
                    }
                    return Err(VideoCaptureError::EndOfStream);
                }
            }
        }
    }
    fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        self.delivered.take();
        if let Some(gpu) = self.gpu.take() {
            gpu.retire();
        }
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            worker
                .join()
                .map_err(|_| backend("controlled capture worker panicked"))?;
        }
        self.events = None;
        Ok(())
    }
    fn gpu_frame_context(&self) -> Option<GpuFrameContext> {
        if self.stop.load(Ordering::Acquire) {
            return None;
        }
        self.gpu.as_ref().map(|gpu| gpu.context())
    }
}
impl Drop for ControlledCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn run(
    request: WorkerRequest,
    stop: &AtomicBool,
    stats: &CaptureStats,
    events: &SyncSender<CapturedEvent>,
    ready: &SyncSender<std::result::Result<Option<Arc<LocalGpuBridge>>, String>>,
) -> std::result::Result<(), String> {
    if request.source.producer_mode != ProducerMode::Commanded {
        return run_helper(request, stop, stats, events, ready);
    }
    let WorkerRequest {
        source,
        stream,
        fps,
        budget,
        control: _,
    } = request;
    let mut peer = FixturePeer::launch_fixture(&source.executable, source.warp, None)?;
    let hello = match peer.receive_cancellable(Duration::from_secs(10), stop)? {
        Message::Hello(hello) => hello,
        _ => return Err("fixture sent media before its handshake".into()),
    };
    if (hello.width, hello.height) != (640, 480) {
        return Err("fixture changed dimensions without a source contract update".into());
    }
    let mut ring =
        unsafe { ConsumerRing::from_child(peer.handle(), peer.pid(), hello, source.warp) }?;
    let mut timeline = QpcTimeline::new(hello.qpc_frequency).ok_or("invalid QPC frequency")?;
    let mut readback = BgraReadback::default();
    let gpu = GpuSession(if source.bridge_mode == BridgeMode::Gpu {
        Some(LocalGpuBridge::new(
            ring.device_context(),
            hello.width,
            hello.height,
            budget,
        )?)
    } else {
        None
    });
    events
        .try_send(CapturedEvent {
            event: VideoCaptureEvent::FormatChanged {
                width: hello.width,
                height: hello.height,
            },
            publication: None,
            epoch: 1,
            gpu_update: None,
        })
        .map_err(|error| error.to_string())?;
    ready
        .try_send(Ok(gpu.0.clone()))
        .map_err(|error| error.to_string())?;
    let cadence = Duration::from_nanos(1_000_000_000 / u64::from(fps));
    let mut next_frame = Instant::now();
    let mut count = 0;
    while !stop.load(Ordering::Acquire) {
        if Instant::now() < next_frame {
            std::thread::park_timeout(next_frame.saturating_duration_since(Instant::now()));
            continue;
        }
        // No catch-up burst after a slow encoder or driver operation.
        next_frame = Instant::now() + cadence;
        if source
            .crash_after_frames
            .is_some_and(|limit| count >= limit)
        {
            peer.kill()?;
            return Err("controlled producer exited (injected lifecycle test)".into());
        }
        peer.send(Message::Render {
            count: 1,
            color: ((count / u64::from(fps)) % 3) as u32,
        })?;
        let header = match peer.receive_cancellable(Duration::from_secs(5), stop)? {
            Message::Frame(header) => header,
            _ => return Err("fixture did not publish the requested frame".into()),
        };
        let imported = ring.receive(header)?;
        let (payload, publication) = if let Some(gpu) = &gpu.0 {
            match gpu.copy_imported(imported.surface())? {
                Some(publication) => {
                    stats.gpu_copies.fetch_add(1, Ordering::Relaxed);
                    (
                        Some(FramePayload::Gpu(GpuFrameHandle(publication.handle()))),
                        Some(publication),
                    )
                }
                None => {
                    stats.dropped_pool_full.fetch_add(1, Ordering::Relaxed);
                    (None, None)
                }
            }
        } else {
            let bytes = readback
                .read_scaled(imported.surface(), header.width, header.height)
                .map_err(|error| error.to_string())?;
            stats.cpu_readbacks.fetch_add(1, Ordering::Relaxed);
            (Some(FramePayload::Cpu(bytes.into())), None)
        };
        let pts = timeline.milliseconds(header.qpc_ticks)?.max(0);
        let released = imported.release()?;
        peer.send(Message::Ack {
            session: released.session,
            slot: released.slot,
            sequence: released.sequence,
        })?;
        count += 1;
        match peer.receive_cancellable(Duration::from_secs(5), stop)? {
            Message::Report {
                presented,
                published,
                dropped,
            } if presented == count && published == count && dropped == 0 => {}
            _ => return Err("fixture report did not match the bounded capture request".into()),
        }
        stats.copied.fetch_add(1, Ordering::Relaxed);
        let Some(payload) = payload else {
            continue;
        };
        let frame = CapturedEvent {
            event: VideoCaptureEvent::Frame(VideoFrame {
                stream_id: stream,
                width: header.width,
                height: header.height,
                pixel_format: PixelFormat::Bgra8,
                pts,
                time_base: TimeBase::MILLISECOND,
                payload,
            }),
            publication,
            epoch: 1,
            gpu_update: None,
        };
        match events.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                stats.dropped_queue_full.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => return Ok(()),
        }
    }
    // Drop owns cancellation of this exact helper and its pipe reader. No UI or
    // encoder work is performed while joining the capture worker.
    Ok(())
}

fn helper_hello(
    peer: &mut FixturePeer,
    stop: &AtomicBool,
    expected: Option<(crate::SessionIdentity, HelperRearm)>,
) -> std::result::Result<crate::transport::Hello, String> {
    let Message::Select(selected) = peer.receive_cancellable(Duration::from_secs(8), stop)? else {
        return Err("helper omitted target offer".into());
    };
    unsafe {
        crate::shared_transport::validate_selected_child(peer.handle(), peer.pid(), selected)
    }?;
    let dimensions = if let Some((previous, rearm)) = expected {
        if selected.target != previous.target
            || selected.generation != previous.generation
            || selected.epoch
                != previous
                    .epoch
                    .checked_add(1)
                    .ok_or("helper epoch overflow")?
        {
            return Err("helper rearm changed approved target/generation/epoch".into());
        }
        (rearm.width, rearm.height)
    } else {
        if selected.generation != 1 || selected.epoch != 1 {
            return Err("invalid initial helper epoch".into());
        }
        (640, 480)
    };
    peer.send(Message::Select(selected))?;
    let Message::Hello(hello) = peer.receive_cancellable(Duration::from_secs(8), stop)? else {
        return Err("DLL omitted shared-ring greeting".into());
    };
    if hello.session != selected || (hello.width, hello.height) != dimensions {
        return Err("DLL changed approved source contract".into());
    }
    Ok(hello)
}

fn stop_helper(peer: &mut FixturePeer, stop: &AtomicBool) -> std::result::Result<(), String> {
    peer.send(Message::Stop)?;
    // At most three exported slots can be pending; permit bounded pipe metadata
    // but never import or deliver those retired frame headers.
    for _ in 0..8 {
        match peer.receive_cancellable(Duration::from_secs(5), stop)? {
            Message::Bye => return Ok(()),
            Message::Frame(_) => {}
            _ => return Err("unexpected retired helper response".into()),
        }
    }
    Err("helper Stop exceeded bounded drain".into())
}

fn helper_format(
    events: &SyncSender<CapturedEvent>,
    stop: &AtomicBool,
    stats: &CaptureStats,
    hello: crate::transport::Hello,
    gpu: &GpuSession,
) -> std::result::Result<(), String> {
    let mut event = CapturedEvent {
        event: VideoCaptureEvent::FormatChanged {
            width: hello.width,
            height: hello.height,
        },
        publication: None,
        epoch: hello.session.epoch,
        gpu_update: gpu.0.clone(),
    };
    loop {
        if stop.load(Ordering::Acquire) || !stats.source_live() {
            return Err("helper stopped/exited during format delivery".into());
        }
        match events.try_send(event) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Full(pending)) => {
                event = pending;
                std::thread::park_timeout(Duration::from_millis(2));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err("capture consumer disconnected".into())
            }
        }
    }
}

fn run_helper(
    request: WorkerRequest,
    stop: &AtomicBool,
    stats: &CaptureStats,
    events: &SyncSender<CapturedEvent>,
    ready: &SyncSender<std::result::Result<Option<Arc<LocalGpuBridge>>, String>>,
) -> std::result::Result<(), String> {
    let WorkerRequest {
        source,
        stream,
        fps,
        budget,
        control,
    } = request;
    #[cfg(feature = "resident-hooks")]
    let (mut peer, _resident_worker, initial_hello) = if source.producer_mode
        == ProducerMode::ResidentDll
    {
        let peer = FixturePeer::launch_passive_fixture(&source.executable, source.warp)?;
        let selected = crate::owned_attachment::select_owned_window(&peer)?;
        let worker = crate::owned_attachment::attach_owned_fixture(&peer, selected)?;
        let Message::Hello(hello) = peer.receive_cancellable(Duration::from_secs(15), stop)? else {
            return Err("resident worker omitted greeting".into());
        };
        if hello.session != selected || (hello.width, hello.height) != (640, 480) {
            return Err("resident discovery changed approved fixture contract".into());
        }
        (peer, Some(worker), Some(hello))
    } else {
        (
            FixturePeer::launch_helper_fixture(&source.executable, source.warp)?,
            None,
            None,
        )
    };
    #[cfg(not(feature = "resident-hooks"))]
    let (mut peer, initial_hello) = (
        FixturePeer::launch_helper_fixture(&source.executable, source.warp)?,
        None,
    );
    *stats
        .process
        .lock()
        .map_err(|_| "helper process state poisoned")? = Some(peer.owned_process_handle()?);
    let mut hello = match initial_hello {
        Some(hello) => hello,
        None => helper_hello(&mut peer, stop, None)?,
    };
    let mut ring =
        unsafe { ConsumerRing::from_child(peer.handle(), peer.pid(), hello, source.warp) }?;
    let mut gpu = GpuSession(if source.bridge_mode == BridgeMode::Gpu {
        Some(LocalGpuBridge::new(
            ring.device_context(),
            hello.width,
            hello.height,
            budget.clone(),
        )?)
    } else {
        None
    });
    let mut timeline =
        QpcTimeline::new(hello.qpc_frequency).ok_or("invalid helper QPC frequency")?;
    let mut readback = BgraReadback::default();
    helper_format(events, stop, stats, hello, &gpu)?;
    stats.helper_epochs.store(1, Ordering::Release);
    ready
        .try_send(Ok(gpu.0.clone()))
        .map_err(|error| error.to_string())?;
    let cadence = Duration::from_nanos(1_000_000_000 / u64::from(fps));
    let mut next_frame = Instant::now();
    let mut count = 0;
    while !stop.load(Ordering::Acquire) {
        if !peer.is_running() {
            return Err("controlled helper producer exited; pending media discarded".into());
        }
        if source
            .crash_after_frames
            .is_some_and(|limit| count >= limit)
        {
            peer.kill()?;
            peer.wait_status(Duration::from_secs(5))?;
            return Err("controlled helper producer exited (injected lifecycle test); pending media discarded".into());
        }
        let rearm = control
            .pending
            .try_lock()
            .ok()
            .and_then(|mut pending| pending.take());
        if let Some(rearm) = rearm {
            let next = hello
                .session
                .epoch
                .checked_add(1)
                .ok_or("helper epoch overflow")?;
            if !stats.delivery.advance(next) {
                return Err("helper delivery epoch closed".into());
            }
            if let Some(gpu) = &gpu.0 {
                gpu.retire();
            }
            stop_helper(&mut peer, stop)?;
            peer.send(Message::Rearm {
                epoch: next,
                width: rearm.width,
                height: rearm.height,
                recreate: rearm.recreate,
            })?;
            let replacement = helper_hello(&mut peer, stop, Some((hello.session, rearm)))?;
            if replacement.qpc_frequency != hello.qpc_frequency {
                return Err("helper QPC frequency changed across rearm".into());
            }
            ring = unsafe {
                ConsumerRing::from_child(peer.handle(), peer.pid(), replacement, source.warp)
            }?;
            gpu = GpuSession(if source.bridge_mode == BridgeMode::Gpu {
                Some(LocalGpuBridge::new(
                    ring.device_context(),
                    replacement.width,
                    replacement.height,
                    budget.clone(),
                )?)
            } else {
                None
            });
            hello = replacement;
            readback = BgraReadback::default();
            next_frame = Instant::now();
            helper_format(events, stop, stats, hello, &gpu)?;
            stats.helper_epochs.store(next, Ordering::Release);
        }
        let Message::Frame(header) = peer.receive_cancellable(Duration::from_secs(5), stop)? else {
            return Err("unexpected helper media packet".into());
        };
        if !peer.is_running() {
            return Err("controlled helper producer exited before import".into());
        }
        let imported = ring.receive(header)?;
        let pts = timeline.milliseconds(header.qpc_ticks)?;
        let due = Instant::now() >= next_frame;
        let (payload, publication) = if !due {
            stats.dropped_cadence.fetch_add(1, Ordering::Relaxed);
            (None, None)
        } else if let Some(gpu) = &gpu.0 {
            next_frame = Instant::now() + cadence;
            match gpu.copy_imported(imported.surface())? {
                Some(publication) => {
                    stats.gpu_copies.fetch_add(1, Ordering::Relaxed);
                    (
                        Some(FramePayload::Gpu(GpuFrameHandle(publication.handle()))),
                        Some(publication),
                    )
                }
                None => {
                    stats.dropped_pool_full.fetch_add(1, Ordering::Relaxed);
                    (None, None)
                }
            }
        } else {
            next_frame = Instant::now() + cadence;
            let bytes = readback
                .read_scaled(imported.surface(), header.width, header.height)
                .map_err(|error| error.to_string())?;
            stats.cpu_readbacks.fetch_add(1, Ordering::Relaxed);
            (Some(FramePayload::Cpu(bytes.into())), None)
        };
        let released = imported.release()?;
        if !peer.is_running() {
            return Err(
                "controlled helper producer exited during copy; pending media discarded".into(),
            );
        }
        peer.send(Message::Ack {
            session: released.session,
            slot: released.slot,
            sequence: released.sequence,
        })?;
        let Some(payload) = payload else {
            continue;
        };
        count += 1;
        stats.copied.fetch_add(1, Ordering::Relaxed);
        let frame = CapturedEvent {
            event: VideoCaptureEvent::Frame(VideoFrame {
                stream_id: stream,
                width: header.width,
                height: header.height,
                pixel_format: PixelFormat::Bgra8,
                pts,
                time_base: TimeBase::MILLISECOND,
                payload,
            }),
            publication,
            epoch: header.session.epoch,
            gpu_update: None,
        };
        if !stats.source_live() {
            return Err("controlled helper producer exited before publication".into());
        }
        match events.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                stats.dropped_queue_full.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => return Ok(()),
        }
    }
    // The exact owned child is cleaned up by FixturePeer. Graceful DLL Stop/EOF
    // behavior is separately tested; no arbitrary-app termination is authorized.
    Ok(())
}

#[cfg(test)]
mod helper_control_tests {
    use super::*;
    #[test]
    fn rearm_mailbox_is_bounded_and_validates_before_enqueue() {
        assert!(HelperControl::default()
            .rearm(HelperRearm {
                width: 800,
                height: 600,
                recreate: false
            })
            .is_err());
        let control = HelperControl {
            allow_rearm: true,
            ..Default::default()
        };
        for (width, height) in [
            (0, 480),
            (641, 480),
            (640, 481),
            (crate::MAX_WIDTH + 2, 480),
        ] {
            assert!(control
                .rearm(HelperRearm {
                    width,
                    height,
                    recreate: false
                })
                .is_err());
        }
        let request = HelperRearm {
            width: 800,
            height: 600,
            recreate: false,
        };
        control.rearm(request).unwrap();
        assert!(control.rearm(request).is_err());
        assert_eq!(control.pending.lock().unwrap().take().unwrap().width, 800);
        control.rearm(request).unwrap();
    }
}
