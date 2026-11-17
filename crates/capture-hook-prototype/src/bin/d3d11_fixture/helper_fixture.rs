//! Cooperative owned-process DLL tests. No injector or arbitrary target API.
use capture_hook_prototype::cooperative_peer::{read_message, write_message, FixturePeer};
use capture_hook_prototype::helper_abi as abi;
use capture_hook_prototype::native::own_window_identity;
use capture_hook_prototype::shared_transport::{
    validate_selected_child, ConsumerRing, ImportedFrame,
};
use capture_hook_prototype::transport::{Message, PACKET_SIZE};
use capture_hook_prototype::SessionIdentity;
use gpu_windows::BgraReadback;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{
    mpsc::{sync_channel, TryRecvError},
    Arc,
};
use std::time::{Duration, Instant};
use windows::core::{s, Interface, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
    ID3D11Texture2D, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain, DXGI_PRESENT};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

struct Helper {
    dispatch: abi::Dispatch,
}
impl Helper {
    fn load() -> Result<Self, String> {
        if !std::env::args().any(|arg| arg == "--allow-helper-load") {
            return Err("explicit helper load opt-in required".into());
        }
        let executable = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let directory = executable.parent().ok_or("fixture has no directory")?;
        let path = directory
            .join("silk_capture_hook_helper.dll")
            .canonicalize()
            .map_err(|e| format!("build the experimental helper first: {e}"))?;
        if path.parent() != Some(directory) {
            return Err("helper must be a canonical build-directory sibling".into());
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // Intentionally retain this LoadLibrary reference until process exit,
        // even on export lookup/attachment failure. No unsafe FreeLibrary path.
        let module = unsafe {
            LoadLibraryExW(
                PCWSTR(wide.as_ptr()),
                None,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .map_err(|e| e.to_string())?;
        let address = unsafe { GetProcAddress(module, s!("silk_capture_helper_dispatch")) }
            .ok_or("helper ABI export absent")?;
        let dispatch = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, abi::Dispatch>(address)
        };
        Ok(Self { dispatch })
    }
    fn raw(
        &self,
        version: u32,
        op: u32,
        chain: Option<&IDXGISwapChain>,
        request: Message,
    ) -> (u32, [u8; PACKET_SIZE]) {
        let request = request.encode();
        let mut response = [0; PACKET_SIZE];
        let status = unsafe {
            (self.dispatch)(
                version,
                op,
                chain.map_or(std::ptr::null_mut(), Interface::as_raw),
                request.as_ptr(),
                request.len(),
                response.as_mut_ptr(),
                response.len(),
            )
        };
        (status, response)
    }
    fn call(
        &self,
        op: u32,
        chain: Option<&IDXGISwapChain>,
        request: Message,
    ) -> Result<Option<Message>, String> {
        let (status, response) = self.raw(abi::VERSION, op, chain, request);
        match status {
            abi::OK => Message::decode(&response)
                .map(Some)
                .map_err(|e| format!("helper ABI response: {e:?}")),
            abi::EMPTY => Ok(None),
            _ => Err(format!("helper rejected operation {op}: status={status}")),
        }
    }
    fn stop(&self) -> Result<(), String> {
        self.call(abi::STOP, None, Message::Stop)?;
        Ok(())
    }
}
use std::os::windows::ffi::OsStrExt;

fn draw(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    chain: &IDXGISwapChain,
    color: u32,
) -> Result<(), String> {
    unsafe {
        let buffer: ID3D11Texture2D = chain.GetBuffer(0).map_err(|e| e.to_string())?;
        let mut view: Option<ID3D11RenderTargetView> = None;
        device
            .CreateRenderTargetView(&buffer, None, Some(&mut view))
            .map_err(|e| e.to_string())?;
        let colors = [
            [1., 0., 0., 1.],
            [0., 0., 1., 1.],
            [0., 1., 0., 1.],
            [1., 1., 0., 1.],
        ];
        context.ClearRenderTargetView(
            view.as_ref().ok_or("null helper fixture view")?,
            &colors[color as usize],
        );
        chain
            .Present(0, DXGI_PRESENT(0))
            .ok()
            .map_err(|e| e.to_string())
    }
}

pub fn producer(
    mut chain: IDXGISwapChain,
    mut device: ID3D11Device,
    mut context: ID3D11DeviceContext,
    warp: bool,
) -> Result<(), String> {
    if !std::env::args().any(|arg| arg == "--allow-helper-load") {
        return Err("helper opt-in missing".into());
    }
    let desc = unsafe { chain.GetDesc() }.map_err(|e| e.to_string())?;
    let mut session = SessionIdentity {
        target: own_window_identity(desc.OutputWindow)?,
        generation: 1,
        epoch: 1,
    };
    let (input, commands) = sync_channel(4);
    // Bounded pipe queues keep all pipe IO off the independently paced renderer.
    let _reader = std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        loop {
            let message = read_message(&mut stdin);
            let failed = message.is_err();
            if input.send(message).is_err() || failed {
                break;
            }
        }
    });
    let (output, messages) = sync_channel(8);
    let broken = Arc::new(AtomicBool::new(false));
    let writer_broken = broken.clone();
    let writer = std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        for message in messages {
            if write_message(&mut stdout, message).is_err() {
                writer_broken.store(true, Ordering::Release);
                break;
            }
        }
    });
    let emit = |message| {
        output
            .try_send(message)
            .map_err(|e| format!("bounded helper output unavailable: {e}"))
    };
    emit(Message::Select(session))?;
    let mut helper: Option<Helper> = None;
    let mut attached = false;
    let mut color = 0;
    let mut presented = 0;
    let mut published = 0;
    let mut dropped = 0;
    let mut next = Instant::now();
    let mut disconnected_at = None;
    let deadline = Instant::now() + Duration::from_secs(60);
    let result = (|| {
        loop {
            if Instant::now() > deadline {
                return Err("helper fixture overall deadline".into());
            }
            let mut msg = MSG::default();
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            let command = if disconnected_at.is_none() {
                commands.try_recv()
            } else {
                Err(TryRecvError::Empty)
            };
            match command {
                Ok(Ok(Message::Select(selected))) => {
                    if attached || selected != session {
                        return Err("controller approved a different identity/epoch".into());
                    }
                    if helper.is_none() {
                        let loaded = Helper::load()?;
                        if loaded
                            .raw(
                                abi::VERSION + 1,
                                abi::ATTACH,
                                Some(&chain),
                                Message::Select(session),
                            )
                            .0
                            != abi::REJECTED
                        {
                            return Err("bad helper ABI version accepted".into());
                        }
                        helper = Some(loaded);
                    }
                    let loaded = helper.as_ref().unwrap();
                    // Rejected target must install no hook; the valid request
                    // immediately afterwards proves failed attach can be retried.
                    let wrong = SessionIdentity {
                        target: capture_hook_prototype::TargetIdentity {
                            creation_time: session.target.creation_time + 1,
                            ..session.target
                        },
                        ..session
                    };
                    if loaded
                        .raw(
                            abi::VERSION,
                            abi::ATTACH,
                            Some(&chain),
                            Message::Select(wrong),
                        )
                        .0
                        != abi::REJECTED
                    {
                        return Err("DLL accepted changed creation time".into());
                    }
                    let hello = loaded
                        .call(abi::ATTACH, Some(&chain), Message::Select(session))?
                        .ok_or("helper omitted greeting")?;
                    let foreign_status = std::thread::scope(|scope| {
                        scope
                            .spawn(|| loaded.raw(abi::VERSION, abi::STOP, None, Message::Stop).0)
                            .join()
                            .map_err(|_| "foreign-owner ABI probe panicked")
                    })?;
                    if foreign_status != abi::REJECTED {
                        return Err(
                            "foreign thread was allowed to stop the owner-thread helper".into()
                        );
                    }
                    attached = true;
                    emit(hello)?;
                }
                Ok(Ok(Message::Ack {
                    session,
                    slot,
                    sequence,
                })) => {
                    if !attached {
                        return Err("ACK outside attached session".into());
                    }
                    helper.as_ref().unwrap().call(
                        abi::ACK,
                        None,
                        Message::Ack {
                            session,
                            slot,
                            sequence,
                        },
                    )?;
                }
                Ok(Ok(Message::Render {
                    color: requested, ..
                })) => {
                    color = requested;
                    emit(Message::Report {
                        presented,
                        published,
                        dropped,
                    })?;
                }
                Ok(Ok(Message::Stop)) => {
                    if let Some(helper) = &helper {
                        helper.stop()?;
                        if helper
                            .raw(
                                abi::VERSION,
                                abi::ATTACH,
                                Some(&chain),
                                Message::Select(session),
                            )
                            .0
                            != abi::REJECTED
                        {
                            return Err("helper accepted reuse of its stopped epoch".into());
                        }
                    }
                    attached = false;
                    emit(Message::Bye)?;
                }
                Ok(Ok(Message::Rearm {
                    epoch,
                    width,
                    height,
                    recreate,
                })) => {
                    if attached || epoch <= session.epoch {
                        return Err("rearm requires stopped helper and increasing epoch".into());
                    }
                    if recreate {
                        // No tap or Present call is active. Destroy only owned resources.
                        let mut desc = desc;
                        desc.BufferDesc.Width = width;
                        desc.BufferDesc.Height = height;
                        let mut new_chain = None;
                        let mut new_device = None;
                        let mut new_context = None;
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
                                Some(&mut new_chain),
                                Some(&mut new_device),
                                None,
                                Some(&mut new_context),
                            )
                        }
                        .map_err(|e| e.to_string())?;
                        chain = new_chain.ok_or("null recreated chain")?;
                        device = new_device.ok_or("null recreated device")?;
                        context = new_context.ok_or("null recreated context")?;
                        color = 1;
                    } else {
                        unsafe {
                            chain.ResizeBuffers(
                                2,
                                width,
                                height,
                                DXGI_FORMAT_UNKNOWN,
                                Default::default(),
                            )
                        }
                        .map_err(|e| e.to_string())?;
                        color = 2;
                    }
                    session.epoch = epoch;
                    emit(Message::Select(session))?;
                }
                Ok(Ok(_)) => return Err("unexpected helper fixture command".into()),
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                    if let Some(helper) = &helper {
                        helper.stop()?;
                    }
                    attached = false;
                    disconnected_at = Some(presented);
                }
                Err(TryRecvError::Empty) => {}
            }
            if broken.load(Ordering::Acquire) && disconnected_at.is_none() {
                if let Some(helper) = &helper {
                    helper.stop()?;
                }
                attached = false;
                disconnected_at = Some(presented);
            }
            if Instant::now() >= next {
                draw(&device, &context, &chain, color)?;
                presented += 1;
                if attached {
                    match helper
                        .as_ref()
                        .unwrap()
                        .call(abi::POLL, None, Message::Stop)?
                    {
                        Some(message) => {
                            emit(message)?;
                            published += 1;
                        }
                        None => dropped += 1,
                    }
                }
                next = Instant::now() + Duration::from_millis(16);
            }
            if disconnected_at.is_some_and(|start| presented >= start + 12) {
                if !broken.load(Ordering::Acquire) {
                    emit(Message::Report {
                        presented: 12,
                        published: 0,
                        dropped: 0,
                    })?;
                    emit(Message::Bye)?;
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    })();
    // On every normal error, stop on the render owner before dropping graphics.
    if let Some(helper) = &helper {
        helper.stop()?;
        if result.is_ok() && session.epoch == 3 {
            let fourth = SessionIdentity {
                epoch: 4,
                ..session
            };
            let status = helper
                .raw(
                    abi::VERSION,
                    abi::ATTACH,
                    Some(&chain),
                    Message::Select(fourth),
                )
                .0;
            helper.stop()?;
            if status != abi::REJECTED {
                return Err("helper exceeded its three-epoch shared-surface budget".into());
            }
        }
    }
    drop(output);
    // Joining a writer can block if a live controller stops draining its pipe;
    // do not join it on an error path. Process exit retains the DLL reference.
    if result.is_ok() {
        writer.join().map_err(|_| "helper writer panicked")?;
    }
    result
}

