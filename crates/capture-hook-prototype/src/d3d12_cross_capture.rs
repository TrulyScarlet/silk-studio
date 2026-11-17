//! Recorder adapter for exact owned D3D12 child GPU transport; never a game backend.
use crate::d3d12_cross_process::{is_owned_loss_probe, Peer, RemoteConsumer};
use crate::d3d12_protocol::Wire;
use crate::delivery_gate::DeliveryGate;
use crate::local_gpu_bridge::{GpuPoolBudget, LocalGpuBridge};
use crate::publication_pool::Publication;
use crate::timeline::{FrameCadence, QpcTimeline};
use crate::transport::Message;
use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent};
use encoder_api::GpuFrameContext;
use gpu_windows::GpuTextureSlot;
use media_types::{
    FramePayload, GpuFrameHandle, PixelFormat, StreamId, TimeBase, VideoFrame, VideoSourceInfo,
};
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, WAIT_TIMEOUT};
use windows::Win32::System::Threading::WaitForSingleObject;

pub const SOURCE_ID: &str = "owned-d3d12-cross-process-gpu";
pub const HOOK_SOURCE_ID: &str = "owned-d3d12-cross-process-present-hook";
pub const FACTORY_SOURCE_ID: &str = "owned-d3d12-factory-and-present-hook";
pub const BOOTSTRAP_SOURCE_ID: &str = "owned-d3d12-independent-bootstrap";
pub const ATTACHED_SOURCE_ID: &str = "owned-d3d12-early-remote-helper";
pub const LATE_RESIZE_SOURCE_ID: &str = "owned-d3d12-late-resize-helper";
pub const AUTOMATIC_RESIZE_SOURCE_ID: &str = "owned-d3d12-observed-resize-helper";
pub const REARM_SOURCE_ID: &str = "owned-d3d12-live-resize";
pub const DEVICE_REPLACE_SOURCE_ID: &str = "owned-d3d12-device-recreation";
type ResizeMailbox = Arc<Mutex<Option<ControlRoute>>>;
struct ControlRoute {
    sender: SyncSender<RearmAction>,
    replace_device: bool,
    resize_enabled: bool,
    loss_probe: bool,
}
enum RearmAction {
    Resize(u32, u32),
    ReplaceDevice(u32, u32),
    TriggerLoss,
}
#[derive(Clone, Default)]
pub struct CrossControl {
    latest: Arc<Mutex<Option<Arc<CrossStats>>>>,
    resize: ResizeMailbox,
}
impl CrossControl {
    pub fn trigger_owned_loss(&self) -> Result<(), String> {
        let route = self
            .resize
            .lock()
            .map_err(|_| "owned loss control poisoned")?;
        let route = route
            .as_ref()
            .filter(|route| route.loss_probe)
            .ok_or("owned loss probe route not active")?;
        route
            .sender
            .try_send(RearmAction::TriggerLoss)
            .map_err(|error| format!("bounded owned loss mailbox unavailable: {error}"))
    }
    pub fn resize(&self, width: u32, height: u32) -> Result<(), String> {
        self.request(RearmAction::Resize(width, height))
    }
    pub fn replace_device(&self, width: u32, height: u32) -> Result<(), String> {
        self.request(RearmAction::ReplaceDevice(width, height))
    }
    fn request(&self, action: RearmAction) -> Result<(), String> {
        let (width, height) = match action {
            RearmAction::Resize(w, h) | RearmAction::ReplaceDevice(w, h) => (w, h),
            RearmAction::TriggerLoss => return Err("loss probe is not a resize request".into()),
        };
        if width < 2
            || height < 2
            || width > 1920
            || height > 1080
            || !width.is_multiple_of(2)
            || !height.is_multiple_of(2)
        {
            return Err("invalid bounded resize dimensions".into());
        }
        let route = self.resize.lock().map_err(|_| "resize control poisoned")?;
        let route = route.as_ref().ok_or("rearm route not active")?;
        if !route.resize_enabled {
            return Err("resize is not enabled on this route".into());
        }
        if matches!(action, RearmAction::ReplaceDevice(..)) && !route.replace_device {
            return Err("device replacement not enabled on this route".into());
        }
        route
            .sender
            .try_send(action)
            .map_err(|e| format!("bounded resize mailbox unavailable: {e}"))
    }
    pub fn stats(&self) -> Option<Arc<CrossStats>> {
        self.latest.lock().ok()?.clone()
    }
}
#[derive(Clone)]
pub struct CrossSource {
    pub executable: PathBuf,
    pub warp: bool,
    pub fault: Option<String>,
    pub present_hook: bool,
    pub factory_hook: bool,
    pub bootstrap: bool,
    pub legacy_factory: bool,
    pub attached: bool,
    pub late_resize: bool,
    pub automatic_resize: bool,
    pub rearm: bool,
    pub device_replace: bool,
}
pub struct CrossStats {
    target: Mutex<Option<crate::TargetIdentity>>,
    process: Mutex<Option<OwnedHandle>>,
    error: Mutex<Option<String>>,
    delivery: DeliveryGate,
    pub worker_exited: AtomicBool,
    pub gpu_copies: AtomicU64,
    pub dropped: AtomicU64,
    pub cpu_readbacks: AtomicU64,
    pub retired_events: AtomicU64,
}
impl Default for CrossStats {
    fn default() -> Self {
        Self {
            target: Mutex::new(None),
            process: Mutex::new(None),
            error: Mutex::new(None),
            delivery: DeliveryGate::default(),
            worker_exited: AtomicBool::new(true),
            gpu_copies: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            cpu_readbacks: AtomicU64::new(0),
            retired_events: AtomicU64::new(0),
        }
    }
}
impl CrossStats {
    pub fn owned_target(&self) -> Option<crate::TargetIdentity> {
        if !self.source_live() {
            return None;
        }
        *self.target.lock().ok()?
    }
    pub fn current_epoch(&self) -> u64 {
        self.delivery.current()
    }
    pub fn source_live(&self) -> bool {
        self.process
            .lock()
            .map(|p| {
                p.as_ref().is_some_and(|p| {
                    (unsafe { WaitForSingleObject(HANDLE(p.as_raw_handle()), 0) }) == WAIT_TIMEOUT
                })
            })
            .unwrap_or(false)
    }
}
enum Event {
    Retire {
        epoch: u64,
        ready: SyncSender<()>,
    },
    Frame {
        frame: VideoFrame,
        publication: Publication<GpuTextureSlot>,
        epoch: u64,
    },
    Format {
        gpu: Arc<LocalGpuBridge>,
        width: u32,
        height: u32,
        epoch: u64,
    },
}
pub struct CrossCapture {
    source: CrossSource,
    stream: StreamId,
    budget: Arc<GpuPoolBudget>,
    stats: Arc<CrossStats>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    events: Option<Receiver<Event>>,
    delivered: Option<Publication<GpuTextureSlot>>,
    gpu: Option<Arc<LocalGpuBridge>>,
    format: bool,
    control: CrossControl,
}
fn backend(details: impl Into<String>) -> VideoCaptureError {
    VideoCaptureError::Backend {
        details: details.into(),
    }
}
impl CrossCapture {
    pub fn new(source: CrossSource, stream: StreamId) -> Self {
        Self {
            source,
            stream,
            budget: Arc::new(GpuPoolBudget::default()),
            stats: Arc::new(CrossStats::default()),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            events: None,
            delivered: None,
            gpu: None,
            format: false,
            control: CrossControl::default(),
        }
    }
    pub fn stats(&self) -> Arc<CrossStats> {
        self.stats.clone()
    }
    pub fn control(&self) -> CrossControl {
        self.control.clone()
    }
    fn source_id(&self) -> &'static str {
        if self.source.automatic_resize {
            AUTOMATIC_RESIZE_SOURCE_ID
        } else if self.source.late_resize {
            LATE_RESIZE_SOURCE_ID
        } else if self.source.attached {
            ATTACHED_SOURCE_ID
        } else if self.source.device_replace {
            DEVICE_REPLACE_SOURCE_ID
        } else if self.source.rearm {
            REARM_SOURCE_ID
        } else if self.source.bootstrap {
            BOOTSTRAP_SOURCE_ID
        } else if self.source.factory_hook {
            FACTORY_SOURCE_ID
        } else if self.source.present_hook {
            HOOK_SOURCE_ID
        } else {
            SOURCE_ID
        }
    }
    fn check_terminal(&mut self) -> capture_api::Result<()> {
        let error = self
            .stats
            .error
            .lock()
            .map_err(|_| backend("D3D12 capture error state poisoned"))?
            .clone();
        if !self.stats.source_live() || self.stats.delivery.current() == 0 || error.is_some() {
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
                "owned D3D12 producer exited/ended; queued media discarded".into()
            })));
        }
        Ok(())
    }
}
impl VideoCapture for CrossCapture {
    fn enumerate_sources(&self) -> capture_api::Result<Vec<VideoSourceInfo>> {
        Ok(vec![VideoSourceInfo {
            id: self.source_id().into(),
            name: "Owned D3D12 cross-process GPU fixture".into(),
            is_primary: false,
            width: 318,
            height: 178,
        }])
    }
    fn start(&mut self, config: VideoCaptureConfig) -> capture_api::Result<()> {
        if self.worker.is_some()
            || config.source_id != self.source_id()
            || config.target_fps != 30
            || (self.source.present_hook && !cfg!(feature = "d3d12-present-hook-fixture"))
            || (self.source.factory_hook
                && (!self.source.present_hook || !cfg!(feature = "d3d12-factory-hook-fixture")))
            || (self.source.bootstrap
                && (!self.source.factory_hook || !cfg!(feature = "d3d12-bootstrap-fixture")))
            || (self.source.legacy_factory
                && (!self.source.bootstrap
                    || self.source.rearm
                    || self.source.device_replace
                    || self.source.fault.is_some()))
            || (self.source.attached
                && (!cfg!(feature = "d3d12-remote-helper-fixture")
                    || !self.source.bootstrap
                    || self.source.rearm
                    || self.source.device_replace
                    || self.source.fault.is_some()))
            || (self.source.late_resize
                && (!self.source.attached
                    || !cfg!(feature = "d3d12-late-resize-fixture")
                    || self.source.legacy_factory))
            || (self.source.automatic_resize
                && (!self.source.late_resize
                    || self.source.rearm
                    || self.source.device_replace
                    || !cfg!(feature = "d3d12-auto-resize-fixture")))
            || (self.source.rearm
                && (!self.source.bootstrap || !cfg!(feature = "d3d12-rearm-fixture")))
            || (self.source.device_replace
                && (!self.source.rearm || !cfg!(feature = "d3d12-device-replace-fixture")))
            || (is_owned_loss_probe(self.source.fault.as_deref())
                && (self.source.warp
                    || self.source.present_hook
                    || self.source.factory_hook
                    || self.source.bootstrap
                    || self.source.attached
                    || self.source.rearm
                    || self.source.device_replace
                    || !cfg!(feature = "d3d12-device-replace-fixture")))
        {
            return Err(VideoCaptureError::InvalidConfiguration {
                reason:
                    "only exact owned D3D12 source at 30 FPS, no double start or monitor fallback"
                        .into(),
            });
        }
        self.stats = Arc::new(CrossStats::default());
        *self
            .control
            .latest
            .lock()
            .map_err(|_| backend("D3D12 stats control poisoned"))? = Some(self.stats.clone());
        self.stats.worker_exited.store(false, Ordering::Release);
        self.stop = Arc::new(AtomicBool::new(false));
        let source = self.source.clone();
        let budget = self.budget.clone();
        let stats = self.stats.clone();
        let stop = self.stop.clone();
        let stream = self.stream;
        let (send, events) = sync_channel(3);
        let (ready, started) = sync_channel(1);
        let (resize_tx, resize_rx) = sync_channel(1);
        *self
            .control
            .resize
            .lock()
            .map_err(|_| backend("resize control poisoned"))? = (self.source.rearm
            || is_owned_loss_probe(self.source.fault.as_deref()))
        .then_some(ControlRoute {
            sender: resize_tx,
            replace_device: self.source.device_replace,
            resize_enabled: self.source.rearm,
            loss_probe: is_owned_loss_probe(self.source.fault.as_deref()),
        });
        let worker = std::thread::Builder::new()
            .name("silk-owned-d3d12-cross-capture".into())
            .spawn(move || {
                let result = run(
                    source,
                    budget,
                    stream,
                    &stop,
                    &stats,
                    RunChannels {
                        events: &send,
                        ready: &ready,
                        requests: &resize_rx,
                    },
                );
                if let Err(error) = result {
                    if !stop.load(Ordering::Acquire) {
                        if let Ok(mut stored) = stats.error.lock() {
                            *stored = Some(error.clone());
                        }
                    }
                    let _ = ready.try_send(Err(error));
                }
                stats.delivery.close();
                stats.worker_exited.store(true, Ordering::Release);
            })
            .map_err(|e| backend(e.to_string()))?;
        self.events = Some(events);
        self.worker = Some(worker);
        match started.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(gpu)) => {
                self.gpu = Some(gpu);
                self.format = true;
                Ok(())
            }
            result => {
                let details = match result {
                    Ok(Err(e)) => e,
                    Err(e) => e.to_string(),
                    _ => unreachable!(),
                };
                let _ = self.stop();
                Err(backend(details))
            }
        }
    }
    fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
        self.delivered.take();
        loop {
            if self.stop.load(Ordering::Acquire) {
                return Err(VideoCaptureError::EndOfStream);
            }
            self.check_terminal()?;
            if self.format {
                self.format = false;
                return Ok(VideoCaptureEvent::FormatChanged {
                    width: 318,
                    height: 178,
                });
            }
            match self
                .events
                .as_ref()
                .ok_or(VideoCaptureError::EndOfStream)?
                .recv_timeout(Duration::from_millis(25))
            {
                Ok(event) => {
                    self.check_terminal()?;
                    let epoch = match &event {
                        Event::Frame { epoch, .. }
                        | Event::Format { epoch, .. }
                        | Event::Retire { epoch, .. } => *epoch,
                    };
                    if !self.stats.delivery.accepts(epoch) {
                        self.stats.retired_events.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    match event {
                        Event::Retire { ready, .. } => {
                            if let Some(old) = self.gpu.take() {
                                old.retire();
                            }
                            ready.try_send(()).map_err(|e| backend(e.to_string()))?;
                            continue;
                        }
                        Event::Frame {
                            frame, publication, ..
                        } => {
                            self.delivered = Some(publication);
                            return Ok(VideoCaptureEvent::Frame(frame));
                        }
                        Event::Format {
                            gpu, width, height, ..
                        } => {
                            if let Some(old) = self.gpu.replace(gpu) {
                                old.retire();
                            }
                            return Ok(VideoCaptureEvent::FormatChanged { width, height });
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.check_terminal()?;
                    return Err(VideoCaptureError::EndOfStream);
                }
            }
        }
    }
    fn stop(&mut self) -> capture_api::Result<()> {
        if let Ok(mut resize) = self.control.resize.lock() {
            resize.take();
        }
        self.stop.store(true, Ordering::Release);
        self.stats.delivery.close();
        self.delivered.take();
        if let Some(gpu) = self.gpu.take() {
            gpu.retire();
        }
        self.events.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| backend("owned D3D12 capture worker panicked"))?;
        }
        Ok(())
    }
    fn gpu_frame_context(&self) -> Option<GpuFrameContext> {
        if self.stop.load(Ordering::Acquire) || !self.stats.source_live() {
            None
        } else {
            self.gpu.as_ref().map(|g| g.context())
        }
    }
}
impl Drop for CrossCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
struct RunChannels<'a> {
    events: &'a SyncSender<Event>,
    ready: &'a SyncSender<Result<Arc<LocalGpuBridge>, String>>,
    requests: &'a Receiver<RearmAction>,
}
fn run(
    source: CrossSource,
    budget: Arc<GpuPoolBudget>,
    stream: StreamId,
    stop: &AtomicBool,
    stats: &CrossStats,
    channels: RunChannels<'_>,
) -> Result<(), String> {
    let RunChannels {
        events,
        ready,
        requests,
    } = channels;
    let peer = if source.automatic_resize {
        #[cfg(feature = "d3d12-auto-resize-fixture")]
        {
            Peer::launch_automatic_resize(&source.executable, source.warp)?
        }
        #[cfg(not(feature = "d3d12-auto-resize-fixture"))]
        {
            return Err("automatic resize feature disabled".into());
        }
    } else if source.late_resize {
        #[cfg(feature = "d3d12-late-resize-fixture")]
        {
            Peer::launch_late_resize(&source.executable, source.warp)?
        }
        #[cfg(not(feature = "d3d12-late-resize-fixture"))]
        {
            return Err("owned late resize feature disabled".into());
        }
    } else if source.attached {
        #[cfg(feature = "d3d12-remote-helper-fixture")]
        {
            Peer::launch_passive_factory(&source.executable, source.warp, source.legacy_factory)?
        }
        #[cfg(not(feature = "d3d12-remote-helper-fixture"))]
        {
            return Err("owned D3D12 remote helper feature disabled".into());
        }
    } else if source.legacy_factory {
        #[cfg(feature = "d3d12-bootstrap-fixture")]
        {
            Peer::launch_legacy_factory(&source.executable, source.warp)?
        }
        #[cfg(not(feature = "d3d12-bootstrap-fixture"))]
        {
            return Err("legacy factory requires owned bootstrap feature".into());
        }
    } else if source.device_replace {
        #[cfg(feature = "d3d12-device-replace-fixture")]
        {
            Peer::launch_device_replace(&source.executable, source.warp, source.fault.as_deref())?
        }
        #[cfg(not(feature = "d3d12-device-replace-fixture"))]
        {
            return Err("owned device replacement feature not enabled".into());
        }
    } else if source.rearm {
        #[cfg(feature = "d3d12-rearm-fixture")]
        {
            Peer::launch_rearm(&source.executable, source.warp, source.fault.as_deref())?
        }
        #[cfg(not(feature = "d3d12-rearm-fixture"))]
        {
            return Err("owned resize feature not enabled".into());
        }
    } else if source.bootstrap {
        #[cfg(feature = "d3d12-bootstrap-fixture")]
        {
            Peer::launch_bootstrap(&source.executable, source.warp, source.fault.as_deref())?
        }
        #[cfg(not(feature = "d3d12-bootstrap-fixture"))]
        {
            return Err("owned bootstrap feature not enabled".into());
        }
    } else if source.factory_hook {
        #[cfg(feature = "d3d12-factory-hook-fixture")]
        {
            Peer::launch_factory_hook(&source.executable, source.warp, source.fault.as_deref())?
        }
        #[cfg(not(feature = "d3d12-factory-hook-fixture"))]
        {
            return Err("owned factory hook feature not enabled".into());
        }
    } else if source.present_hook {
        #[cfg(feature = "d3d12-present-hook-fixture")]
        {
            Peer::launch_hook(&source.executable, source.warp, source.fault.as_deref())?
        }
        #[cfg(not(feature = "d3d12-present-hook-fixture"))]
        {
            return Err("owned Present hook feature not enabled".into());
        }
    } else {
        Peer::launch(&source.executable, source.warp, source.fault.as_deref())?
    };
    *stats
        .process
        .lock()
        .map_err(|_| "D3D12 process state poisoned")? = Some(peer.process_handle()?);
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    let attached_worker = if source.attached {
        let Wire::Control(Message::Select(session)) =
            peer.receive_cancellable(Duration::from_secs(10), stop)?
        else {
            return Err("missing passive target identity".into());
        };
        if source.late_resize {
            let Wire::Control(Message::Report {
                presented,
                published: 0,
                dropped: 0,
            }) = peer.receive_cancellable(Duration::from_secs(5), stop)?
            else {
                return Err("missing pre-load target rendering proof".into());
            };
            if presented < 12 {
                return Err("late target was not already rendering".into());
            }
        }
        let worker = crate::owned_attachment::attach_owned_d3d12_fixture(&peer, session)?;
        if peer.receive_cancellable(Duration::from_secs(10), stop)?
            != Wire::Control(Message::Report {
                presented: 0,
                published: 0,
                dropped: 0,
            })
        {
            return Err("missing remote factory readiness".into());
        }
        peer.send(Message::Select(session))?;
        Some((worker, session))
    } else {
        None
    };
    let Wire::Hello(hello) = peer.receive_cancellable(Duration::from_secs(10), stop)? else {
        return Err("missing D3D12 GPU greeting".into());
    };
    *stats
        .target
        .lock()
        .map_err(|_| "owned target metadata poisoned")? = Some(hello.session.target);
    #[cfg(feature = "d3d12-remote-helper-fixture")]
    if attached_worker
        .as_ref()
        .is_some_and(|(_, selected)| hello.session != *selected)
    {
        return Err("remote helper greeting changed the pinned passive selection".into());
    }
    let mut consumer = RemoteConsumer::open(&peer, hello, source.warp, budget.clone())?;
    let gpu = consumer.output();
    peer.approve(hello)?;
    ready.try_send(Ok(gpu)).map_err(|e| e.to_string())?;
    let mut clock = QpcTimeline::new(hello.qpc_frequency).ok_or("invalid D3D12 QPC frequency")?;
    let mut cadence = FrameCadence::new(30).ok_or("invalid fixture cadence")?;
    let mut queue_drops = 0;
    let mut epoch = crate::resize_epoch::ResizeEpoch::new(hello)?;
    let loss_probe = is_owned_loss_probe(source.fault.as_deref());
    let recover_loss = source.fault.as_deref() == Some("remove-and-recreate-on-command");
    let mut loss_waiting_since = None;
    let mut current = hello;
    let mut pending = false;
    let mut copies_base = 0;
    let mut drops_base = 0;
    while !stop.load(Ordering::Acquire) {
        if !peer.is_running() {
            return Err("owned D3D12 producer exited before receive".into());
        }
        if (source.rearm || loss_probe) && !pending {
            if let Ok(action) = requests.try_recv() {
                if matches!(action, RearmAction::TriggerLoss) {
                    if !loss_probe {
                        return Err("unsolicited owned loss trigger".into());
                    }
                    #[cfg(feature = "d3d12-device-replace-fixture")]
                    peer.trigger_owned_loss(current.session)?;
                    #[cfg(not(feature = "d3d12-device-replace-fixture"))]
                    return Err("owned loss probe feature disabled".into());
                    #[cfg(feature = "d3d12-device-replace-fixture")]
                    continue;
                }
                let (width, height) = match action {
                    RearmAction::Resize(w, h) | RearmAction::ReplaceDevice(w, h) => (w, h),
                    RearmAction::TriggerLoss => unreachable!(),
                };
                let request = crate::d3d12_protocol::ResizeRequest {
                    session: current.session,
                    width,
                    height,
                };
                epoch.begin(request)?;
                match action {
                    RearmAction::Resize(..) => peer.resize(request)?,
                    RearmAction::TriggerLoss => {
                        return Err("loss trigger entered resize dispatch".into())
                    }
                    RearmAction::ReplaceDevice(..) => {
                        #[cfg(feature = "d3d12-device-replace-fixture")]
                        {
                            peer.replace_device(request)?;
                        }
                        #[cfg(not(feature = "d3d12-device-replace-fixture"))]
                        {
                            return Err("device replacement feature disabled".into());
                        }
                    }
                }
                pending = true;
            }
        }
        let wire = peer.receive_cancellable(Duration::from_secs(5), stop)?;
        if let Wire::DeviceLost(notice) = wire {
            if !loss_probe || pending {
                return Err("unsolicited/overlapping producer-loss notice".into());
            }
            let mut loss = crate::device_loss_epoch::DeviceLossEpoch::new(current)
                .map_err(|error| format!("invalid loss epoch: {error:?}"))?;
            let next = loss
                .begin(notice)
                .map_err(|error| format!("invalid loss notice: {error:?}"))?;
            if !stats.delivery.advance(next.epoch) {
                return Err("loss delivery epoch did not advance".into());
            }
            let (ready, crossed) = sync_channel(1);
            enqueue_transition(
                events,
                Event::Retire {
                    epoch: next.epoch,
                    ready,
                },
                stop,
            )?;
            crossed
                .recv_timeout(Duration::from_secs(5))
                .map_err(|error| format!("loss delivery barrier unavailable: {error}"))?;
            consumer.retire_epoch_after_producer_loss()?;
            loss.retired(next)
                .map_err(|error| format!("loss retirement mismatch: {error:?}"))?;
            if recover_loss {
                copies_base += consumer.gpu_copies;
                drops_base += consumer.dropped;
                drop(consumer);
                peer.retired_epoch(next)?;
                let Wire::Hello(replacement) =
                    peer.receive_cancellable(Duration::from_secs(8), stop)?
                else {
                    return Err("missing owned loss replacement handles".into());
                };
                loss.offer(replacement)
                    .map_err(|error| format!("invalid owned loss replacement: {error:?}"))?;
                consumer = RemoteConsumer::open(&peer, replacement, source.warp, budget.clone())?;
                enqueue_transition(
                    events,
                    Event::Format {
                        gpu: consumer.output(),
                        width: replacement.width,
                        height: replacement.height,
                        epoch: next.epoch,
                    },
                    stop,
                )?;
                peer.approve(replacement)?;
                current = loss
                    .approve(next)
                    .map_err(|error| format!("owned loss approval mismatch: {error:?}"))?;
                loss_waiting_since = None;
                continue;
            }
            loss.close();
            return Err(format!("owned producer device lost ({:?}); old imports retired after delivery boundary; automatic replacement unavailable, no monitor fallback", notice.reason));
        }
        if let Wire::Resize(request) = wire {
            if !source.automatic_resize || pending {
                return Err("unsolicited/overlapping observed resize".into());
            }
            epoch.begin(request)?;
            pending = true;
            continue;
        }
        if let Wire::Retire(next) = wire {
            if !(source.rearm || source.automatic_resize) || !pending || next != epoch.expected()? {
                return Err("unsolicited/mismatched epoch retirement".into());
            }
            if !stats.delivery.advance(next.epoch) {
                return Err("resize delivery epoch did not advance".into());
            }
            // Let next_event cross the old delivered-frame/resolver gap before
            // invalidating its context. Encoder-held resolved leases remain valid.
            let (acknowledge, crossed) = sync_channel(1);
            enqueue_transition(
                events,
                Event::Retire {
                    epoch: next.epoch,
                    ready: acknowledge,
                },
                stop,
            )?;
            crossed
                .recv_timeout(Duration::from_secs(5))
                .map_err(|e| format!("resize delivery barrier unavailable: {e}"))?;
            copies_base += consumer.gpu_copies;
            drops_base += consumer.dropped;
            consumer.retire_epoch()?;
            drop(consumer);
            peer.retired_epoch(next)?;
            let Wire::Hello(replacement) =
                peer.receive_cancellable(Duration::from_secs(8), stop)?
            else {
                return Err("missing replacement GPU handles".into());
            };
            epoch.offer(replacement)?;
            consumer = RemoteConsumer::open(&peer, replacement, source.warp, budget.clone())?;
            enqueue_transition(
                events,
                Event::Format {
                    gpu: consumer.output(),
                    width: replacement.width,
                    height: replacement.height,
                    epoch: next.epoch,
                },
                stop,
            )?;
            peer.approve(replacement)?;
            current = replacement;
            pending = false;
            continue;
        }
        let Wire::Control(Message::Frame(header)) = wire else {
            return Err("unexpected D3D12 frame-stream packet".into());
        };
        if !peer.is_running() {
            return Err("owned D3D12 producer exited with queued frames".into());
        }
        let pts = clock.milliseconds(header.qpc_ticks)?;
        let admitted = cadence.admit(pts)?;
        let (publication, ack) = if loss_probe {
            match consumer.copy_if_preserving_loss(header, admitted)? {
                crate::d3d12_cross_process::RemoteCopyOutcome::Copied(publication, ack) => {
                    (publication, ack)
                }
                crate::d3d12_cross_process::RemoteCopyOutcome::ProducerLost => {
                    let since = loss_waiting_since.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() >= Duration::from_secs(5) {
                        return Err("producer removal lacked a bounded typed loss notice".into());
                    }
                    continue;
                }
            }
        } else {
            consumer.copy_if(header, admitted)?
        };
        peer.send(ack)?;
        stats
            .gpu_copies
            .store(copies_base + consumer.gpu_copies, Ordering::Release);
        if let Some(publication) = publication {
            if !peer.is_running() {
                return Err("owned D3D12 producer exited before publication".into());
            }
            let frame = VideoFrame {
                stream_id: stream,
                width: header.width,
                height: header.height,
                pixel_format: PixelFormat::Bgra8,
                pts,
                time_base: TimeBase::MILLISECOND,
                payload: FramePayload::Gpu(GpuFrameHandle(publication.handle())),
            };
            if events
                .try_send(Event::Frame {
                    frame,
                    publication,
                    epoch: header.session.epoch,
                })
                .is_err()
            {
                queue_drops += 1;
            }
        }
        stats.dropped.store(
            drops_base + consumer.dropped + queue_drops,
            Ordering::Release,
        );
    }
    consumer.close();
    Ok(())
}

