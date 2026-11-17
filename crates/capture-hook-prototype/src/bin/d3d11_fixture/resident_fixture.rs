//! Select only the passive child launched here. Never discovers or targets games.
use capture_hook_prototype::cooperative_peer::FixturePeer;
use capture_hook_prototype::owned_attachment::{attach_owned_fixture, select_owned_window};
use capture_hook_prototype::shared_transport::{ConsumerRing, ImportedFrame};
use capture_hook_prototype::transport::Message;
use capture_hook_prototype::{SessionIdentity, TargetIdentity};
use gpu_windows::BgraReadback;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetPropW;
fn progress(session: SessionIdentity) -> usize {
    unsafe {
        GetPropW(
            HWND(session.target.window as usize as *mut _),
            w!("SilkFixturePresents"),
        )
    }
    .0 as usize
}
fn wait(label: &str, mut check: impl FnMut() -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !check() {
        if Instant::now() > deadline {
            return Err(format!("resident fixture deadline: {label}"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
fn frame(peer: &FixturePeer, ring: &mut ConsumerRing) -> Result<ImportedFrame, String> {
    let Message::Frame(header) = peer.receive(Duration::from_secs(5))? else {
        return Err("resident helper omitted frame".into());
    };
    ring.receive(header)
}
fn red(frame: &ImportedFrame) -> Result<(), String> {
    let pixels = BgraReadback::default()
        .read_scaled(frame.surface(), frame.header.width, frame.header.height)
        .map_err(|e| e.to_string())?;
    if !pixels.as_chunks::<4>().0.iter().all(|pixel| {
        pixel
            .iter()
            .zip([0u8, 0, 255, 255])
            .all(|(a, b)| a.abs_diff(b) <= 1)
    }) {
        return Err("resident source was blank, partial or the unrelated blue chain".into());
    }
    Ok(())
}
pub fn parent(warp: bool) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut peer = FixturePeer::launch_passive_fixture(&executable, warp)?;
    let session = select_owned_window(&peer)?;
    wait("rendering before attachment", || progress(session) >= 10)?;
    let wrong = SessionIdentity {
        target: TargetIdentity {
            creation_time: session.target.creation_time + 1,
            ..session.target
        },
        ..session
    };
    if attach_owned_fixture(&peer, wrong).is_ok() {
        return Err("owned loader accepted changed process lifetime".into());
    }
    let mut worker = attach_owned_fixture(&peer, session)?;
    let Message::Hello(hello) = peer.receive(Duration::from_secs(15))? else {
        return Err("resident worker omitted greeting".into());
    };
    if hello.session != session {
        return Err("resident discovery changed approved source".into());
    }
    let mut ring = unsafe { ConsumerRing::from_child(peer.handle(), peer.pid(), hello, warp) }?;
    let first = frame(&peer, &mut ring)?;
    red(&first)?;
    let second = frame(&peer, &mut ring)?;
    let third = frame(&peer, &mut ring)?;
    red(&second)?;
    red(&third)?;
    let before = progress(session);
    wait("rendering with full resident ring", || {
        progress(session) >= before + 12
    })?;
    peer.send(Message::Render { count: 1, color: 0 })?;
    match peer.receive(Duration::from_secs(5))? {
        Message::Report {
            presented, dropped, ..
        } if presented > 0 && dropped > 0 => {}
        _ => return Err("resident pressure did not report continued original Present".into()),
    }
    // Keep all slots held: Stop must not require ACKs or renderer participation.
    peer.send(Message::Stop)?;
    if peer.receive(Duration::from_secs(5))? != Message::Bye {
        return Err("resident cancellation did not complete".into());
    }
    worker.finish()?;
    if attach_owned_fixture(&peer, session).is_ok() {
        return Err("resident loader allowed repeated remote allocation/attachment".into());
    }
    peer.close_input();
    let after = progress(session);
    wait("original Present after worker exit", || {
        progress(session) >= after + 20
    })?;
    if !peer.is_running() {
        return Err("resident cancellation terminated the owned renderer".into());
    }
    red(&first)?;
    drop((first, second, third));
    println!("PASS: selected owned process already rendering; normal remote DLL load; worker-side Present discovery without pointer handoff/pause; unrelated-chain isolation; bounded full-ring progress; cancellation and worker exit while original Present continues; resident trampoline/DLL retained. driver={}",if warp{"WARP"}else{"hardware"});
    Ok(())
}
