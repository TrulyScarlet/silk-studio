//! In-process independent owned capture worker. No remote loading or game route.
use crate::d3d12_cross_process::{read_wire, write_wire};
use crate::d3d12_factory_hook::{claim_window, OwnedFactoryHook};
use crate::d3d12_gpu_bridge::D3d12GpuBridge;
use crate::d3d12_owned_hook::OwnedPresentHook;
use crate::d3d12_present_fixture::D3d12PresentFixture;
use crate::d3d12_protocol::{WindowAction, Wire};
use crate::d3d12_queue_registry::{Association, QueueRegistry};
use crate::local_gpu_bridge::GpuPoolBudget;
use crate::transport::Message;
use crate::{SessionIdentity, TargetIdentity};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{IUnknown, Interface};
use windows::Win32::Foundation::{FILETIME, HWND};
use windows::Win32::Graphics::Direct3D12::{ID3D12Device, D3D12_COMMAND_LIST_TYPE_DIRECT};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory4};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};

struct Shared {
    stop: AtomicBool,
    done: AtomicBool,
}
struct Action {
    action: ActionKind,
    answer: SyncSender<Result<(u64, i64), String>>,
}
enum ActionKind {
    Window(WindowAction),
    #[cfg(feature = "d3d12-rearm-fixture")]
    Resize(u32, u32),
    #[cfg(feature = "d3d12-device-replace-fixture")]
    ReplaceDevice(u32, u32),
    #[cfg(feature = "d3d12-device-replace-fixture")]
    PauseForPendingLoss,
    #[cfg(feature = "d3d12-device-replace-fixture")]
    ResumeAfterPendingLossCancel,
}
struct Startup {
    hello: crate::d3d12_protocol::GpuHello,
    prior_presents: u64,
    lifetime: Duration,
    #[cfg(feature = "d3d12-rearm-fixture")]
    resize: Option<ResizeResources>,
}
#[cfg(feature = "d3d12-rearm-fixture")]
struct ResizeResources {
    device: ID3D12Device,
    queue: windows::Win32::Graphics::Direct3D12::ID3D12CommandQueue,
    chain: windows::Win32::Graphics::Dxgi::IDXGISwapChain3,
    warp: bool,
    budget: Arc<GpuPoolBudget>,
}
fn identity(object: &impl Interface) -> Result<u64, String> {
    let canonical: IUnknown = object.cast().map_err(|e| e.to_string())?;
    Ok(canonical.as_raw() as usize as u64)
}
fn fixture_lifetime(value: Option<&str>) -> Result<Duration, String> {
    let Some(value) = value else {
        return Ok(Duration::from_secs(175));
    };
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid owned fixture lifetime".into());
    }
    let seconds = value
        .parse::<u64>()
        .map_err(|_| "invalid owned fixture lifetime")?;
    if !(30..=28_830).contains(&seconds) {
        return Err("owned fixture lifetime must be bounded to 30..28830 seconds".into());
    }
    Ok(Duration::from_secs(seconds))
}
fn validate(target: TargetIdentity) -> Result<(), String> {
    let hwnd = HWND(target.window as usize as *mut _);
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if !target.is_valid()
        || target.pid != std::process::id()
        || pid != target.pid
        || !unsafe { IsWindow(Some(hwnd)) }.as_bool()
    {
        return Err("bootstrap selected target no longer owned/live".into());
    }
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    }
    .map_err(|e| e.to_string())?;
    if target.creation_time
        != (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)
    {
        return Err("bootstrap selected process lifetime changed".into());
    }
    Ok(())
}