fn enqueue_transition(
    events: &SyncSender<Event>,
    mut event: Event,
    stop: &AtomicBool,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match events.try_send(event) {
            Ok(()) => return Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(held)) => {
                event = held;
                if stop.load(Ordering::Acquire) || std::time::Instant::now() >= deadline {
                    return Err("resize event delivery deadline".into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    #[test]
    fn rearm_controls_validate_route_dimensions_and_bounded_mailbox() {
        let control = CrossControl::default();
        assert!(control.trigger_owned_loss().is_err());
        assert!(control.replace_device(640, 360).is_err());
        let (sender, requests) = sync_channel(1);
        *control.resize.lock().unwrap() = Some(ControlRoute {
            sender,
            replace_device: false,
            resize_enabled: true,
            loss_probe: false,
        });
        assert!(control.trigger_owned_loss().is_err());
        assert!(control.replace_device(640, 360).is_err());
        for (width, height) in [(0, 360), (641, 360), (640, 361), (1922, 360), (640, 1082)] {
            assert!(control.resize(width, height).is_err());
        }
        assert!(requests.try_recv().is_err());
        control.resize(640, 360).unwrap();
        assert!(control.resize(800, 450).is_err());
        assert!(matches!(
            requests.try_recv().unwrap(),
            RearmAction::Resize(640, 360)
        ));
        control
            .resize
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .replace_device = true;
        control.replace_device(640, 360).unwrap();
        assert!(control.resize(800, 450).is_err());
        assert!(matches!(
            requests.try_recv().unwrap(),
            RearmAction::ReplaceDevice(640, 360)
        ));
        control.resize.lock().unwrap().take();
        assert!(control.replace_device(640, 360).is_err());
    }
    #[test]
    fn loss_probe_mailbox_is_distinct_bounded_and_never_authorizes_resize() {
        let control = CrossControl::default();
        let (sender, requests) = sync_channel(1);
        *control.resize.lock().unwrap() = Some(ControlRoute {
            sender,
            replace_device: false,
            resize_enabled: false,
            loss_probe: true,
        });
        assert!(control.resize(640, 360).is_err());
        assert!(control.replace_device(640, 360).is_err());
        control.trigger_owned_loss().unwrap();
        assert!(control.trigger_owned_loss().is_err());
        assert!(matches!(
            requests.try_recv().unwrap(),
            RearmAction::TriggerLoss
        ));
    }
}