fn selected(peer: &mut FixturePeer) -> Result<SessionIdentity, String> {
    let Message::Select(session) = peer.receive(Duration::from_secs(8))? else {
        return Err("missing target offer".into());
    };
    unsafe { validate_selected_child(peer.handle(), peer.pid(), session) }?;
    for target in [
        capture_hook_prototype::TargetIdentity {
            pid: session.target.pid + 1,
            ..session.target
        },
        capture_hook_prototype::TargetIdentity {
            creation_time: session.target.creation_time + 1,
            ..session.target
        },
        capture_hook_prototype::TargetIdentity {
            window: 0,
            ..session.target
        },
    ] {
        if unsafe {
            validate_selected_child(
                peer.handle(),
                peer.pid(),
                SessionIdentity { target, ..session },
            )
        }
        .is_ok()
        {
            return Err("controller accepted forged target offer".into());
        }
    }
    peer.send(Message::Select(session))?;
    Ok(session)
}
fn imported(peer: &FixturePeer, ring: &mut ConsumerRing) -> Result<ImportedFrame, String> {
    match peer.receive(Duration::from_secs(5))? {
        Message::Frame(header) => ring.receive(header),
        _ => Err("expected helper frame".into()),
    }
}
fn pixels(frame: &ImportedFrame, expected: [u8; 4]) -> Result<(), String> {
    let bytes = BgraReadback::default()
        .read_scaled(frame.surface(), frame.header.width, frame.header.height)
        .map_err(|e| e.to_string())?;
    if !bytes
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| pixel.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1))
    {
        let mismatch = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .find(|(_, pixel)| pixel.iter().zip(expected).any(|(a, b)| a.abs_diff(b) > 1));
        return Err(format!("helper cross-process pixels changed: epoch={} expected={expected:?} mismatch={mismatch:?}",frame.header.session.epoch));
    }
    Ok(())
}
fn ack(peer: &mut FixturePeer, frame: ImportedFrame) -> Result<(), String> {
    let header = frame.release()?;
    peer.send(Message::Ack {
        session: header.session,
        slot: header.slot,
        sequence: header.sequence,
    })
}
fn stop(peer: &mut FixturePeer) -> Result<(), String> {
    peer.send(Message::Stop)?;
    for _ in 0..8 {
        match peer.receive(Duration::from_secs(5))? {
            Message::Bye => return Ok(()),
            Message::Frame(_) => {}
            _ => return Err("unexpected stop response".into()),
        }
    }
    Err("stop did not drain bounded output".into())
}
fn ring(peer: &mut FixturePeer, warp: bool) -> Result<ConsumerRing, String> {
    let selected = selected(peer)?;
    let Message::Hello(hello) = peer.receive(Duration::from_secs(8))? else {
        return Err("DLL did not greet".into());
    };
    if hello.session != selected {
        return Err("DLL changed approved identity".into());
    }
    unsafe { ConsumerRing::from_child(peer.handle(), peer.pid(), hello, warp) }
}