/// The renderer owns HWNDs and continues Present independently. The worker gets
/// only PID/lifetime/HWND plus bounded window-action mailboxes: never COM objects.
pub fn run_owned_producer(warp: bool, fault: Option<&str>) -> Result<(), String> {
    OwnedFactoryHook::validate_opt_in()?;
    if warp && fault == Some("replace-remove-pending") {
        return Err("controlled remote pending-copy removal on WARP is unsupported after observed native runtime faults; no removal attempted".into());
    }
    if !std::env::args().any(|arg| arg == "--bootstrap") {
        return Err("explicit owned bootstrap opt-in required".into());
    }
    let mut lifetime_args = std::env::args().filter_map(|arg| {
        arg.strip_prefix("--fixture-lifetime-seconds=")
            .map(str::to_owned)
    });
    let lifetime = fixture_lifetime(lifetime_args.next().as_deref())?;
    if lifetime_args.next().is_some() {
        return Err("duplicate owned fixture lifetime".into());
    }
    if std::env::args().any(|arg| arg == "--replace-device")
        && (!cfg!(feature = "d3d12-device-replace-fixture")
            || !std::env::args().any(|arg| arg == "--rearm"))
    {
        return Err("explicit enabled replacement/rearm opt-in required".into());
    }
    #[cfg(not(feature = "d3d12-rearm-fixture"))]
    if std::env::args().any(|arg| arg == "--rearm") {
        return Err("owned live resize feature not enabled".into());
    }
    if fault.is_some_and(|fault| {
        !matches!(
            fault,
            "exit-after-six"
                | "bad-frame"
                | "bootstrap-stale-target"
                | "replace-after-release"
                | "replace-before-hello"
                | "replace-bad-hello"
                | "replace-remove-quiescent"
                | "replace-remove-pending"
        )
    }) {
        return Err("unsupported bootstrap fixture fault".into());
    }
    if fault.is_some_and(|f| f.starts_with("replace-"))
        && !std::env::args().any(|arg| arg == "--replace-device")
    {
        return Err("replacement fault requires explicit replacement opt-in".into());
    }
    let fault = fault.map(str::to_owned);
    let fail_after_release = fault.as_deref() == Some("replace-after-release");
    let remove_quiescent = fault.as_deref() == Some("replace-remove-quiescent");
    let remove_pending = fault.as_deref() == Some("replace-remove-pending");
    let stale_target = fault.as_deref() == Some("bootstrap-stale-target");
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        done: AtomicBool::new(false),
    });
    let (ready_tx, ready_rx) = sync_channel(1);
    let (target_tx, target_rx) = sync_channel(1);
    let (action_tx, action_rx) = sync_channel::<Action>(2);
    let worker_shared = shared.clone();
    let worker = std::thread::Builder::new()
        .name("silk-owned-d3d12-bootstrap".into())
        .spawn(move || {
            let result = capture_worker(
                warp,
                fault.as_deref(),
                &worker_shared,
                &|| ready_tx.try_send(Ok(())).map_err(|e| e.to_string()),
                &target_rx,
                &action_tx,
                lifetime,
            );
            if let Err(error) = &result {
                let _ = ready_tx.try_send(Err(error.clone()));
            }
            worker_shared.done.store(true, Ordering::Release);
            result
        })
        .map_err(|e| e.to_string())?;
    // Startup is bounded; every error path signals/joins the worker before the
    // renderer's window guards can be dropped.
    let mut renderer = None;
    #[cfg(feature = "d3d12-device-replace-fixture")]
    let mut retained_windows = None;
    #[cfg(feature = "d3d12-device-replace-fixture")]
    let mut paused = false;
    #[cfg(not(feature = "d3d12-device-replace-fixture"))]
    let _ = (fail_after_release, remove_quiescent, remove_pending);
    let result = (|| {
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| e.to_string())??;
        renderer = Some(D3d12PresentFixture::render_only(warp, 318, 178)?);
        renderer.as_mut().unwrap().cover_selected()?;
        let mut target = renderer.as_ref().unwrap().selected_target()?;
        if stale_target {
            target.creation_time = target
                .creation_time
                .checked_add(1)
                .ok_or("fixture timestamp overflow")?;
        }
        target_tx.try_send(target).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + lifetime + Duration::from_secs(5);
        while !shared.done.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                return Err("owned bootstrap renderer deadline".into());
            }
            match action_rx.try_recv() {
                Ok(request) => {
                    let action = match request.action {
                        ActionKind::Window(action) => {
                            renderer.as_mut().unwrap().window_action(action)
                        }
                        #[cfg(feature = "d3d12-rearm-fixture")]
                        ActionKind::Resize(width, height) => renderer
                            .as_mut()
                            .unwrap()
                            .resize_render_only(width, height)
                            .map(|()| 0),
                        #[cfg(feature = "d3d12-device-replace-fixture")]
                        ActionKind::PauseForPendingLoss => {
                            paused = true;
                            Ok(0)
                        }
                        #[cfg(feature = "d3d12-device-replace-fixture")]
                        ActionKind::ResumeAfterPendingLossCancel => {
                            if paused {
                                paused = false;
                                Ok(0)
                            } else {
                                Err("pending-loss cancellation requires paused renderer".into())
                            }
                        }
                        #[cfg(feature = "d3d12-device-replace-fixture")]
                        ActionKind::ReplaceDevice(width, height) => (|| {
                            let old = renderer.as_ref().ok_or("renderer missing")?;
                            let windows = if remove_pending {
                                old.removed_replacement_windows()?
                            } else {
                                old.replacement_windows()?
                            };
                            let removal_device = if remove_quiescent {
                                Some(
                                    renderer
                                        .as_ref()
                                        .ok_or("renderer missing")?
                                        .quiescent_removal_device()?,
                                )
                            } else {
                                None
                            };
                            retained_windows = Some(windows.clone());
                            drop(renderer.take());
                            if let Some(device) = removal_device {
                                // All renderer/worker submissions have already drained
                                // and their graphs have been dropped. This tests the
                                // documented device-removal API, not a driver reset or
                                // removal while GPU work is in flight.
                                unsafe { device.GetDeviceRemovedReason() }
                                    .map_err(|e| e.to_string())?;
                                unsafe { device.RemoveDevice() };
                                let reason = unsafe { device.GetDeviceRemovedReason() }
                                    .err()
                                    .ok_or("RemoveDevice did not mark the owned device removed")?;
                                eprintln!("owned quiescent ID3D12Device5::RemoveDevice observed: {}; releasing final old device before creation",reason);
                                // device drops here, before D3D12CreateDevice.
                            }
                            if fail_after_release {
                                return Err(
                                    "intentional replacement failure after renderer release".into(),
                                );
                            }
                            renderer = Some(D3d12PresentFixture::replace_render_only(
                                warp, width, height, windows,
                            )?);
                            paused = false;
                            Ok(0)
                        })(),
                    };
                    let reply = action.and_then(|flags| {
                        let mut qpc = 0;
                        unsafe {
                            windows::Win32::System::Performance::QueryPerformanceCounter(&mut qpc)
                        }
                        .map_err(|e| e.to_string())?;
                        Ok((flags, qpc))
                    });
                    let failed = reply.is_err();
                    let _ = request.answer.try_send(reply);
                    if failed {
                        return Err("owned renderer action failed".into());
                    }
                }
                Err(TryRecvError::Disconnected | TryRecvError::Empty) => {}
            }
            #[cfg(feature = "d3d12-device-replace-fixture")]
            if paused {
                renderer
                    .as_ref()
                    .ok_or("paused renderer missing")?
                    .pump_paused_windows()?;
            } else {
                renderer
                    .as_mut()
                    .ok_or("renderer replacement failed")?
                    .step()?;
            }
            #[cfg(not(feature = "d3d12-device-replace-fixture"))]
            renderer
                .as_mut()
                .ok_or("renderer replacement failed")?
                .step()?;
            std::thread::sleep(Duration::from_millis(8));
        }
        // Renderer must remain alive until the capture worker has retired hooks,
        // not merely until its final parent-facing message was enqueued.
        Ok::<(), String>(())
    })();
    shared.stop.store(true, Ordering::Release);
    drop(target_tx);
    let worker_result = worker
        .join()
        .map_err(|_| "owned bootstrap worker panicked")?;
    drop(renderer);
    #[cfg(feature = "d3d12-device-replace-fixture")]
    drop(retained_windows);
    result?;
    worker_result
}

