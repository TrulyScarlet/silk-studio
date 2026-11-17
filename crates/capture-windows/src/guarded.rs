//! Game-Guarded window-cropped capture using DXGI Desktop Duplication.
//!
//! Explicit opt-in monitor capture cropped to the selected game's client bounds.
//! Blanks frames when the game is not foreground, but overlapping windows and
//! overlays can still appear in the crop. This is not isolated window capture and
//! does not guarantee capture of protected content or anti-cheat compatibility.

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
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::{Common::*, *};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow,
    GA_ROOTOWNER,
};

use crate::dxgi::{backend, next_tick_ns, texture, Acquired, FrameTimer};
use crate::dxgi_cursor::Cursor;
use crate::dxgi_pool::LeasePool;

const POOL_SIZE: usize = 8;
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1 << 62);

pub struct GuardedShared {
    gpu: Arc<GpuDeviceContext>,
    pool: Mutex<Option<LeasePool<GpuTextureSlot>>>,
    active: AtomicBool,
    terminal: Mutex<Option<Result<VideoCaptureEvent>>>,
}

impl GpuFrameResolver for GuardedShared {
    fn resolve(
        &self,
        handle: GpuFrameHandle,
    ) -> std::result::Result<Arc<GpuFrameResource>, GpuFrameResolveError> {
        if !self.active.load(Ordering::Acquire) {
            return Err(GpuFrameResolveError::ContextUnavailable {
                details: "Guarded window capture stopped".into(),
            });
        }
        let guard = self
            .pool
            .lock()
            .map_err(|_| GpuFrameResolveError::ContextUnavailable {
                details: "Guarded window staging pool unavailable".into(),
            })?;
        guard
            .as_ref()
            .and_then(|pool| pool.resolve(handle.0))
            .map(|slot| slot as Arc<GpuFrameResource>)
            .ok_or(GpuFrameResolveError::StaleHandle { handle })
    }

    fn session_resource(&self) -> Option<Arc<GpuFrameResource>> {
        Some(self.gpu.clone())
    }
}

pub struct GuardedWindowCapture {
    source_id: Option<String>,
    stream: StreamId,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    events: Option<Receiver<(VideoCaptureEvent, Arc<GpuTextureSlot>)>>,
    delivered: Option<Arc<GpuTextureSlot>>,
    shared: Option<Arc<GuardedShared>>,
}

impl GuardedWindowCapture {
    pub fn new(stream: StreamId) -> Self {
        Self {
            source_id: None,
            stream,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            events: None,
            delivered: None,
            shared: None,
        }
    }
}

pub(crate) fn is_guarded_source(source: &str) -> Option<&str> {
    source
        .strip_prefix("game-guarded:")
        .or_else(|| source.strip_prefix("guarded:"))
}