pub fn parent(warp: bool) -> Result<(), String> {
    let mut peer = FixturePeer::launch_helper(warp)?;
    let mut current = ring(&mut peer, warp)?;
    let red = imported(&peer, &mut current)?;
    pixels(&red, [0, 0, 255, 255]).map_err(|e| format!("initial: {e}"))?;
    // Hold all three transport slots; independent rendering must still advance.
    let held = [
        imported(&peer, &mut current)?,
        imported(&peer, &mut current)?,
    ];
    std::thread::sleep(Duration::from_millis(350));
    peer.send(Message::Render { count: 1, color: 0 })?;
    match peer.receive(Duration::from_secs(5))? {
        Message::Report {
            presented, dropped, ..
        } if presented >= 12 && dropped > 0 => {}
        _ => return Err("helper pressure blocked independent rendering".into()),
    }
    for frame in held {
        ack(&mut peer, frame)?;
    }
    stop(&mut peer)?;
    peer.send(Message::Render { count: 1, color: 0 })?;
    let before = match peer.receive(Duration::from_secs(5))? {
        Message::Report { presented, .. } => presented,
        _ => return Err("stopped app did not respond".into()),
    };
    std::thread::sleep(Duration::from_millis(100));
    peer.send(Message::Render { count: 1, color: 0 })?;
    if !matches!(peer.receive(Duration::from_secs(5))?,Message::Report {presented,..} if presented>before)
    {
        return Err("Stop terminated app rendering".into());
    }
    peer.send(Message::Rearm {
        epoch: 2,
        width: 800,
        height: 600,
        recreate: false,
    })?;
    current = ring(&mut peer, warp)?;
    let green = imported(&peer, &mut current)?;
    pixels(&green, [0, 255, 0, 255])?;
    if (green.header.width, green.header.height) != (800, 600) {
        return Err("resize dimensions lost".into());
    }
    stop(&mut peer)?;
    peer.send(Message::Rearm {
        epoch: 3,
        width: 960,
        height: 540,
        recreate: true,
    })?;
    current = ring(&mut peer, warp)?;
    let blue = imported(&peer, &mut current)?;
    pixels(&blue, [255, 0, 0, 255])?;
    if (blue.header.width, blue.header.height) != (960, 540) {
        return Err("recreated dimensions lost".into());
    }
    pixels(&red, [0, 0, 255, 255]).map_err(|e| format!("retained after recreate: {e}"))?;
    pixels(&green, [0, 255, 0, 255])?;
    // Simulate recorder input disappearance, not a graceful capture command.
    peer.close_input();
    let mut continued = false;
    for _ in 0..5 {
        match peer.receive(Duration::from_secs(5))? {
            Message::Frame(_) => {}
            Message::Report {
                presented: 12,
                published: 0,
                ..
            } => {
                continued = true;
                break;
            }
            _ => return Err("unexpected EOF cleanup response".into()),
        }
    }
    if !continued {
        return Err("app did not keep presenting after pipe EOF".into());
    }
    if peer.receive(Duration::from_secs(5))? != Message::Bye {
        return Err("missing EOF cleanup completion".into());
    }
    peer.expect_exit()?;
    pixels(&blue, [255, 0, 0, 255])?;
    // Crash the owned target while attached; no reattachment/desktop fallback.
    let mut crashed = FixturePeer::launch_helper(warp)?;
    let mut crashed_ring = ring(&mut crashed, warp)?;
    let retained = imported(&crashed, &mut crashed_ring)?;
    // Establish completion before terminating the WARP producer. Process death
    // may interrupt outstanding GPU work; acquire/header delivery alone is not
    // evidence that such an interrupted frame has valid pixels after a crash.
    pixels(&retained, [0, 0, 255, 255])?;
    crashed.kill()?;
    let _ = crashed.wait_status(Duration::from_secs(5))?;
    pixels(&retained, [0, 0, 255, 255]).map_err(|e| format!("retained after crash: {e}"))?;
    // Drain at most the bounded published frames, then require terminal EOF.
    let mut ended = false;
    for _ in 0..8 {
        if crashed.receive(Duration::from_secs(2)).is_err() {
            ended = true;
            break;
        }
    }
    if !ended {
        return Err("target exit did not close transport".into());
    }
    println!("PASS: cooperative DLL helper; ABI/identity rejection; independent cross-process pixels/pressure; Stop keeps app alive; resize/device rearm; retained frames; pipe EOF cleanup; owned target exit. driver={}",if warp{"WARP"}else{"hardware"});
    Ok(())
}
