//! Independent renderers with explicit fixture-wide quiescence, never games.
use capture_hook_prototype::admission_gate::AdmissionGate;
use capture_hook_prototype::fixture_probe::FixtureProbe;
use capture_hook_prototype::native::{own_window_identity, CapturedFrame, PresentTap, TapStats};
use capture_hook_prototype::SessionIdentity;
use gpu_windows::BgraReadback;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
    ID3D11Texture2D, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_MODE_DESC, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_ERROR_DEVICE_REMOVED, DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC,
    DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW, RegisterClassW,
    TranslateMessage, MSG, PM_REMOVE, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

fn win_error(error: windows::core::Error) -> String {
    error.to_string()
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(window, message, wparam, lparam)
}

struct Graphics {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    chain: IDXGISwapChain,
}
impl Graphics {
    fn new(window: HWND, warp: bool, width: u32, height: u32) -> Result<Self, String> {
        let desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                ..Default::default()
            },
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            OutputWindow: window,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
            ..Default::default()
        };
        let mut chain = None;
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDeviceAndSwapChain(
                None,
                if warp {
                    D3D_DRIVER_TYPE_WARP
                } else {
                    D3D_DRIVER_TYPE_HARDWARE
                },
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&desc),
                Some(&mut chain),
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(win_error)?;
        Ok(Self {
            device: device.ok_or("null lifecycle device")?,
            context: context.ok_or("null lifecycle context")?,
            chain: chain.ok_or("null lifecycle swap chain")?,
        })
    }
    fn draw(&self, color: [f32; 4]) -> Result<i32, String> {
        unsafe {
            let buffer: ID3D11Texture2D = self.chain.GetBuffer(0).map_err(win_error)?;
            let mut view: Option<ID3D11RenderTargetView> = None;
            self.device
                .CreateRenderTargetView(&buffer, None, Some(&mut view))
                .map_err(win_error)?;
            self.context.ClearRenderTargetView(
                view.as_ref().ok_or("null lifecycle render target")?,
                &color,
            );
            // Drop every backbuffer reference before later ResizeBuffers calls.
            drop(view);
            drop(buffer);
            Ok(self.chain.Present(0, DXGI_PRESENT(0)).0)
        }
    }
}
struct Renderer {
    graphics: Option<Graphics>,
    window: Window,
    color: [f32; 4],
    warp: bool,
}
impl Renderer {
    fn new(target: bool, warp: bool) -> Result<Self, String> {
        let instance = unsafe { GetModuleHandleW(None) }.map_err(win_error)?;
        let class: Vec<u16> = if target {
            "SilkIndependentTarget"
        } else {
            "SilkIndependentOther"
        }
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let class = PCWSTR(class.as_ptr());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&wc) } == 0 {
            return Err("could not register independent renderer window".into());
        }
        let window = Window(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class,
                    class,
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    if target { 40 } else { 760 },
                    60,
                    680,
                    540,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
            }
            .map_err(win_error)?,
        );
        let graphics = Some(Graphics::new(window.0, warp, 640, 480)?);
        Ok(Self {
            graphics,
            window,
            color: if target {
                [1., 0., 0., 1.]
            } else {
                [1., 1., 0., 1.]
            },
            warp,
        })
    }
    fn graphics(&self) -> Result<&Graphics, String> {
        self.graphics
            .as_ref()
            .ok_or("renderer has no active device".into())
    }
    fn pump(&self) {
        let mut message = MSG::default();
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

#[derive(Default)]
struct Progress {
    frames: AtomicU64,
    failures: AtomicU64,
    error: Mutex<Option<String>>,
}
enum Control {
    Attach(u64, Arc<FixtureProbe>),
    Start,
    Pause,
    Resize(u32, u32),
    Recreate,
    Detach,
}
enum Reply {
    Ready(u64),
    Attached(Receiver<CapturedFrame>, Arc<TapStats>),
    Ack,
    Failed(String),
}

/// Closing the fixture gate covers ALL render/Present invocations we own, not
/// just capture work. It does not establish this guarantee for any other process.
struct HookOwner {
    tap: Option<PresentTap>,
    render_gate: Arc<AdmissionGate>,
}
impl HookOwner {
    fn detach(&mut self) -> Result<(), String> {
        if !self.render_gate.is_drained() {
            return Err("cannot remove hook before all owned Present calls are quiescent".into());
        }
        if let Some(tap) = self.tap.as_mut() {
            tap.detach()?;
        }
        self.tap.take();
        Ok(())
    }
}
impl Drop for HookOwner {
    fn drop(&mut self) {
        if self.tap.is_none() {
            return;
        }
        self.render_gate.close();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.render_gate.is_drained() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.render_gate.is_drained() {
            let _ = self.detach();
        } else if let Some(tap) = self.tap.take() {
            std::mem::forget(tap);
        } // retain executable trampoline until process exit
    }
}

struct ThreadOwner {
    commands: SyncSender<Control>,
    replies: Receiver<Reply>,
    worker: Option<JoinHandle<()>>,
    progress: Arc<Progress>,
    stop: Arc<AtomicBool>,
    gate: Arc<AdmissionGate>,
}
impl ThreadOwner {
    fn spawn(target: bool, warp: bool, gate: Arc<AdmissionGate>) -> Result<Self, String> {
        let (commands, receiver) = sync_channel(4);
        let (reply, replies) = sync_channel(8);
        let progress = Arc::new(Progress::default());
        let stop = Arc::new(AtomicBool::new(false));
        let thread_progress = progress.clone();
        let thread_stop = stop.clone();
        let thread_gate = gate.clone();
        let worker = std::thread::Builder::new()
            .name(
                if target {
                    "silk-independent-target"
                } else {
                    "silk-independent-other"
                }
                .into(),
            )
            .spawn(move || {
                let result = render_loop(
                    target,
                    warp,
                    &thread_gate,
                    &thread_stop,
                    &thread_progress,
                    &receiver,
                    &reply,
                );
                if let Err(error) = result {
                    if let Ok(mut stored) = thread_progress.error.lock() {
                        *stored = Some(error.clone());
                    }
                    thread_gate.close();
                    let _ = reply.try_send(Reply::Failed(error));
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            commands,
            replies,
            worker: Some(worker),
            progress,
            stop,
            gate,
        })
    }
    fn send(&self, command: Control) -> Result<(), String> {
        self.commands
            .try_send(command)
            .map_err(|error| error.to_string())?;
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
        Ok(())
    }
    fn receive(&self) -> Result<Reply, String> {
        match self
            .replies
            .recv_timeout(Duration::from_secs(8))
            .map_err(|error| error.to_string())?
        {
            Reply::Failed(error) => Err(error),
            reply => Ok(reply),
        }
    }
    fn ack(&self, command: Control) -> Result<(), String> {
        self.send(command)?;
        if matches!(self.receive()?, Reply::Ack) {
            Ok(())
        } else {
            Err("unexpected renderer reply".into())
        }
    }
    fn attached(
        &self,
        epoch: u64,
        probe: &Arc<FixtureProbe>,
    ) -> Result<(Receiver<CapturedFrame>, Arc<TapStats>), String> {
        self.send(Control::Attach(epoch, probe.clone()))?;
        match self.receive()? {
            Reply::Attached(frames, stats) => Ok((frames, stats)),
            _ => Err("renderer did not attach".into()),
        }
    }
}
impl Drop for ThreadOwner {
    fn drop(&mut self) {
        self.gate.close();
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

fn render_loop(
    target: bool,
    warp: bool,
    gate: &Arc<AdmissionGate>,
    stop: &AtomicBool,
    progress: &Progress,
    commands: &Receiver<Control>,
    replies: &SyncSender<Reply>,
) -> Result<(), String> {
    let mut renderer = Renderer::new(target, warp)?;
    let address = renderer.graphics()?.chain.vtable().Present as usize as u64;
    replies
        .try_send(Reply::Ready(address))
        .map_err(|error| error.to_string())?;
    let mut hook = HookOwner {
        tap: None,
        render_gate: gate.clone(),
    };
    let mut running = false;
    let cadence = Duration::from_nanos(1_000_000_000 / 60);
    let mut next = Instant::now();
    while !stop.load(Ordering::Acquire) {
        renderer.pump();
        match commands.try_recv() {
            Ok(command) => {
                match command {
                    Control::Attach(epoch, probe) => {
                        if !target || running || hook.tap.is_some() || !gate.is_drained() {
                            return Err("attach requires paused/quiescent selected renderer".into());
                        }
                        let session = SessionIdentity {
                            target: own_window_identity(renderer.window.0)?,
                            generation: 1,
                            epoch,
                        };
                        // All owned renderers are explicitly paused and the
                        // shared fixture gate is closed/drained at this point.
                        let (mut tap, frames, stats) =
                            unsafe { PresentTap::attach(&renderer.graphics()?.chain, session) }?;
                        tap.install_fixture_probe(probe)?;
                        hook.tap = Some(tap);
                        replies
                            .try_send(Reply::Attached(frames, stats))
                            .map_err(|error| error.to_string())?;
                        continue;
                    }
                    Control::Start => {
                        running = true;
                        next = Instant::now();
                    }
                    Control::Pause => {
                        running = false;
                    }
                    Control::Resize(width, height) => {
                        if running || !gate.is_drained() {
                            return Err("resize requires paused fixture".into());
                        }
                        unsafe {
                            renderer.graphics()?.chain.ResizeBuffers(
                                2,
                                width,
                                height,
                                DXGI_FORMAT_UNKNOWN,
                                Default::default(),
                            )
                        }
                        .map_err(win_error)?;
                        renderer.color = [0., 1., 0., 1.];
                    }
                    Control::Recreate => {
                        if running || hook.tap.is_some() || !gate.is_drained() {
                            return Err(
                                "device recreation requires detached/quiescent fixture".into()
                            );
                        }
                        renderer.graphics.take();
                        renderer.graphics =
                            Some(Graphics::new(renderer.window.0, renderer.warp, 960, 540)?);
                        renderer.color = [0., 0., 1., 1.];
                    }
                    Control::Detach => {
                        if running {
                            return Err("cannot detach a running renderer".into());
                        }
                        hook.detach()?;
                    }
                }
                replies
                    .try_send(Reply::Ack)
                    .map_err(|error| error.to_string())?;
            }
            Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }
        if running && Instant::now() >= next {
            if let Some(_permit) = gate.try_enter() {
                let result = renderer.graphics()?.draw(renderer.color)?;
                progress.frames.fetch_add(1, Ordering::Release);
                if result < 0 {
                    progress.failures.fetch_add(1, Ordering::Release);
                }
            }
            next = Instant::now() + cadence;
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
    Ok(())
}

fn wait_for(label: &str, mut condition: impl FnMut() -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        if Instant::now() >= deadline {
            return Err(format!("lifecycle deadline: {label}"));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}
fn pause_all(
    target: &ThreadOwner,
    other: &ThreadOwner,
    gate: &AdmissionGate,
) -> Result<(), String> {
    gate.close();
    target.send(Control::Pause)?;
    other.send(Control::Pause)?;
    for owner in [target, other] {
        if !matches!(owner.receive()?, Reply::Ack) {
            return Err("renderer did not acknowledge pause".into());
        }
    }
    if !gate.is_drained() {
        return Err("fixture reported pause while Present was in flight".into());
    }
    Ok(())
}
fn start_all(
    target: &ThreadOwner,
    other: &ThreadOwner,
    gate: &AdmissionGate,
) -> Result<(), String> {
    if !gate.reopen_cooperative_phase() {
        return Err("fixture gate was not drained before resume".into());
    }
    target.ack(Control::Start)?;
    other.ack(Control::Start)?;
    Ok(())
}
fn pixel(frame: &CapturedFrame, expected: [u8; 4]) -> Result<(), String> {
    let bytes = BgraReadback::default()
        .read_scaled(&frame.surface, frame.header.width, frame.header.height)
        .map_err(win_error)?;
    if !bytes.as_chunks::<4>().0.iter().all(|pixel| {
        pixel
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
    }) {
        return Err(format!(
            "independent source pixels did not match {expected:?}"
        ));
    }
    Ok(())
}
fn frame(frames: &Receiver<CapturedFrame>) -> Result<CapturedFrame, String> {
    frames
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())
}

pub fn run(warp: bool) -> Result<(), String> {
    let gate = Arc::new(AdmissionGate::default());
    gate.close();
    let target = ThreadOwner::spawn(true, warp, gate.clone())?;
    let other = ThreadOwner::spawn(false, warp, gate.clone())?;
    let target_address = match target.receive()? {
        Reply::Ready(address) => address,
        _ => return Err("target renderer failed startup".into()),
    };
    let other_address = match other.receive()? {
        Reply::Ready(address) => address,
        _ => return Err("other renderer failed startup".into()),
    };
    if target_address != other_address {
        return Err("cannot establish same-entry multi-thread Present coverage".into());
    }
    let (probe, entered) = FixtureProbe::new();
    let (first_frames, first_stats) = target.attached(1, &probe)?;
    start_all(&target, &other, &gate)?;
    wait_for("independent/full-queue progress", || {
        target.progress.frames.load(Ordering::Acquire) >= 20
            && other.progress.frames.load(Ordering::Acquire) >= 20
            && first_stats.dropped.load(Ordering::Acquire) > 0
    })?;
    let retained_red = frame(&first_frames)?;
    pixel(&retained_red, [0, 0, 255, 255])?;
    pause_all(&target, &other, &gate)?;
    let before_resize = first_stats.published.load(Ordering::Acquire);
    target.ack(Control::Resize(800, 600))?;
    start_all(&target, &other, &gate)?;
    wait_for("resize invalidation", || {
        first_stats.invalidated.load(Ordering::Acquire)
    })?;
    if first_stats.published.load(Ordering::Acquire) != before_resize {
        return Err("incompatible resized frame escaped its old epoch".into());
    }
    pixel(&retained_red, [0, 0, 255, 255])?;
    pause_all(&target, &other, &gate)?;
    drop(first_frames);
    target.ack(Control::Detach)?;
    let (second_frames, second_stats) = target.attached(2, &probe)?;
    start_all(&target, &other, &gate)?;
    let retained_green = frame(&second_frames)?;
    if retained_green.header.session.epoch != 2 {
        return Err("wrong resize epoch".into());
    }
    if (retained_green.header.width, retained_green.header.height) != (800, 600) {
        return Err("resized source dimensions were not captured".into());
    }
    pixel(&retained_green, [0, 255, 0, 255])?;
    probe.arm_checkpoint();
    entered
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    let unrelated_before = other.progress.frames.load(Ordering::Acquire);
    wait_for("unrelated renderer while selected callback is held", || {
        other.progress.frames.load(Ordering::Acquire) >= unrelated_before + 4
    })?;
    if second_stats.capture_gate.in_flight() != 1 || gate.in_flight() == 0 {
        return Err("checkpoint did not hold an actual in-flight callback".into());
    }
    let published = second_stats.published.load(Ordering::Acquire);
    second_stats.cancel_capture();
    if second_stats.capture_gate.is_drained() {
        return Err("cancel falsely reported in-flight capture as drained".into());
    }
    let target_before = target.progress.frames.load(Ordering::Acquire);
    probe.release_checkpoint();
    wait_for("cancelled callback return", || {
        second_stats.capture_gate.is_drained()
            && target.progress.frames.load(Ordering::Acquire) >= target_before + 4
    })?;
    if probe.timed_out.load(Ordering::Acquire)
        || second_stats.published.load(Ordering::Acquire) != published
    {
        return Err(
            "cancellation checkpoint timed out or published after observing cancellation".into(),
        );
    }
    pause_all(&target, &other, &gate)?;
    drop(second_frames);
    target.ack(Control::Detach)?;
    let (third_frames, third_stats) = target.attached(3, &probe)?;
    probe.inject_next_present_result(DXGI_ERROR_DEVICE_REMOVED.0);
    start_all(&target, &other, &gate)?;
    wait_for("injected device-loss notification", || {
        third_stats
            .reported_present_failures
            .load(Ordering::Acquire)
            == 1
            && third_stats.invalidated.load(Ordering::Acquire)
    })?;
    let published = third_stats.published.load(Ordering::Acquire);
    let target_before = target.progress.frames.load(Ordering::Acquire);
    wait_for("forwarding after injected loss", || {
        target.progress.frames.load(Ordering::Acquire) >= target_before + 4
    })?;
    if third_stats
        .original_present_failures
        .load(Ordering::Acquire)
        != 0
        || third_stats.published.load(Ordering::Acquire) != published
    {
        return Err("injected loss did not close capture independently of original Present".into());
    }
    pause_all(&target, &other, &gate)?;
    drop(third_frames);
    target.ack(Control::Detach)?;
    target.ack(Control::Recreate)?;
    let (last_frames, last_stats) = target.attached(4, &probe)?;
    start_all(&target, &other, &gate)?;
    let blue = frame(&last_frames)?;
    if blue.header.session.epoch != 4 || (blue.header.width, blue.header.height) != (960, 540) {
        return Err("new device/epoch did not report its source dimensions".into());
    }
    pixel(&blue, [255, 0, 0, 255])?;
    pixel(&retained_red, [0, 0, 255, 255])?;
    pixel(&retained_green, [0, 255, 0, 255])?;
    pause_all(&target, &other, &gate)?;
    drop(last_frames);
    target.ack(Control::Detach)?;
    if last_stats.original_present_failures.load(Ordering::Acquire) != 0 {
        return Err("real Present failed during new-device capture".into());
    }
    for owner in [&target, &other] {
        if let Some(error) = owner
            .progress
            .error
            .lock()
            .map_err(|_| "renderer error lock poisoned")?
            .as_ref()
        {
            return Err(error.clone());
        }
    }
    println!("PASS: independent target/unrelated renderer threads; same Present entry; full-queue progress; resize/new epoch; in-flight capture cancellation; injected loss notification; own-device recreation; retained pixels; quiescent hook removal. driver={}",if warp{"WARP"}else{"hardware"});
    Ok(())
}
