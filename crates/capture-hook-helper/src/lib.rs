//! Default-off cooperative fixture DLL. No custom DllMain or self-unload.
#[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-helper"))]
mod d3d12_worker;
#[cfg(all(windows, target_arch = "x86_64", feature = "resident-helper"))]
mod resident_worker;
#[cfg(all(windows, target_arch = "x86_64", feature = "experimental-helper"))]
mod helper {
    use capture_hook_prototype::helper_abi::*;
    use capture_hook_prototype::native::{
        own_window_identity, CapturedFrame, PresentTap, TapStats,
    };
    use capture_hook_prototype::shared_transport::ProducerRing;
    use capture_hook_prototype::transport::{Message, PACKET_SIZE};
    use gpu_windows::GpuDeviceContext;
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{mpsc::Receiver, Arc};
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D11::ID3D11Device;
    use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
    use windows::Win32::System::Threading::GetCurrentThreadId;

    struct Session {
        tap: PresentTap,
        frames: Receiver<CapturedFrame>,
        stats: Arc<TapStats>,
        ring: ProducerRing,
    }
    thread_local! { static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) }; }
    thread_local! { static LAST: Cell<Option<capture_hook_prototype::SessionIdentity>> = const { Cell::new(None) }; }
    static OWNER: AtomicU32 = AtomicU32::new(0);
    // Remote duplicated handles may outlive Stop. This fixture cannot prove
    // their retirement, so cap successful rings over the DLL's process lifetime.
    const MAX_ATTACHMENTS: u32 = 3;
    thread_local! { static ATTACHMENTS: Cell<u32> = const { Cell::new(0) }; }

    fn opted_in() -> bool {
        std::env::args().any(|arg| arg == "--allow-helper-load")
            && std::env::current_exe().ok().is_some_and(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .eq_ignore_ascii_case("silk-d3d11-hook-fixture.exe")
                })
            })
    }

    unsafe fn dispatch(
        operation: u32,
        chain: *mut c_void,
        bytes: &[u8],
    ) -> Result<Option<Message>, String> {
        if !opted_in() {
            return Err("owned fixture opt-in required".into());
        }
        let current = GetCurrentThreadId();
        let owner = OWNER
            .compare_exchange(0, current, Ordering::AcqRel, Ordering::Acquire)
            .unwrap_or_else(|owner| owner);
        if owner != 0 && owner != current {
            return Err("helper operations require the original render owner".into());
        }
        if operation != ATTACH && !chain.is_null() {
            return Err("chain pointer only permitted on attach".into());
        }
        SESSION.with(|cell| {
            let mut state = cell.try_borrow_mut().map_err(|_| "helper busy")?;
            match operation {
                ATTACH => {
                    if state.is_some() || chain.is_null() {
                        return Err("already attached or null chain".into());
                    }
                    if ATTACHMENTS.with(|count| count.get() >= MAX_ATTACHMENTS) {
                        return Err(
                            "three-epoch cooperative helper budget exhausted; relaunch fixture"
                                .into(),
                        );
                    }
                    let Message::Select(session) =
                        Message::decode(bytes).map_err(|_| "bad selection")?
                    else {
                        return Err("selection required".into());
                    };
                    if LAST.with(|last| {
                        last.get().is_some_and(|last| {
                            session.target != last.target
                                || session.generation != last.generation
                                || session.epoch <= last.epoch
                        })
                    }) {
                        return Err(
                            "rearm requires the same target and a strictly newer epoch".into()
                        );
                    }
                    let chain = IDXGISwapChain::from_raw_borrowed(&chain).ok_or("null chain")?;
                    let desc = chain.GetDesc().map_err(|error| error.to_string())?;
                    if own_window_identity(desc.OutputWindow)? != session.target {
                        return Err("selected process/window changed".into());
                    }
                    let device: ID3D11Device =
                        chain.GetDevice().map_err(|error| error.to_string())?;
                    let context = device
                        .GetImmediateContext()
                        .map_err(|error| error.to_string())?;
                    let ring = ProducerRing::new(
                        Arc::new(GpuDeviceContext::new(device, context)),
                        session,
                        desc.BufferDesc.Width,
                        desc.BufferDesc.Height,
                    )?;
                    let hello = ring.hello();
                    let (tap, frames, stats) = PresentTap::attach(chain, session)?;
                    *state = Some(Session {
                        tap,
                        frames,
                        stats,
                        ring,
                    });
                    LAST.with(|last| last.set(Some(session)));
                    ATTACHMENTS.with(|count| count.set(count.get() + 1));
                    Ok(Some(Message::Hello(hello)))
                }
                STOP => {
                    if !matches!(Message::decode(bytes), Ok(Message::Stop)) {
                        return Err("stop packet required".into());
                    }
                    if let Some(session) = state.as_mut() {
                        session.stats.cancel_capture();
                        session.tap.detach()?;
                    }
                    state.take();
                    Ok(Some(Message::Bye))
                }
                POLL => {
                    if !matches!(Message::decode(bytes), Ok(Message::Stop)) {
                        return Err("poll requires an empty Stop envelope".into());
                    }
                    let session = state.as_mut().ok_or("not attached")?;
                    if session.stats.invalidated.load(Ordering::Acquire) {
                        return Err("source invalidated".into());
                    }
                    match session.frames.try_recv() {
                        Ok(frame) => Ok(session.ring.export(&frame)?.map(Message::Frame)),
                        Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
                        Err(_) => Err("capture closed".into()),
                    }
                }
                ACK => {
                    let Message::Ack {
                        session,
                        slot,
                        sequence,
                    } = Message::decode(bytes).map_err(|_| "bad ACK")?
                    else {
                        return Err("ACK required".into());
                    };
                    state
                        .as_mut()
                        .ok_or("not attached")?
                        .ring
                        .acknowledge(session, slot, sequence)?;
                    Ok(None)
                }
                _ => Err("unsupported operation".into()),
            }
        })
    }

    /// # Safety
    /// Call only on the fixture's sole render owner with no Present in flight.
    /// Buffers must be valid, nonoverlapping allocations of PACKET_SIZE bytes;
    /// ATTACH borrows a live IDXGISwapChain pointer. No unload before process exit.
    #[no_mangle]
    pub unsafe extern "system" fn silk_capture_helper_dispatch(
        version: u32,
        operation: u32,
        chain: *mut c_void,
        request: *const u8,
        request_len: usize,
        response: *mut u8,
        response_len: usize,
    ) -> u32 {
        if version != VERSION
            || request_len != PACKET_SIZE
            || response_len != PACKET_SIZE
            || request.is_null()
            || response.is_null()
        {
            return REJECTED;
        }
        // Copy input first: do not retain caller-owned memory across calls.
        let mut bytes = [0; PACKET_SIZE];
        std::ptr::copy_nonoverlapping(request, bytes.as_mut_ptr(), PACKET_SIZE);
        std::ptr::write_bytes(response, 0, PACKET_SIZE);
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            dispatch(operation, chain, &bytes)
        })) {
            Ok(Ok(Some(message))) => {
                std::ptr::copy_nonoverlapping(message.encode().as_ptr(), response, PACKET_SIZE);
                OK
            }
            Ok(Ok(None)) => EMPTY,
            result => {
                // Keep executable code resident. A failure never authorizes unload.
                let _ = SESSION.try_with(|cell| {
                    if let Ok(state) = cell.try_borrow() {
                        if let Some(session) = state.as_ref() {
                            session.stats.cancel_capture();
                        }
                    }
                });
                if result.is_err() {
                    PANICKED
                } else {
                    REJECTED
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn abi_rejects_version_lengths_and_null_buffers_before_dereference() {
            unsafe {
                for (version, length) in [(VERSION + 1, PACKET_SIZE), (VERSION, PACKET_SIZE - 1)] {
                    assert_eq!(
                        silk_capture_helper_dispatch(
                            version,
                            ATTACH,
                            std::ptr::null_mut(),
                            std::ptr::null(),
                            length,
                            std::ptr::null_mut(),
                            PACKET_SIZE
                        ),
                        REJECTED
                    );
                }
                assert_eq!(
                    silk_capture_helper_dispatch(
                        VERSION,
                        ATTACH,
                        std::ptr::null_mut(),
                        std::ptr::null(),
                        PACKET_SIZE,
                        std::ptr::null_mut(),
                        PACKET_SIZE
                    ),
                    REJECTED
                );
                let request = Message::Stop.encode();
                let mut response = [0; PACKET_SIZE];
                assert_eq!(
                    silk_capture_helper_dispatch(
                        VERSION,
                        ATTACH,
                        std::ptr::null_mut(),
                        request.as_ptr(),
                        PACKET_SIZE,
                        response.as_mut_ptr(),
                        PACKET_SIZE
                    ),
                    REJECTED,
                    "non-fixture test process must not be opted in"
                );
            }
        }
    }
}