fn capture_worker(
    warp: bool,
    fault: Option<&str>,
    shared: &Shared,
    ready: &dyn Fn() -> Result<(), String>,
    target_rx: &Receiver<TargetIdentity>,
    actions: &SyncSender<Action>,
    lifetime: Duration,
) -> Result<(), String> {
    // Independently construct the bootstrap factory; no renderer factory pointer.
    let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1() }.map_err(|e| e.to_string())?;
    #[cfg(feature = "d3d12-late-resize-fixture")]
    let (factory_hook, late_hook, prepared_target) =
        if std::env::args().any(|arg| arg == "--late-d3d12") {
            let target = target_rx
                .recv_timeout(Duration::from_secs(10))
                .map_err(|e| e.to_string())?;
            validate(target)?;
            // This private probe supplies a method entry only, never target media or
            // the target queue. Dispose it before waiting on the real target resize.
            let probe = D3d12PresentFixture::late_discovery_probe(warp)?;
            let late = unsafe {
                crate::d3d12_late_resize::OwnedLateResizeHook::install(
                    &probe.independent_probe_chain(),
                    target,
                )
            }?;
            drop(probe);
            (
                unsafe { OwnedFactoryHook::install(&factory) }?,
                Some(late),
                Some(target),
            )
        } else {
            (unsafe { OwnedFactoryHook::install(&factory) }?, None, None)
        };
    #[cfg(not(feature = "d3d12-late-resize-fixture"))]
    let factory_hook = unsafe { OwnedFactoryHook::install(&factory) }?;
    #[cfg(not(feature = "d3d12-late-resize-fixture"))]
    let prepared_target = None::<TargetIdentity>;
    ready()?;
    let target = match prepared_target {
        Some(target) => target,
        None => target_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| e.to_string())?,
    };
    validate(target)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (chain, queue, window) = loop {
        if shared.stop.load(Ordering::Acquire) {
            return Err("bootstrap canceled before observation".into());
        }
        #[cfg(feature = "d3d12-late-resize-fixture")]
        let observed = if let Some(late) = late_hook.as_ref() {
            late.claim()?
                .map(|(chain, queue)| (chain, queue, target.window as usize))
        } else {
            claim_window(target.window)?.map(|observed| observed.into_parts())
        };
        #[cfg(not(feature = "d3d12-late-resize-fixture"))]
        let observed = claim_window(target.window)?.map(|observed| observed.into_parts());
        if let Some(observed) = observed {
            break observed;
        }
        if Instant::now() >= deadline {
            #[cfg(feature = "d3d12-late-resize-fixture")]
            if late_hook.is_some() {
                return Err("late discovery deadline: no supported successful ResizeBuffers1; queue inference is disabled".into());
            }
            return Err("no observed creation for selected HWND".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    if window as u64 != target.window
        || unsafe { chain.GetHwnd() }.map_err(|e| e.to_string())?.0 as usize as u64 != target.window
        || unsafe { queue.GetDesc() }.Type != D3D12_COMMAND_LIST_TYPE_DIRECT
    {
        return Err("bootstrap association window/queue mismatch".into());
    }
    let mut device: Option<ID3D12Device> = None;
    unsafe { queue.GetDevice(&mut device) }.map_err(|e| e.to_string())?;
    let device = device.ok_or("bootstrap queue returned null device")?;
    let chain_device: ID3D12Device = unsafe { chain.GetDevice() }.map_err(|e| e.to_string())?;
    if identity(&device)? != identity(&chain_device)? {
        return Err("bootstrap queue/chain device mismatch".into());
    }
    let mut registry = QueueRegistry::default();
    let token = registry
        .observe(Association {
            chain: identity(&chain)?,
            queue: identity(&queue)?,
            device: identity(&device)?,
            window: target.window,
        })
        .map_err(|e| format!("bootstrap association rejected: {e:?}"))?;
    registry
        .select(token)
        .map_err(|e| format!("bootstrap selection rejected: {e:?}"))?;
    let desc = unsafe { chain.GetDesc1() }.map_err(|e| e.to_string())?;
    if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM || desc.SampleDesc.Count != 1 {
        return Err("unsupported bootstrap source format".into());
    }
    // Establish renderer progress through this module's actual forwarding hook,
    // not executable-private counters that would be separate globals in a DLL.
    let hook = unsafe { OwnedPresentHook::install_forwarding(&chain) }?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while hook.counts().0 < 12 {
        if shared.stop.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err("renderer did not progress before capture bootstrap".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if hook.failed() || hook.counts().2 != 0 {
        return Err("forwarding-only bootstrap warmup failed or captured media".into());
    }
    let prior_presents = hook.counts().0;
    let budget = Arc::new(GpuPoolBudget::default());
    let mut gpu = D3d12GpuBridge::new(
        &device,
        &queue,
        desc.Width,
        desc.Height,
        budget.clone(),
        warp,
    )?;
    let hello = gpu.export_hello(
        &device,
        SessionIdentity {
            target,
            generation: 1,
            epoch: 1,
        },
    )?;
    #[cfg(feature = "d3d12-late-resize-fixture")]
    if crate::d3d12_late_resize::OwnedLateResizeHook::failed() {
        return Err("late observation failed before initial media activation".into());
    }
    hook.arm_initial(&chain, gpu)?;
    #[cfg(feature = "d3d12-rearm-fixture")]
    let resize = std::env::args()
        .any(|arg| {
            arg == "--rearm" || arg == "--auto-resize-owned" || arg == "--observe-owned-loss"
        })
        .then(|| ResizeResources {
            device: device.clone(),
            queue: queue.clone(),
            chain: chain.clone(),
            warp,
            budget,
        });
    // Do not keep hidden old-device/chain references across the transport loop.
    drop(chain_device);
    drop(chain);
    drop(queue);
    drop(device);
    let result = transport_loop(
        shared,
        target,
        fault,
        actions,
        &factory_hook,
        &hook,
        Startup {
            hello,
            prior_presents,
            lifetime,
            #[cfg(feature = "d3d12-rearm-fixture")]
            resize,
        },
    );
    registry.close();
    hook.cancel();
    factory_hook.close();
    // The hook's own bridge teardown drains its GPU submissions; forwarding stays
    // executable until process exit. Renderer still owns its own resources.
    drop(hook);
    result
}

/// Exact passive fixture DLL entry, called only by an explicit remote worker.
/// The owner consumes one startup approval before creating any chain; only then
/// can this worker claim that chain and start reading media approval packets.
#[cfg(feature = "d3d12-remote-helper-fixture")]
pub fn validate_owned_attached_process() -> Result<(), String> {
    OwnedFactoryHook::validate_opt_in()?;
    let late = std::env::args().any(|arg| arg == "--late-d3d12");
    let automatic = std::env::args().any(|arg| arg == "--auto-resize-owned");
    if std::env::args().any(|arg| arg == "--observe-owned-loss")
        && (!cfg!(feature = "d3d12-observed-loss-fixture")
            || late
            || automatic
            || std::env::args().any(|arg| matches!(arg.as_str(), "--warp" | "--legacy-factory")))
    {
        return Err("observed helper loss requires enabled early modern hardware route".into());
    }
    if automatic && (!late || !cfg!(feature = "d3d12-auto-resize-fixture")) {
        return Err("automatic resize requires enabled exact owned late route".into());
    }
    let late_faults = std::env::args()
        .filter(|arg| matches!(arg.as_str(), "--late-no-resize" | "--late-distinct-queues"))
        .count();
    if late_faults > 1 || (late_faults != 0 && (!late || automatic)) {
        return Err("late fixture scenario requires exactly one explicit late route".into());
    }
    if std::env::args().any(|arg| arg == "--late-d3d12")
        && !cfg!(feature = "d3d12-late-resize-fixture")
    {
        return Err("late resize observation feature not enabled".into());
    }
    if !std::env::args().any(|arg| arg == "--passive-d3d12")
        || !std::env::args().any(|arg| arg == "--allow-external-helper")
    {
        return Err("D3D12 worker requires exact passive fixture opt-in".into());
    }
    if std::env::args().any(|arg| {
        matches!(arg.as_str(), "--bootstrap" | "--rearm" | "--replace-device")
            || arg.starts_with("--fault=")
            || arg.starts_with("--fixture-lifetime-seconds=")
    }) {
        return Err(
            "D3D12 helper does not support integrated/rearm/fault/extended lifetime controls"
                .into(),
        );
    }
    Ok(())
}
#[cfg(feature = "d3d12-remote-helper-fixture")]
pub fn run_owned_attached_worker(session: SessionIdentity) -> Result<(), String> {
    validate_owned_attached_process()?;
    if session.generation != 1 || session.epoch != 1 {
        return Err("D3D12 helper requires initial session".into());
    }
    validate(session.target)?;
    let shared = Shared {
        stop: AtomicBool::new(false),
        done: AtomicBool::new(false),
    };
    let (target_tx, target_rx) = sync_channel(1);
    target_tx
        .try_send(session.target)
        .map_err(|e| e.to_string())?;
    drop(target_tx);
    let (actions, unsupported_actions) = sync_channel(1);
    drop(unsupported_actions);
    capture_worker(
        std::env::args().any(|arg| arg == "--warp"),
        None,
        &shared,
        &|| {
            write_wire(
                &mut std::io::stdout(),
                Wire::Control(Message::Report {
                    presented: 0,
                    published: 0,
                    dropped: 0,
                }),
            )
        },
        &target_rx,
        &actions,
        Duration::from_secs(175),
    )
}

fn stop_forwarding(
    shared: &Shared,
    factory: &OwnedFactoryHook,
    hook: &OwnedPresentHook,
    out: &SyncSender<Wire>,
    published: u64,
) -> Result<(), String> {
    hook.cancel();
    factory.close();
    let after = hook
        .counts()
        .0
        .checked_add(12)
        .ok_or("forwarding progress counter exhausted")?;
    let until = Instant::now() + Duration::from_secs(3);
    while hook.counts().0 < after {
        if shared.stop.load(Ordering::Acquire) || Instant::now() >= until {
            return Err("bootstrap renderer stopped after capture cancellation".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    out.try_send(Wire::Control(Message::Report {
        presented: hook.counts().0,
        published,
        dropped: hook.counts().3,
    }))
    .map_err(|e| e.to_string())?;
    out.try_send(Wire::Control(Message::Bye))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn transport_loop(
    shared: &Shared,
    target: TargetIdentity,
    fault: Option<&str>,
    actions: &SyncSender<Action>,
    factory: &OwnedFactoryHook,
    hook: &OwnedPresentHook,
    startup: Startup,
) -> Result<(), String> {
    #[cfg(feature = "d3d12-device-replace-fixture")]
    let mut startup = startup;
    #[cfg(feature = "d3d12-rearm-fixture")]
    let mut hello = startup.hello;
    #[cfg(not(feature = "d3d12-rearm-fixture"))]
    let hello = startup.hello;
    let mut pending = [None; 3];
    let (out, output_rx) = sync_channel(8);
    let writer = std::thread::spawn(move || {
        let mut stdout = std::io::stdout().lock();
        for wire in output_rx {
            write_wire(&mut stdout, wire)?;
        }
        Ok::<_, String>(())
    });
    let (input_tx, input) = sync_channel(4);
    let _reader = std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        loop {
            let message = read_wire(&mut stdin);
            let failed = message.is_err();
            if input_tx.send(message).is_err() || failed {
                break;
            }
        }
    });
    let emit = |wire| {
        out.try_send(wire)
            .map_err(|e| format!("bounded bootstrap output unavailable: {e}"))
    };
    emit(Wire::Hello(hello))?;
    match input
        .recv_timeout(Duration::from_secs(10))
        .map_err(|e| e.to_string())??
    {
        Wire::Control(Message::Select(session)) if session == hello.session => {}
        _ => return Err("bootstrap source approval mismatch".into()),
    }
    let deadline = Instant::now() + startup.lifetime;
    let mut published = 0;
    #[cfg(feature = "d3d12-auto-resize-fixture")]
    let automatic = std::env::args().any(|arg| arg == "--auto-resize-owned");
    let result = (|| loop {
        if shared.stop.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err("owned bootstrap capture canceled/deadline".into());
        }
        validate(target)?;
        #[cfg(feature = "d3d12-observed-loss-fixture")]
        if std::env::args().any(|arg| arg == "--observe-owned-loss") {
            let resources = startup
                .resize
                .as_ref()
                .ok_or("observed loss resources unavailable")?;
            if let Err(removed) = unsafe { resources.device.GetDeviceRemovedReason() } {
                let reason =
                    crate::d3d12_protocol::DeviceLossReason::from_hresult(removed.code().0 as u32)
                        .ok_or("unsupported observed device removal reason")?;
                hook.pause_for_pending_loss();
                let mut loss = crate::device_loss_epoch::DeviceLossEpoch::new(hello)
                    .map_err(|error| format!("observed loss initialization: {error:?}"))?;
                let notice = crate::d3d12_protocol::DeviceLossNotice {
                    session: hello.session,
                    reason,
                };
                let next = loss
                    .begin(notice)
                    .map_err(|error| format!("observed loss begin: {error:?}"))?;
                emit(Wire::DeviceLost(notice))?;
                let until = Instant::now() + Duration::from_secs(5);
                loop {
                    if shared.stop.load(Ordering::Acquire) || Instant::now() >= until {
                        return Err("observed loss retirement canceled/deadline".into());
                    }
                    match input.recv_timeout(Duration::from_millis(25)) {
                        Ok(Ok(Wire::Retired(session))) if session == next => break,
                        Ok(Ok(Wire::Control(Message::Ack {
                            session,
                            slot,
                            sequence,
                        }))) => acknowledge_pending(
                            &mut pending,
                            hello.session,
                            session,
                            slot,
                            sequence,
                        )?,
                        Ok(Ok(Wire::Control(Message::Stop))) => {
                            emit(Wire::Control(Message::Bye))?;
                            return Ok(());
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        other => {
                            return Err(format!(
                                "unexpected observed loss retirement packet: {other:?}"
                            ))
                        }
                    }
                }
                loss.retired(next)
                    .map_err(|error| format!("observed loss retirement: {error:?}"))?;
                hook.access(|gpu| gpu.approve_remote_retirement(hello.session))?;
                hook.release_media_for_resize()?;
                startup.resize.take();
                crate::d3d12_factory_hook::discard_pending_for_replacement()?;
                factory.close();
                validate(target)?;
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(windows::Win32::Foundation::HWND(
                            target.window as usize as *mut _,
                        )),
                        crate::d3d12_present_fixture::OWNED_HELPER_RELEASED_MESSAGE,
                        windows::Win32::Foundation::WPARAM(next.epoch as usize),
                        windows::Win32::Foundation::LPARAM(0),
                    )
                }
                .map_err(|error| error.to_string())?;
                emit(Wire::Control(Message::Bye))?;
                return Ok(());
            }
        }
        if hook.failed() || factory.failed() {
            return Err("owned bootstrap observation/Present failure".into());
        }
        #[cfg(feature = "d3d12-late-resize-fixture")]
        if crate::d3d12_late_resize::OwnedLateResizeHook::failed() {
            return Err("late/automatic resize observation became terminal".into());
        }
        #[cfg(feature = "d3d12-auto-resize-fixture")]
        if automatic {
            if let Some(offer) =
                crate::d3d12_late_resize::OwnedLateResizeHook::take_automatic_offer()?
            {
                let resources = startup
                    .resize
                    .as_ref()
                    .ok_or("automatic resize resources unavailable")?;
                if identity(&offer.chain)? != identity(&resources.chain)?
                    || identity(&offer.queue)? != identity(&resources.queue)?
                {
                    return Err("automatic resize changed chain/queue identity".into());
                }
                let request = crate::d3d12_protocol::ResizeRequest {
                    session: hello.session,
                    width: offer.width,
                    height: offer.height,
                };
                let mut epoch = crate::resize_epoch::ResizeEpoch::new(hello)?;
                let next = epoch.begin(request)?;
                if next.epoch != offer.epoch {
                    return Err("automatic observed epoch mismatch".into());
                }
                emit(Wire::Resize(request))?;
                emit(Wire::Retire(next))?;
                let until = Instant::now() + Duration::from_secs(5);
                loop {
                    if shared.stop.load(Ordering::Acquire)
                        || crate::d3d12_late_resize::OwnedLateResizeHook::failed()
                        || Instant::now() >= until
                    {
                        return Err("automatic retirement canceled/failed/deadline".into());
                    }
                    match input.recv_timeout(Duration::from_millis(25)) {
                        Ok(Ok(Wire::Retired(session))) if session == next => break,
                        Ok(Ok(Wire::Control(Message::Ack {
                            session,
                            slot,
                            sequence,
                        }))) => acknowledge_pending(
                            &mut pending,
                            hello.session,
                            session,
                            slot,
                            sequence,
                        )?,
                        Ok(Ok(Wire::Control(Message::Stop))) => {
                            stop_forwarding(shared, factory, hook, &out, published)?;
                            return Ok(());
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        other => {
                            return Err(format!(
                                "unexpected automatic retirement packet: {other:?}"
                            ))
                        }
                    }
                }
                let mut gpu = D3d12GpuBridge::new(
                    &resources.device,
                    &offer.queue,
                    offer.width,
                    offer.height,
                    resources.budget.clone(),
                    resources.warp,
                )?;
                let replacement = gpu.export_hello(&resources.device, next)?;
                epoch.offer(replacement)?;
                hook.stage_resize(&offer.chain, gpu)?;
                emit(Wire::Hello(replacement))?;
                let until = Instant::now() + Duration::from_secs(5);
                loop {
                    if shared.stop.load(Ordering::Acquire)
                        || crate::d3d12_late_resize::OwnedLateResizeHook::failed()
                        || Instant::now() >= until
                    {
                        return Err("automatic approval canceled/failed/deadline".into());
                    }
                    match input.recv_timeout(Duration::from_millis(25)) {
                        Ok(Ok(Wire::Control(Message::Select(session)))) if session == next => break,
                        Ok(Ok(Wire::Control(Message::Stop))) => {
                            stop_forwarding(shared, factory, hook, &out, published)?;
                            return Ok(());
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        other => {
                            return Err(format!("unexpected automatic approval packet: {other:?}"))
                        }
                    }
                }
                hook.activate_resize()?;
                crate::d3d12_late_resize::OwnedLateResizeHook::approve_automatic(next.epoch)?;
                pending.fill(None);
                hello = replacement;
                continue;
            }
        }
        match input.try_recv() {
            Ok(Ok(Wire::Control(Message::Render { color: 1, .. }))) => {
                emit(Wire::Control(Message::Report {
                    presented: startup.prior_presents,
                    published: 0,
                    dropped: 0,
                }))?;
            }
            Ok(Ok(Wire::Control(Message::Ack {
                session,
                slot,
                sequence,
            }))) => {
                acknowledge_pending(&mut pending, hello.session, session, slot, sequence)?;
                #[cfg(feature = "d3d12-auto-resize-fixture")]
                let suspended = crate::d3d12_late_resize::automatic_media_suspended();
                #[cfg(not(feature = "d3d12-auto-resize-fixture"))]
                let suspended = false;
                if !suspended {
                    let ack = hook.access(|gpu| gpu.remote_ack(session, slot, sequence));
                    #[cfg(feature = "d3d12-auto-resize-fixture")]
                    if ack.is_err() && crate::d3d12_late_resize::automatic_media_suspended() {
                        continue;
                    }
                    ack?;
                }
            }
            #[cfg(feature = "d3d12-rearm-fixture")]
            Ok(Ok(command @ (Wire::Resize(_) | Wire::ReplaceDevice(_)))) => {
                #[cfg(feature = "d3d12-auto-resize-fixture")]
                if automatic {
                    return Err(
                        "automatic source cannot accept parent capture-resize commands".into(),
                    );
                }
                let replacing = matches!(command, Wire::ReplaceDevice(_));
                if replacing
                    && (!cfg!(feature = "d3d12-device-replace-fixture")
                        || !std::env::args().any(|arg| arg == "--replace-device"))
                {
                    return Err("device replacement not enabled for this source".into());
                }
                let request = match command {
                    Wire::Resize(request) | Wire::ReplaceDevice(request) => request,
                    _ => unreachable!(),
                };
                if startup.resize.is_none() {
                    return Err("rearm is not enabled for this source".into());
                }
                let mut epoch = crate::resize_epoch::ResizeEpoch::new(hello)?;
                let next = epoch.begin(request)?;
                #[cfg(feature = "d3d12-device-replace-fixture")]
                let pending_loss = replacing && fault == Some("replace-remove-pending");
                #[cfg(not(feature = "d3d12-device-replace-fixture"))]
                let pending_loss = false;
                #[cfg(feature = "d3d12-device-replace-fixture")]
                if pending_loss {
                    hook.pause_for_pending_loss();
                    let (answer, reply) = sync_channel(1);
                    actions
                        .try_send(Action {
                            action: ActionKind::PauseForPendingLoss,
                            answer,
                        })
                        .map_err(|e| e.to_string())?;
                    reply
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|e| e.to_string())??;
                }
                if !pending_loss {
                    hook.cancel();
                }
                emit(Wire::Retire(next))?;
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() || shared.stop.load(Ordering::Acquire) {
                        return Err("old GPU epoch retirement deadline".into());
                    }
                    match input.recv_timeout(remaining.min(Duration::from_millis(25))) {
                        Ok(Ok(Wire::Retired(session))) if session == next => break,
                        Ok(Ok(Wire::Control(Message::Ack {
                            session,
                            slot,
                            sequence,
                        }))) => {
                            acknowledge_pending(
                                &mut pending,
                                hello.session,
                                session,
                                slot,
                                sequence,
                            )?;
                            if pending_loss {
                                hook.access(|gpu| gpu.remote_ack(session, slot, sequence))?;
                            }
                        }
                        Ok(Ok(Wire::Control(Message::Stop))) => {
                            #[cfg(feature = "d3d12-device-replace-fixture")]
                            if pending_loss {
                                let (answer, reply) = sync_channel(1);
                                actions
                                    .try_send(Action {
                                        action: ActionKind::ResumeAfterPendingLossCancel,
                                        answer,
                                    })
                                    .map_err(|e| e.to_string())?;
                                reply
                                    .recv_timeout(Duration::from_secs(3))
                                    .map_err(|e| e.to_string())??;
                            }
                            stop_forwarding(shared, factory, hook, &out, published)?;
                            return Ok(());
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        other => {
                            return Err(format!(
                                "unexpected old epoch retirement packet: {other:?}"
                            ))
                        }
                    }
                }
                #[cfg(feature = "d3d12-device-replace-fixture")]
                if pending_loss {
                    let old = startup
                        .resize
                        .as_ref()
                        .ok_or("missing pending-loss graph")?;
                    hook.retired_remote_pending_loss(hello.session, &old.device, &old.chain)?;
                }
                hook.release_media_for_resize()?;
                #[cfg(feature = "d3d12-device-replace-fixture")]
                let replacement_config = if replacing {
                    let resources = startup
                        .resize
                        .take()
                        .ok_or("missing old replacement resources")?;
                    let config = (resources.warp, resources.budget.clone());
                    drop(resources); // release ALL worker chain/queue/device interfaces
                    crate::d3d12_factory_hook::discard_pending_for_replacement()?;
                    Some(config)
                } else {
                    None
                };
                let (answer, reply) = sync_channel(1);
                let action = ActionKind::Resize(request.width, request.height);
                #[cfg(feature = "d3d12-device-replace-fixture")]
                let action = if replacing {
                    ActionKind::ReplaceDevice(request.width, request.height)
                } else {
                    action
                };
                actions
                    .try_send(Action { action, answer })
                    .map_err(|e| e.to_string())?;
                reply
                    .recv_timeout(Duration::from_secs(8))
                    .map_err(|e| e.to_string())??;
                validate(target)?;
                #[cfg(feature = "d3d12-device-replace-fixture")]
                if let Some((warp, budget)) = replacement_config {
                    let observed =
                        claim_window(target.window)?.ok_or("replacement creation not observed")?;
                    let (chain, queue, window) = observed.into_parts();
                    let mut device = None::<ID3D12Device>;
                    unsafe { queue.GetDevice(&mut device) }.map_err(|e| e.to_string())?;
                    let device = device.ok_or("null replacement device")?;
                    let chain_device: ID3D12Device =
                        unsafe { chain.GetDevice() }.map_err(|e| e.to_string())?;
                    if window as u64 != target.window
                        || unsafe { chain.GetHwnd() }.map_err(|e| e.to_string())?.0 as usize as u64
                            != target.window
                        || unsafe { queue.GetDesc() }.Type != D3D12_COMMAND_LIST_TYPE_DIRECT
                        || identity(&device)? != identity(&chain_device)?
                    {
                        return Err(
                            "replacement DIRECT/device/chain/HWND association mismatch".into()
                        );
                    }
                    unsafe { device.GetDeviceRemovedReason() }.map_err(|e| e.to_string())?;
                    startup.resize = Some(ResizeResources {
                        device,
                        queue,
                        chain,
                        warp,
                        budget,
                    });
                }
                let resources = startup.resize.as_ref().ok_or("missing rearm resources")?;
                let desc = unsafe { resources.chain.GetDesc1() }.map_err(|e| e.to_string())?;
                if (desc.Width, desc.Height) != (request.width, request.height)
                    || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
                    || desc.SampleDesc.Count != 1
                {
                    return Err("renderer resize descriptor mismatch".into());
                }
                let mut gpu = D3d12GpuBridge::new(
                    &resources.device,
                    &resources.queue,
                    desc.Width,
                    desc.Height,
                    resources.budget.clone(),
                    resources.warp,
                )?;
                let replacement = gpu.export_hello(&resources.device, next)?;
                epoch.offer(replacement)?;
                #[cfg(feature = "d3d12-device-replace-fixture")]
                if replacing {
                    hook.stage_device_replacement(&resources.chain, gpu, target.window)?;
                } else {
                    hook.stage_resize(&resources.chain, gpu)?;
                }
                #[cfg(not(feature = "d3d12-device-replace-fixture"))]
                hook.stage_resize(&resources.chain, gpu)?;
                if replacing && fault == Some("replace-before-hello") {
                    return Err("intentional replacement failure before hello".into());
                }
                let replacement = if replacing && fault == Some("replace-bad-hello") {
                    crate::d3d12_protocol::GpuHello {
                        session: SessionIdentity {
                            epoch: next.epoch.checked_add(1).ok_or("fault epoch overflow")?,
                            ..next
                        },
                        ..replacement
                    }
                } else {
                    replacement
                };
                emit(Wire::Hello(replacement))?;
                match input
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|e| e.to_string())??
                {
                    Wire::Control(Message::Select(session)) if session == next => {}
                    Wire::Control(Message::Stop) => {
                        stop_forwarding(shared, factory, hook, &out, published)?;
                        return Ok(());
                    }
                    _ => return Err("replacement epoch approval mismatch".into()),
                }
                pending.fill(None);
                hello = replacement;
                hook.activate_resize()?;
            }
            Ok(Ok(Wire::Control(Message::Render { color: 2, .. }))) => {
                let (forwarded, observed, claimed, dropped, errors) = factory.counts();
                if dropped != 0 || errors != 0 {
                    return Err("bootstrap factory diagnostics failed".into());
                }
                emit(Wire::Control(Message::Report {
                    presented: forwarded,
                    published: observed,
                    dropped: claimed,
                }))?;
            }
            Ok(Ok(Wire::Control(Message::Render { color: 3, .. }))) => {
                let (selected, unrelated, captured, _) = hook.counts();
                emit(Wire::Control(Message::Report {
                    presented: selected,
                    published: captured,
                    dropped: unrelated,
                }))?;
            }
            Ok(Ok(Wire::Control(Message::Render { .. }))) => {
                let (_, _, _, dropped) = hook.counts();
                emit(Wire::Control(Message::Report {
                    presented: hook.counts().0,
                    published,
                    dropped,
                }))?;
            }
            Ok(Ok(Wire::Window(action))) => {
                let (answer, reply) = sync_channel(1);
                actions
                    .try_send(Action {
                        action: ActionKind::Window(action),
                        answer,
                    })
                    .map_err(|e| e.to_string())?;
                let (flags, qpc) = reply
                    .recv_timeout(Duration::from_secs(3))
                    .map_err(|e| e.to_string())??;
                emit(Wire::Control(Message::Report {
                    presented: action as u64,
                    published: qpc as u64,
                    dropped: flags,
                }))?;
            }
            Ok(Ok(Wire::Control(Message::Stop))) => {
                stop_forwarding(shared, factory, hook, &out, published)?;
                return Ok(());
            }
            Ok(Err(error)) => return Err(error),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return Err("bootstrap input disconnected".into()),
            _ => return Err("unexpected bootstrap command".into()),
        }
        #[cfg(feature = "d3d12-auto-resize-fixture")]
        if crate::d3d12_late_resize::automatic_media_suspended() {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        let exported = hook.access(|gpu| gpu.export_frame());
        #[cfg(feature = "d3d12-auto-resize-fixture")]
        if exported.is_err() && crate::d3d12_late_resize::automatic_media_suspended() {
            continue;
        }
        if let Some(mut header) = exported? {
            let slot = pending
                .get_mut(header.slot as usize)
                .ok_or("invalid published slot")?;
            if slot.replace(header.sequence).is_some() {
                return Err("published slot reused without exact ACK".into());
            }
            if fault == Some("bad-frame") {
                header.session.epoch += 1;
            }
            emit(Wire::Control(Message::Frame(header)))?;
            published += 1;
            if fault == Some("exit-after-six") && published >= 6 {
                return Err("intentional bootstrap producer exit".into());
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    })();
    drop(out);
    if result.is_ok() {
        writer.join().map_err(|_| "bootstrap writer panicked")??;
    }
    result
}

fn acknowledge_pending(
    pending: &mut [Option<u64>; 3],
    expected: SessionIdentity,
    session: SessionIdentity,
    slot: u32,
    sequence: u64,
) -> Result<(), String> {
    if session != expected || sequence == 0 {
        return Err("stale or mismatched epoch ACK".into());
    }
    let entry = pending.get_mut(slot as usize).ok_or("invalid ACK slot")?;
    if *entry != Some(sequence) {
        return Err("stale or mismatched publication ACK".into());
    }
    *entry = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endurance_lifetime_is_explicit_strict_and_bounded() {
        assert_eq!(fixture_lifetime(None).unwrap(), Duration::from_secs(175));
        assert_eq!(
            fixture_lifetime(Some("28830")).unwrap(),
            Duration::from_secs(28830)
        );
        for value in [
            "",
            "0",
            "29",
            "28831",
            "-1",
            "+60",
            "60 ",
            "18446744073709551616",
        ] {
            assert!(fixture_lifetime(Some(value)).is_err(), "{value}");
        }
    }
    #[test]
    fn resize_ack_snapshot_requires_exact_session_slot_sequence_once() {
        let session = SessionIdentity {
            target: TargetIdentity {
                pid: 1,
                creation_time: 2,
                window: 3,
            },
            generation: 1,
            epoch: 1,
        };
        let mut pending = [Some(1), Some(2), None];
        for (identity, slot, sequence) in [
            (
                SessionIdentity {
                    epoch: 2,
                    ..session
                },
                0,
                1,
            ),
            (session, 0, 2),
            (session, 3, 1),
            (session, 2, 1),
            (session, 0, 0),
        ] {
            assert!(acknowledge_pending(&mut pending, session, identity, slot, sequence).is_err());
            assert_eq!(pending, [Some(1), Some(2), None]);
        }
        acknowledge_pending(&mut pending, session, session, 0, 1).unwrap();
        assert!(acknowledge_pending(&mut pending, session, session, 0, 1).is_err());
        let next = SessionIdentity {
            epoch: 2,
            ..session
        };
        let mut fresh = [Some(1), None, None];
        assert!(acknowledge_pending(&mut fresh, next, session, 0, 1).is_err());
        assert_eq!(fresh, [Some(1), None, None]);
    }
    #[test]
    fn bootstrap_rejects_test_process_before_threads_or_graphics() {
        assert!(run_owned_producer(true, None).is_err());
    }
    #[test]
    fn bootstrap_target_never_defaults_to_foreground_or_monitor() {
        for target in [
            TargetIdentity {
                pid: 0,
                creation_time: 0,
                window: 0,
            },
            TargetIdentity {
                pid: std::process::id(),
                creation_time: 1,
                window: 0,
            },
        ] {
            assert!(validate(target).is_err());
        }
    }
}