impl VideoCapture for GuardedWindowCapture {
    fn active_source_id(&self) -> Option<String> {
        self.worker.as_ref().and(self.source_id.clone())
    }
    fn timestamp_origin(&self) -> Option<Instant> {
        Some(media_clock::capture_epoch())
    }
    fn enumerate_sources(&self) -> Result<Vec<VideoSourceInfo>> {
        let mut sources = Vec::new();
        for window in crate::enumerate_windows()? {
            let Ok(hwnd) = crate::enumerate::resolve_window(&window.id) else {
                continue;
            };
            let mut bounds = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut bounds) }.is_err() {
                continue;
            }
            let width = (bounds.right - bounds.left).max(0) as u32;
            let height = (bounds.bottom - bounds.top).max(0) as u32;
            if width == 0 || height == 0 {
                continue;
            }
            sources.push(VideoSourceInfo {
                id: format!("game-guarded:{}", window.id),
                name: format!("{} (Guarded)", window.title),
                is_primary: false,
                width,
                height,
            });
        }
        Ok(sources)
    }

    fn start(&mut self, config: VideoCaptureConfig) -> Result<()> {
        self.source_id = Some(config.source_id.clone());
        if self.worker.is_some() {
            return Err(backend("guarded capture already started"));
        }
        if !matches!(config.target_fps, 30 | 60 | 120) {
            return Err(VideoCaptureError::InvalidConfiguration {
                reason: "target_fps must be 30, 60, or 120".into(),
            });
        }
        let target_str = is_guarded_source(&config.source_id).unwrap_or(&config.source_id);
        let (expected_pid, expected_creation, _) =
            crate::enumerate::parse_window_source(target_str).ok_or_else(|| {
                VideoCaptureError::InvalidConfiguration {
                    reason: "invalid window source format for guarded capture".into(),
                }
            })?;
        let hwnd = crate::enumerate::resolve_window(target_str)?;
        // Desktop Duplication may omit an excluded game and expose windows
        // behind it even while that game's HWND is foreground. Never treat
        // foreground state as proof that the crop contains game pixels.
        crate::enumerate::check_window_capture_affinity(hwnd)?;
        let hwnd_raw = hwnd.0 as usize;

        let (ready_tx, ready_rx) = sync_channel(1);
        let (event_tx, event_rx) = sync_channel(2);
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&self.stop);
        let stream = self.stream;

        self.worker = Some(
            std::thread::Builder::new()
                .name("silk-guarded-capture".into())
                .spawn(move || {
                    let hwnd = HWND(hwnd_raw as *mut _);
                    let mut session =
                        match GuardedSession::open(hwnd, expected_pid, expected_creation) {
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
                    "guarded",
                    "Game-Guarded Cropped Capture active (DXGI duplication, foreground shield enabled)",
                );
                Ok(())
            }
            result => {
                let error = match result {
                    Ok(Err(error)) => error,
                    _ => backend("guarded capture startup timed out or worker exited"),
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
        match events.recv_timeout(Duration::from_millis(25)) {
            Ok((event, lease)) => {
                self.delivered = Some(lease);
                Ok(event)
            }
            Err(error) => self
                .shared
                .as_ref()
                .and_then(|s| s.terminal.lock().ok()?.take())
                .unwrap_or(match error {
                    std::sync::mpsc::RecvTimeoutError::Timeout => Ok(VideoCaptureEvent::Idle),
                    std::sync::mpsc::RecvTimeoutError::Disconnected => {
                        Err(VideoCaptureError::EndOfStream)
                    }
                }),
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
            if let Ok(mut pool) = shared.pool.lock() {
                if let Some(p) = pool.as_mut() {
                    p.clear();
                }
            }
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

impl Drop for GuardedWindowCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct GuardedSession {
    hwnd: HWND,
    pid: u32,
    creation_time: u64,
    hmon: HMONITOR,
    mon_coords: RECT,
    output: IDXGIOutput1,
    duplication: Option<IDXGIOutputDuplication>,
    shared: Arc<GuardedShared>,
    desktop: ID3D11Texture2D,
    frozen: ID3D11Texture2D,
    cursor: Cursor,
    mon_width: u32,
    mon_height: u32,
    crop_width: u32,
    crop_height: u32,
}

impl GuardedSession {
    fn open(hwnd: HWND, pid: u32, creation_time: u64) -> Result<Self> {
        let (output, desc, hmon, mi) = find_output_for_window(hwnd)?;
        if !matches!(
            desc.Rotation,
            DXGI_MODE_ROTATION_IDENTITY | DXGI_MODE_ROTATION_UNSPECIFIED
        ) {
            return Err(backend("rotated display unsupported in guarded capture"));
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
        let (mon_width, mon_height) = (mode.ModeDesc.Width, mode.ModeDesc.Height);
        let desktop = texture(&gpu, mon_width, mon_height, false)?;

        let (crop_width, crop_height, _, _) =
            compute_crop_rect(hwnd, &mi.rcMonitor, mon_width, mon_height);
        let (crop_width, crop_height) = (crop_width.max(2), crop_height.max(2));
        let frozen = texture(&gpu, crop_width, crop_height, false)?;
        clear_texture_black(&gpu, &frozen)?;

        let mut slots = Vec::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            slots.push(GpuTextureSlot::new(
                texture(&gpu, crop_width, crop_height, false)?,
                Arc::clone(&gpu),
            ));
        }

        let shared = Arc::new(GuardedShared {
            gpu,
            pool: Mutex::new(Some(LeasePool::new(slots))),
            active: AtomicBool::new(true),
            terminal: Mutex::new(None),
        });

        Ok(Self {
            hwnd,
            pid,
            creation_time,
            hmon,
            mon_coords: mi.rcMonitor,
            output,
            duplication: Some(duplication),
            shared,
            desktop,
            frozen,
            cursor: Cursor::default(),
            mon_width,
            mon_height,
            crop_width,
            crop_height,
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
        let mut origin = Some(media_clock::capture_epoch());
        let mut has_desktop = false;
        let mut delivered = 0_u64;
        let mut dropped = 0_u64;
        let mut last_foreground = None;
        let mut recovery = DuplicationRecovery::new();
        let mut has_game_frame = false;

        // Emit initial format
        let _ = events.try_send((
            VideoCaptureEvent::FormatChanged {
                width: self.crop_width,
                height: self.crop_height,
            },
            self.acquire_slot()?,
        ));

        while !stop.load(Ordering::Acquire) {
            let sampled = Instant::now();

            // 1. Check window lifetime
            if !unsafe { IsWindow(Some(self.hwnd)).as_bool() }
                || crate::enumerate::process_creation_time(self.pid).ok()
                    != Some(self.creation_time)
            {
                diagnostics::info("guarded", "target game window was closed or terminated");
                return Err(VideoCaptureError::SourceUnavailable);
            }

            // 2. Capture exclusions remain enforced even while frozen.
            crate::enumerate::check_window_capture_affinity(self.hwnd)?;

            // A focus return must wait for a fresh desktop presentation. Never
            // reuse a cached desktop acquired while another app was foreground.
            let foreground_before_acquire = is_window_foreground(self.hwnd, self.pid);
            if !foreground_before_acquire {
                recovery.waiting_for_presentation = true;
            }
            // 3. Update desktop frame via DXGI
            if self.duplication.is_none() && recovery.retry_due(sampled) {
                self.recreate_duplication()?;
            }
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            let acquired = self.duplication.as_ref().map(|duplication| unsafe {
                duplication.AcquireNextFrame(0, &mut info, &mut resource)
            });
            match acquired {
                Some(Ok(())) => {
                    let duplication = self.duplication.as_ref().unwrap();
                    let _frame = Acquired(duplication);
                    let source: ID3D11Texture2D = resource
                        .ok_or_else(|| backend("empty desktop resource"))?
                        .cast()
                        .map_err(backend)?;
                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                    unsafe {
                        source.GetDesc(&mut desc);
                    }
                    if (desc.Width, desc.Height) != (self.mon_width, self.mon_height) {
                        self.desktop = texture(&self.shared.gpu, desc.Width, desc.Height, false)?;
                        self.mon_width = desc.Width;
                        self.mon_height = desc.Height;
                        recovery.waiting_for_presentation = true;
                    }
                    if info.LastPresentTime != 0 {
                        // Only interpret monitor movement against a healthy
                        // presentation, not transient fullscreen topology.
                        let current_hmon =
                            unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST) };
                        if current_hmon != self.hmon && foreground_before_acquire {
                            return Err(VideoCaptureError::SourceUnavailable);
                        }
                        unsafe {
                            self.shared
                                .gpu
                                .context()
                                .CopyResource(&self.desktop, &source);
                        }
                        has_desktop = true;
                        if foreground_before_acquire {
                            recovery.waiting_for_presentation = false;
                        }
                    }
                    self.cursor.update(duplication, &info)?;
                }
                Some(Err(error)) if temporary_duplication_loss(error.code()) => {
                    // Release the invalid duplication before attempting another.
                    // Snapshot, leases, GPU device, media origin and encoder stay alive.
                    self.duplication = None;
                    recovery.interrupted(sampled);
                    diagnostics::info(
                        "guarded",
                        "desktop duplication interrupted; freezing during same-device recovery",
                    );
                }
                Some(Err(error)) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => {}
                Some(Err(error))
                    if matches!(
                        error.code(),
                        DXGI_ERROR_DEVICE_REMOVED | DXGI_ERROR_DEVICE_RESET
                    ) =>
                {
                    return Err(VideoCaptureError::DeviceLost);
                }
                Some(Err(error)) => return Err(backend(error)),
                None => {}
            }

            // Sample focus before changing geometry. Minimize/Alt-Tab may
            // temporarily change client bounds; blank at the last game format
            // instead of discarding the replay epoch for a taskbar/desktop size.
            let is_foreground =
                foreground_before_acquire && is_window_foreground(self.hwnd, self.pid);
            if !is_foreground {
                recovery.waiting_for_presentation = true;
            }
            if last_foreground != Some(is_foreground) {
                diagnostics::info(
                    "guarded",
                    &format!(
                        "foreground shield: {}",
                        if is_foreground {
                            "game visible"
                        } else {
                            "last game frame frozen"
                        }
                    ),
                );
                last_foreground = Some(is_foreground);
            }
            // 4. Compute window crop rectangle
            let (crop_w, crop_h, local_x, local_y) =
                compute_crop_rect(self.hwnd, &self.mon_coords, self.mon_width, self.mon_height);

            // Handle resolution changes
            if should_resize_crop(
                is_foreground && !recovery.waiting_for_presentation && self.duplication.is_some(),
                (self.crop_width, self.crop_height),
                (crop_w, crop_h),
            ) {
                self.crop_width = crop_w;
                self.crop_height = crop_h;
                self.reallocate_pool(crop_w, crop_h)?;
                self.frozen = texture(&self.shared.gpu, crop_w, crop_h, false)?;
                clear_texture_black(&self.shared.gpu, &self.frozen)?;
                has_game_frame = false;
                let _ = events.try_send((
                    VideoCaptureEvent::FormatChanged {
                        width: crop_w,
                        height: crop_h,
                    },
                    self.acquire_slot()?,
                ));
            }

            // 5. Privacy Guard Check
            let is_foreground = is_foreground && is_window_foreground(self.hwnd, self.pid);

            // Do not publish initial black frames or start the media origin
            // until a focused game image exists. After that, retain/freeze it.
            if !has_game_frame
                && !(is_foreground && !recovery.waiting_for_presentation && has_desktop)
            {
                timer.wait(Duration::from_millis(10))?;
                continue;
            }

            if has_desktop && self.crop_width >= 2 && self.crop_height >= 2 {
                if let Some((index, slot)) = self.acquire_slot_opt() {
                    if is_foreground && !recovery.waiting_for_presentation {
                        // Game is actively focused: crop game pixels
                        // Clear padding and never copy beyond the observed
                        // client when focus-transition borders jitter slightly.
                        clear_texture_black(&self.shared.gpu, slot.texture())?;
                        let right = (local_x + self.crop_width.min(crop_w)).min(self.mon_width);
                        let bottom = (local_y + self.crop_height.min(crop_h)).min(self.mon_height);
                        if right > local_x && bottom > local_y {
                            let box_region = D3D11_BOX {
                                left: local_x,
                                top: local_y,
                                front: 0,
                                right,
                                bottom,
                                back: 1,
                            };
                            unsafe {
                                self.shared.gpu.context().CopySubresourceRegion(
                                    slot.texture(),
                                    0,
                                    0,
                                    0,
                                    0,
                                    &self.desktop,
                                    0,
                                    Some(&box_region),
                                );
                            }
                            self.cursor.draw_with_offset(
                                &self.shared.gpu,
                                slot.texture(),
                                self.crop_width,
                                self.crop_height,
                                local_x as i32,
                                local_y as i32,
                            )?;
                        } else {
                            // Window has zero visible intersection on this display: blank frame
                            clear_texture_black(&self.shared.gpu, slot.texture())?;
                        }
                        if is_window_foreground(self.hwnd, self.pid) {
                            // Dedicated GPU-owned snapshot, never a published
                            // encoder lease. Bounded to one canvas per session.
                            unsafe {
                                self.shared
                                    .gpu
                                    .context()
                                    .CopyResource(&self.frozen, slot.texture());
                            }
                            has_game_frame = right > local_x && bottom > local_y;
                        } else {
                            recovery.waiting_for_presentation = true;
                            unsafe {
                                self.shared
                                    .gpu
                                    .context()
                                    .CopyResource(slot.texture(), &self.frozen);
                            }
                        }
                    } else {
                        // Keep emitting timed GPU frames at the same dimensions
                        // so replay history/audio continuity survives Alt-Tab.
                        // Before any valid game image, the snapshot is black.
                        unsafe {
                            self.shared
                                .gpu
                                .context()
                                .CopyResource(slot.texture(), &self.frozen);
                        }
                    }

                    if !has_game_frame {
                        timer.wait(Duration::from_millis(10))?;
                        continue;
                    }
                    let origin = *origin.get_or_insert(sampled);
                    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
                    if let Ok(guard) = self.shared.pool.lock() {
                        if let Some(pool) = guard.as_ref() {
                            pool.publish(index, handle);
                        }
                    }

                    let frame = VideoFrame {
                        stream_id: stream,
                        width: self.crop_width,
                        height: self.crop_height,
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
            "guarded",
            &format!("guarded capture stopped: delivered={delivered}, dropped={dropped}"),
        );
        Ok(())
    }

    fn acquire_slot(&self) -> Result<Arc<GpuTextureSlot>> {
        self.acquire_slot_opt()
            .map(|(_, slot)| slot)
            .ok_or_else(|| backend("no available texture slots"))
    }

    fn recreate_duplication(&mut self) -> Result<()> {
        // A real device failure cannot safely reuse the frozen texture/encoder.
        unsafe { self.shared.gpu.device().GetDeviceRemovedReason() }
            .map_err(|_| VideoCaptureError::DeviceLost)?;
        let duplication = match unsafe { self.output.DuplicateOutput(self.shared.gpu.device()) } {
            Ok(duplication) => duplication,
            Err(error) if retry_duplication_creation(error.code()) => return Ok(()),
            Err(error)
                if matches!(
                    error.code(),
                    DXGI_ERROR_DEVICE_REMOVED | DXGI_ERROR_DEVICE_RESET
                ) =>
            {
                return Err(VideoCaptureError::DeviceLost);
            }
            Err(error) => return Err(backend(error)),
        };
        let desc = unsafe { self.output.GetDesc() }.map_err(backend)?;
        let mode = unsafe { duplication.GetDesc() };
        if !matches!(
            desc.Rotation,
            DXGI_MODE_ROTATION_IDENTITY | DXGI_MODE_ROTATION_UNSPECIFIED
        ) || mode.ModeDesc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
        {
            return Err(backend(
                "unsupported desktop format/rotation after duplication recovery",
            ));
        }
        self.mon_coords = desc.DesktopCoordinates;
        self.hmon = desc.Monitor;
        self.cursor = Cursor::default();
        self.duplication = Some(duplication);
        diagnostics::info(
            "guarded",
            "desktop duplication restored on existing GPU; awaiting focused presentation",
        );
        Ok(())
    }

    fn acquire_slot_opt(&self) -> Option<(usize, Arc<GpuTextureSlot>)> {
        let guard = self.shared.pool.lock().ok()?;
        guard.as_ref()?.acquire()
    }

    fn reallocate_pool(&self, width: u32, height: u32) -> Result<()> {
        let mut slots = Vec::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            slots.push(GpuTextureSlot::new(
                texture(&self.shared.gpu, width, height, false)?,
                Arc::clone(&self.shared.gpu),
            ));
        }
        if let Ok(mut guard) = self.shared.pool.lock() {
            *guard = Some(LeasePool::new(slots));
        }
        Ok(())
    }
}

fn temporary_duplication_loss(code: windows::core::HRESULT) -> bool {
    matches!(
        code,
        DXGI_ERROR_ACCESS_LOST | DXGI_ERROR_SESSION_DISCONNECTED
    )
}

struct DuplicationRecovery {
    retry_at: Instant,
    waiting_for_presentation: bool,
}

impl DuplicationRecovery {
    fn new() -> Self {
        Self {
            retry_at: Instant::now(),
            waiting_for_presentation: true,
        }
    }

    fn interrupted(&mut self, now: Instant) {
        self.waiting_for_presentation = true;
        self.retry_at = now + Duration::from_millis(250);
    }

    fn retry_due(&mut self, now: Instant) -> bool {
        if now < self.retry_at {
            return false;
        }
        self.retry_at = now + Duration::from_millis(250);
        true
    }
}

fn retry_duplication_creation(code: windows::core::HRESULT) -> bool {
    temporary_duplication_loss(code)
        || matches!(
            code,
            windows::Win32::Foundation::E_ACCESSDENIED
                | DXGI_ERROR_NOT_CURRENTLY_AVAILABLE
                | DXGI_ERROR_UNSUPPORTED
        )
}

fn should_resize_crop(foreground: bool, current: (u32, u32), observed: (u32, u32)) -> bool {
    foreground
        && (current.0.abs_diff(observed.0) > 2 || current.1.abs_diff(observed.1) > 2)
        && observed.0 >= 2
        && observed.1 >= 2
}

pub(crate) fn is_window_foreground(hwnd: HWND, expected_pid: u32) -> bool {
    if hwnd.0.is_null() || expected_pid == 0 {
        return false;
    }
    unsafe {
        if IsIconic(hwnd).as_bool() {
            return false;
        }
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return false;
        }
        if fg == hwnd {
            return true;
        }
        if GetAncestor(fg, GA_ROOTOWNER) == hwnd {
            return true;
        }
        let mut fg_pid = 0;
        GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        if fg_pid != 0 && fg_pid == expected_pid {
            return true;
        }
        false
    }
}

fn clear_texture_black(gpu: &GpuDeviceContext, target: &ID3D11Texture2D) -> Result<()> {
    let mut rtv: Option<ID3D11RenderTargetView> = None;
    unsafe {
        gpu.device()
            .CreateRenderTargetView(target, None, Some(&mut rtv))
            .map_err(backend)?;
        if let Some(rtv) = rtv {
            gpu.context()
                .ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
        }
    }
    Ok(())
}

fn compute_crop_rect(
    hwnd: HWND,
    mon_coords: &RECT,
    mon_width: u32,
    mon_height: u32,
) -> (u32, u32, u32, u32) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }

    let mon_w = (mon_coords.right - mon_coords.left).max(0);
    let mon_h = (mon_coords.bottom - mon_coords.top).max(0);

    let local_left = (rect.left - mon_coords.left).clamp(0, mon_w) as u32;
    let local_top = (rect.top - mon_coords.top).clamp(0, mon_h) as u32;
    let local_right = (rect.right - mon_coords.left).clamp(0, mon_w) as u32;
    let local_bottom = (rect.bottom - mon_coords.top).clamp(0, mon_h) as u32;

    let width = local_right.saturating_sub(local_left).min(mon_width);
    let height = local_bottom.saturating_sub(local_top).min(mon_height);

    // Enforce even dimensions for hardware video encoding (H.264/HEVC)
    (
        (width & !1).max(2),
        (height & !1).max(2),
        local_left,
        local_top,
    )
}

fn find_output_for_window(
    hwnd: HWND,
) -> Result<(IDXGIOutput, DXGI_OUTPUT_DESC, HMONITOR, MONITORINFO)> {
    let hmon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if hmon.0.is_null() {
        return Err(backend("MonitorFromWindow returned null"));
    }
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { !GetMonitorInfoW(hmon, &mut mi).as_bool() } {
        return Err(backend("GetMonitorInfoW failed"));
    }

    let factory = crate::enumerate::factory()?;
    let mut adapter_index = 0_u32;
    loop {
        let adapter: Option<IDXGIAdapter1> = unsafe { factory.EnumAdapters1(adapter_index) }.ok();
        let Some(adapter) = adapter else { break };
        for output in crate::enumerate::adapter_outputs(&adapter)? {
            let Some(desc) = crate::enumerate::describe(&output) else {
                continue;
            };
            let exact = desc.DesktopCoordinates.left == mi.rcMonitor.left
                && desc.DesktopCoordinates.top == mi.rcMonitor.top;
            let bounds_match = mi.rcMonitor.left >= desc.DesktopCoordinates.left
                && mi.rcMonitor.right <= desc.DesktopCoordinates.right
                && mi.rcMonitor.top >= desc.DesktopCoordinates.top
                && mi.rcMonitor.bottom <= desc.DesktopCoordinates.bottom;
            if exact || bounds_match {
                return Ok((output, desc, hmon, mi));
            }
        }
        adapter_index += 1;
    }
    Err(backend("could not find DXGI output for window monitor"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn duplication_interruption_freezes_and_throttles_retries() {
        let now = std::time::Instant::now();
        let mut recovery = super::DuplicationRecovery::new();
        for cycle in 0..3 {
            let start = now + std::time::Duration::from_secs(cycle);
            recovery.waiting_for_presentation = false;
            recovery.interrupted(start);
            assert!(recovery.waiting_for_presentation);
            assert!(!recovery.retry_due(start + std::time::Duration::from_millis(249)));
            assert!(recovery.retry_due(start + std::time::Duration::from_millis(250)));
            assert!(!recovery.retry_due(start + std::time::Duration::from_millis(251)));
            assert!(recovery.retry_due(start + std::time::Duration::from_millis(500)));
            // Recreating duplication alone never permits cached desktop pixels.
            assert!(recovery.waiting_for_presentation);
        }
    }

    #[test]
    fn temporary_duplication_loss_is_not_device_loss() {
        for code in [DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_SESSION_DISCONNECTED] {
            assert!(temporary_duplication_loss(code));
            assert!(retry_duplication_creation(code));
        }
        for code in [
            DXGI_ERROR_DEVICE_REMOVED,
            DXGI_ERROR_DEVICE_RESET,
            DXGI_ERROR_INVALID_CALL,
        ] {
            assert!(!temporary_duplication_loss(code));
            assert!(!retry_duplication_creation(code));
        }
        for code in [
            windows::Win32::Foundation::E_ACCESSDENIED,
            DXGI_ERROR_NOT_CURRENTLY_AVAILABLE,
            DXGI_ERROR_UNSUPPORTED,
        ] {
            assert!(retry_duplication_creation(code));
        }
    }

    #[test]
    #[ignore = "explicit selected-game consent and interactive Desktop Duplication required"]
    fn selected_game_duplication_recreation_keeps_gpu_and_snapshot() {
        assert_eq!(
            std::env::var("SILK_GAME_REPLAY_APPROVAL").as_deref(),
            Ok("valorant-guarded-only")
        );
        let pid = std::env::var("SILK_GAME_PROBE_PID")
            .unwrap()
            .parse()
            .unwrap();
        let creation = std::env::var("SILK_GAME_PROBE_CREATION")
            .unwrap()
            .parse()
            .unwrap();
        let hwnd = HWND(
            std::env::var("SILK_GAME_PROBE_HWND")
                .unwrap()
                .parse::<usize>()
                .unwrap() as *mut _,
        );
        assert_eq!(
            crate::enumerate::process_creation_time(pid).unwrap(),
            creation
        );
        crate::enumerate::check_window_capture_affinity(hwnd).unwrap();
        let mut session = GuardedSession::open(hwnd, pid, creation).unwrap();
        let gpu = session.shared.gpu.device().as_raw();
        let frozen = session.frozen.as_raw();
        let shared = Arc::clone(&session.shared);
        let slot = session.acquire_slot().unwrap();
        for _ in 0..3 {
            session.duplication = None;
            let deadline = Instant::now() + Duration::from_secs(5);
            while session.duplication.is_none() && Instant::now() < deadline {
                session.recreate_duplication().unwrap();
                std::thread::sleep(Duration::from_millis(250));
            }
            assert!(session.duplication.is_some());
            assert_eq!(session.shared.gpu.device().as_raw(), gpu);
            assert_eq!(session.frozen.as_raw(), frozen);
            assert!(Arc::ptr_eq(&shared, &session.shared));
            assert_eq!(slot.device_context().device().as_raw(), gpu);
        }
    }
    #[test]
    fn background_and_minimize_geometry_preserve_game_replay_format() {
        assert!(!super::should_resize_crop(
            false,
            (2560, 1440),
            (2560, 1438)
        ));
        assert!(!super::should_resize_crop(false, (2560, 1440), (146, 20)));
        assert!(!super::should_resize_crop(true, (2560, 1440), (0, 0)));
        assert!(!super::should_resize_crop(true, (2560, 1440), (2560, 1440)));
        assert!(super::should_resize_crop(true, (2560, 1440), (1920, 1080)));
    }
    use super::*;

    #[test]
    fn guarded_source_prefixes_parsed_correctly() {
        assert_eq!(
            is_guarded_source("game-guarded:pid:12:created:34:hwnd:56"),
            Some("pid:12:created:34:hwnd:56")
        );
        assert_eq!(
            is_guarded_source("guarded:pid:12:created:34:hwnd:56"),
            Some("pid:12:created:34:hwnd:56")
        );
        assert_eq!(is_guarded_source("game:pid:12:created:34:hwnd:56"), None);
        assert_eq!(is_guarded_source("auto"), None);
    }

    #[test]
    fn crop_rect_enforces_even_dimensions_and_clamping() {
        let mon = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        // A rect within monitor: width 1001 -> 1000, height 701 -> 700
        let (w, h, x, y) = compute_crop_rect(HWND(std::ptr::null_mut()), &mon, 1920, 1080);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        assert!(x <= 1920);
        assert!(y <= 1080);

        // Window on secondary monitor with non-zero origin
        let secondary_mon = RECT {
            left: 1920,
            top: 0,
            right: 3840,
            bottom: 1080,
        };
        let (w2, h2, x2, y2) =
            compute_crop_rect(HWND(std::ptr::null_mut()), &secondary_mon, 1920, 1080);
        assert_eq!(w2 % 2, 0);
        assert_eq!(h2 % 2, 0);
        assert!(x2 <= 1920);
        assert!(y2 <= 1080);
    }

    #[test]
    fn is_window_foreground_rejects_null_hwnd_and_unrelated_pid() {
        assert!(!is_window_foreground(HWND(std::ptr::null_mut()), 9999999));
        assert!(!is_window_foreground(unsafe { GetForegroundWindow() }, 0));
    }

    #[test]
    fn guarded_start_rejects_invalid_configurations() {
        let mut capture = GuardedWindowCapture::new(StreamId(0));
        assert!(capture
            .start(VideoCaptureConfig {
                source_id: "guarded:invalid".into(),
                target_fps: 60,
            })
            .is_err());

        assert!(capture
            .start(VideoCaptureConfig {
                source_id: "guarded:pid:1:created:2:hwnd:3".into(),
                target_fps: 45, // invalid fps
            })
            .is_err());
    }
}
