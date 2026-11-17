//! Test-only bounded adapter; all HWND/GPU work belongs to its dedicated worker.
use capture_api::{VideoCapture, VideoCaptureConfig, VideoCaptureError, VideoCaptureEvent};
use capture_hook_prototype::d3d12_present_fixture::D3d12PresentFixture;
#[cfg(feature = "d3d12-gpu-fixture")]
use capture_hook_prototype::local_gpu_bridge::GpuPoolBudget;
use capture_hook_prototype::publication_pool::Publication;
use capture_hook_prototype::timeline::QpcTimeline;
use encoder_api::GpuFrameContext;
use gpu_windows::GpuTextureSlot;
#[cfg(feature = "d3d12-gpu-fixture")]
use media_types::GpuFrameHandle;
use media_types::{FramePayload, PixelFormat, StreamId, TimeBase, VideoFrame, VideoSourceInfo};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const SOURCE: &str = "owned-d3d12-present-cpu-readback";
pub const GPU_SOURCE: &str = "owned-d3d12-present-gpu-interop";
struct Delivery {
    frame: VideoFrame,
    publication: Option<Publication<GpuTextureSlot>>,
}
struct Captured {
    width: u32,
    height: u32,
    submitted_at: Instant,
    payload: FramePayload,
    publication: Option<Publication<GpuTextureSlot>>,
}
fn poll_cpu(
    fixture: &mut D3d12PresentFixture,
    readbacks: &AtomicU64,
) -> Result<Option<Captured>, String> {
    let Some(frame) = fixture.poll()? else {
        return Ok(None);
    };
    if !frame
        .bgra
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == [0, 0, 255, 255])
    {
        return Err("swap-chain replay contained unrelated/partial pixels".into());
    }
    readbacks.fetch_add(1, Ordering::Relaxed);
    Ok(Some(Captured {
        width: frame.width,
        height: frame.height,
        submitted_at: frame.submitted_at,
        payload: FramePayload::Cpu(frame.bgra),
        publication: None,
    }))
}
pub struct Capture {
    warp: bool,
    gpu_mode: bool,
    worker: Option<JoinHandle<()>>,
    frames: Option<Receiver<Result<Delivery, String>>>,
    delivered: Option<Publication<GpuTextureSlot>>,
    context: Option<GpuFrameContext>,
    #[cfg(feature = "d3d12-gpu-fixture")]
    budget: Arc<GpuPoolBudget>,
    stop: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    format: bool,
    pub readbacks: Arc<AtomicU64>,
    #[cfg(feature = "d3d12-gpu-fixture")]
    pub gpu_copies: Arc<AtomicU64>,
    selected: Arc<AtomicU64>,
    unrelated: Arc<AtomicU64>,
}
fn backend(details: impl Into<String>) -> VideoCaptureError {
    VideoCaptureError::Backend {
        details: details.into(),
    }
}
impl Capture {
    pub fn new(warp: bool) -> Self {
        Self {
            warp,
            gpu_mode: false,
            worker: None,
            frames: None,
            delivered: None,
            context: None,
            #[cfg(feature = "d3d12-gpu-fixture")]
            budget: Arc::new(GpuPoolBudget::default()),
            stop: Arc::new(AtomicBool::new(false)),
            exited: Arc::new(AtomicBool::new(true)),
            format: false,
            readbacks: Arc::new(AtomicU64::new(0)),
            #[cfg(feature = "d3d12-gpu-fixture")]
            gpu_copies: Arc::new(AtomicU64::new(0)),
            selected: Arc::new(AtomicU64::new(0)),
            unrelated: Arc::new(AtomicU64::new(0)),
        }
    }
    #[cfg(feature = "d3d12-gpu-fixture")]
    pub fn new_gpu(warp: bool) -> Self {
        let mut capture = Self::new(warp);
        capture.gpu_mode = true;
        capture
    }
    fn source(&self) -> &'static str {
        if self.gpu_mode {
            GPU_SOURCE
        } else {
            SOURCE
        }
    }
}
impl VideoCapture for Capture {
    fn enumerate_sources(&self) -> capture_api::Result<Vec<VideoSourceInfo>> {
        Ok(vec![VideoSourceInfo {
            id: self.source().into(),
            name: "Owned D3D12 selected swap-chain fixture".into(),
            is_primary: true,
            width: 318,
            height: 178,
        }])
    }
    fn start(&mut self, config: VideoCaptureConfig) -> capture_api::Result<()> {
        if self.worker.is_some() || config.source_id != self.source() || config.target_fps != 30 {
            return Err(VideoCaptureError::InvalidConfiguration {
                reason: "owned swap-chain test ID and 30 FPS only; no double start".into(),
            });
        }
        let (send, frames) = sync_channel(3);
        let (ready, started) = sync_channel(1);
        let stop = self.stop.clone();
        let exited = self.exited.clone();
        let readbacks = self.readbacks.clone();
        let selected = self.selected.clone();
        let unrelated = self.unrelated.clone();
        let warp = self.warp;
        #[cfg(feature = "d3d12-gpu-fixture")]
        let (gpu_mode, gpu_copies, budget) =
            (self.gpu_mode, self.gpu_copies.clone(), self.budget.clone());
        stop.store(false, Ordering::Release);
        exited.store(false, Ordering::Release);
        self.frames = Some(frames);
        self.worker = Some(std::thread::spawn(move || {
            let result = (|| {
                let origin = Instant::now();
                #[cfg(feature = "d3d12-gpu-fixture")]
                let created = if gpu_mode {
                    D3d12PresentFixture::new_gpu(warp, 318, 178, budget)
                } else {
                    D3d12PresentFixture::new(warp, 318, 178)
                };
                #[cfg(not(feature = "d3d12-gpu-fixture"))]
                let created = D3d12PresentFixture::new(warp, 318, 178);
                let mut fixture = match created {
                    Ok(fixture) => fixture,
                    Err(error) => {
                        let _ = ready.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                #[cfg(feature = "d3d12-gpu-fixture")]
                let context = fixture.gpu_context();
                #[cfg(not(feature = "d3d12-gpu-fixture"))]
                let context: Option<GpuFrameContext> = None;
                ready
                    .send(Ok(context))
                    .map_err(|_| "replay startup receiver closed")?;
                let mut clock = QpcTimeline::new(1_000_000_000).unwrap();
                let deadline = origin + Duration::from_secs(30);
                let mut next = Instant::now();
                while !stop.load(Ordering::Acquire) {
                    if Instant::now() >= deadline {
                        return Err("owned presentation worker deadline".into());
                    }
                    if Instant::now() >= next {
                        fixture.step()?;
                        let stats = fixture.stats();
                        selected.store(stats.selected, Ordering::Release);
                        unrelated.store(stats.unrelated, Ordering::Release);
                        next = Instant::now() + Duration::from_nanos(1_000_000_000 / 30);
                    }
                    #[cfg(feature = "d3d12-gpu-fixture")]
                    let captured = if gpu_mode {
                        fixture.poll_gpu()?.map(|frame| {
                            gpu_copies.fetch_add(1, Ordering::Relaxed);
                            Captured {
                                width: frame.width,
                                height: frame.height,
                                submitted_at: frame.submitted_at,
                                payload: FramePayload::Gpu(GpuFrameHandle(
                                    frame.publication.handle(),
                                )),
                                publication: Some(frame.publication),
                            }
                        })
                    } else {
                        poll_cpu(&mut fixture, &readbacks)?
                    };
                    #[cfg(not(feature = "d3d12-gpu-fixture"))]
                    let captured = poll_cpu(&mut fixture, &readbacks)?;
                    if let Some(frame) = captured {
                        let ticks =
                            i64::try_from(frame.submitted_at.duration_since(origin).as_nanos())
                                .map_err(|_| "fixture timestamp overflow")?;
                        let pts = clock.milliseconds(ticks)?;
                        let frame_publication = frame.publication;
                        let frame = VideoFrame {
                            stream_id: StreamId(0),
                            width: frame.width,
                            height: frame.height,
                            pixel_format: PixelFormat::Bgra8,
                            pts,
                            time_base: TimeBase::MILLISECOND,
                            payload: frame.payload,
                        };
                        match send.try_send(Ok(Delivery {
                            frame,
                            publication: frame_publication,
                        })) {
                            Ok(()) | Err(TrySendError::Full(_)) => {} // drop, never stall rendering
                            Err(TrySendError::Disconnected(_)) => break,
                        }
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                fixture.cancel_capture();
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                let _ = send.try_send(Err(error));
            }
            exited.store(true, Ordering::Release);
        }));
        match started.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(context)) => {
                self.context = context;
                self.format = true;
                Ok(())
            }
            result => {
                let _ = self.stop();
                let details = match result {
                    Ok(Err(error)) => error,
                    Err(error) => error.to_string(),
                    Ok(Ok(_)) => unreachable!(),
                };
                Err(backend(format!(
                    "owned presentation startup failed: {details}"
                )))
            }
        }
    }
    fn next_event(&mut self) -> capture_api::Result<VideoCaptureEvent> {
        self.delivered.take();
        let frames = self.frames.as_ref().ok_or(VideoCaptureError::EndOfStream)?;
        if self.format {
            self.format = false;
            return Ok(VideoCaptureEvent::FormatChanged {
                width: 318,
                height: 178,
            });
        }
        match frames.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(delivery)) => {
                self.delivered = delivery.publication;
                Ok(VideoCaptureEvent::Frame(delivery.frame))
            }
            Ok(Err(error)) => Err(backend(error)),
            Err(error) => Err(backend(format!(
                "owned presentation stream ended/stalled: {error}"
            ))),
        }
    }
    fn stop(&mut self) -> capture_api::Result<()> {
        self.stop.store(true, Ordering::Release);
        self.delivered.take();
        self.context = None;
        self.frames.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| backend("owned presentation worker panicked"))?;
        }
        if !self.exited.load(Ordering::Acquire) {
            return Err(backend("owned presentation worker did not exit"));
        }
        Ok(())
    }
    fn gpu_frame_context(&self) -> Option<GpuFrameContext> {
        self.context.clone()
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[test]
fn rejects_external_sources_without_worker_or_window() {
    let mut capture = Capture::new(true);
    for (source_id, target_fps) in [("auto", 30), ("game:auto", 30), (SOURCE, 144)] {
        assert!(capture
            .start(VideoCaptureConfig {
                source_id: source_id.into(),
                target_fps
            })
            .is_err());
    }
    assert!(capture.worker.is_none());
    assert!(capture.frames.is_none());
    capture.stop().unwrap();
}

#[test]
#[ignore = "opens owned D3D12 WARP windows and withholds recorder consumption"]
fn bounded_presentation_adapter_warp() {
    let mut capture = Capture::new(true);
    capture
        .start(VideoCaptureConfig {
            source_id: SOURCE.into(),
            target_fps: 30,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    // Three unread CPU leases eventually saturate capture, but both chains progress.
    while capture.selected.load(Ordering::Acquire) < 20
        || capture.unrelated.load(Ordering::Acquire) < 20
    {
        assert!(
            Instant::now() < deadline,
            "unread frame queue blocked rendering"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(capture.readbacks.load(Ordering::Acquire), 3);
    assert!(capture
        .start(VideoCaptureConfig {
            source_id: SOURCE.into(),
            target_fps: 30
        })
        .is_err());
    capture.stop().unwrap();
    capture.stop().unwrap();
    assert!(capture.exited.load(Ordering::Acquire));
    assert!(capture.next_event().is_err());
}

#[cfg(feature = "d3d12-gpu-fixture")]
#[test]
fn gpu_adapter_rejects_cpu_source_before_worker_creation() {
    let mut capture = Capture::new_gpu(true);
    assert!(capture
        .start(VideoCaptureConfig {
            source_id: SOURCE.into(),
            target_fps: 30
        })
        .is_err());
    assert!(capture.worker.is_none());
    assert!(capture.gpu_frame_context().is_none());
}

#[cfg(feature = "d3d12-gpu-fixture")]
#[test]
#[ignore = "owned D3D12 WARP GPU adapter with retained encoder leases"]
fn gpu_adapter_retained_leases_warp() {
    let mut capture = Capture::new_gpu(true);
    capture
        .start(VideoCaptureConfig {
            source_id: GPU_SOURCE.into(),
            target_fps: 30,
        })
        .unwrap();
    let context = capture.gpu_frame_context().unwrap();
    assert!(matches!(
        capture.next_event().unwrap(),
        VideoCaptureEvent::FormatChanged { .. }
    ));
    let mut frames = Vec::new();
    let mut leases = Vec::new();
    for _ in 0..3 {
        let VideoCaptureEvent::Frame(frame) = capture.next_event().unwrap() else {
            panic!("missing GPU frame");
        };
        leases.push(context.resolve(&frame).unwrap());
        frames.push(frame);
    }
    assert!(context.resolve(&frames[0]).is_err());
    let deadline = Instant::now() + Duration::from_secs(8);
    while capture.selected.load(Ordering::Acquire) < 20
        || capture.unrelated.load(Ordering::Acquire) < 20
    {
        assert!(
            Instant::now() < deadline,
            "retained encoder surfaces stopped presentation"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(capture.readbacks.load(Ordering::Acquire), 0);
    assert_eq!(capture.gpu_copies.load(Ordering::Acquire), 3);
    capture.stop().unwrap();
    assert!(context.resolve(&frames[2]).is_err());
    assert!(context.session_resource().is_none());
    let pixels = gpu_windows::BgraReadback::default()
        .read_scaled(
            leases[0].downcast_ref::<GpuTextureSlot>().unwrap(),
            318,
            178,
        )
        .unwrap();
    assert!(pixels
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| *pixel == [0, 0, 255, 255]));
    capture.stop().unwrap();
    assert!(capture.exited.load(Ordering::Acquire));
}
