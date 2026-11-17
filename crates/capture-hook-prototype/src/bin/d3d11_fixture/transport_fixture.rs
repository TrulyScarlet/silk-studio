//! Private-pipe, cooperative parent/child test harness; no game attachment.

use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use capture_hook_prototype::cooperative_peer::{
    read_message as read, write_message as send, FixturePeer as Peer,
};

use capture_hook_prototype::native::{own_window_identity, PresentTap};
use capture_hook_prototype::shared_transport::{ConsumerRing, ImportedFrame, ProducerRing};
use capture_hook_prototype::transport::{Message, PACKET_SIZE};
use capture_hook_prototype::{FrameHeader, SessionIdentity, TargetIdentity, PROTOCOL_VERSION};
use gpu_windows::{BgraReadback, GpuDeviceContext};
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use windows::Win32::System::Threading::WaitForSingleObject;

pub fn producer(
    chain: &IDXGISwapChain,
    mut render: impl FnMut([f32; 4]) -> Result<(), String>,
) -> Result<(), String> {
    let description = unsafe { chain.GetDesc() }.map_err(|error| error.to_string())?;
    let session = SessionIdentity {
        target: own_window_identity(description.OutputWindow)?,
        generation: 1,
        epoch: 1,
    };
    // Only this original renderer calls Present, synchronously on this thread.
    let (mut tap, frames, _) = unsafe { PresentTap::attach(chain, session) }?;
    let device: ID3D11Device = unsafe { chain.GetDevice() }.map_err(|error| error.to_string())?;
    let context = unsafe { device.GetImmediateContext() }.map_err(|error| error.to_string())?;
    let device = Arc::new(GpuDeviceContext::new(device, context));
    let mut ring = ProducerRing::new(
        device,
        session,
        description.BufferDesc.Width,
        description.BufferDesc.Height,
    )?;
    let mut output = std::io::stdout().lock();
    let mut input = std::io::stdin().lock();
    send(&mut output, Message::Hello(ring.hello()))?;
    let mut presented = 0;
    let mut published = 0;
    let mut dropped = 0;
    loop {
        match read(&mut input)? {
            Message::Render { count, color } => {
                let color = match color {
                    0 => [1.0, 0.0, 0.0, 1.0],
                    1 => [0.0, 0.0, 1.0, 1.0],
                    2 => [0.0, 1.0, 0.0, 1.0],
                    _ => [1.0, 1.0, 0.0, 1.0],
                };
                for _ in 0..count {
                    render(color)?;
                    presented += 1;
                    let frame = frames
                        .try_recv()
                        .map_err(|error| format!("hook stopped publishing: {error}"))?;
                    match ring.export(&frame)? {
                        Some(header) => {
                            send(&mut output, Message::Frame(header))?;
                            published += 1;
                        }
                        None => dropped += 1,
                    }
                }
                send(
                    &mut output,
                    Message::Report {
                        presented,
                        published,
                        dropped,
                    },
                )?;
            }
            Message::Ack {
                session,
                slot,
                sequence,
            } => ring.acknowledge(session, slot, sequence)?,
            Message::Stop => {
                tap.detach()?;
                send(&mut output, Message::Bye)?;
                return Ok(());
            }
            _ => return Err("unexpected consumer control message".into()),
        }
    }
}

fn pixels(frame: &ImportedFrame, expected: [u8; 4]) -> Result<(), String> {
    let bytes = BgraReadback::default()
        .read_scaled(frame.surface(), frame.header.width, frame.header.height)
        .map_err(|error| error.to_string())?;
    if !bytes.as_chunks::<4>().0.iter().all(|pixel| {
        pixel
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
    }) {
        return Err(format!("cross-process pixels did not match {expected:?}"));
    }
    Ok(())
}
fn ack(peer: &mut Peer, frame: ImportedFrame) -> Result<(), String> {
    let header = frame.release()?;
    peer.send(Message::Ack {
        session: header.session,
        slot: header.slot,
        sequence: header.sequence,
    })
}
type RenderBatch = (Vec<ImportedFrame>, (u64, u64, u64));

fn render(
    peer: &mut Peer,
    ring: &mut ConsumerRing,
    color: u32,
    count: u32,
) -> Result<RenderBatch, String> {
    peer.send(Message::Render { count, color })?;
    let mut frames = Vec::with_capacity(3);
    loop {
        match peer.receive(Duration::from_secs(5))? {
            Message::Frame(header) => {
                if frames.len() >= 3 {
                    return Err("peer exceeded bounded frame count".into());
                }
                frames.push(ring.receive(header)?);
            }
            Message::Report {
                presented,
                published,
                dropped,
            } => return Ok((frames, (presented, published, dropped))),
            _ => return Err("unexpected render response".into()),
        }
    }
}

fn scenario(warp: bool, crash: bool) -> Result<(), String> {
    let mut peer = Peer::launch(warp, None)?;
    let hello = match peer.receive(Duration::from_secs(10))? {
        Message::Hello(hello) => hello,
        _ => return Err("producer did not greet before sending frames".into()),
    };
    if !crash {
        for changed in [
            capture_hook_prototype::transport::Hello {
                session: SessionIdentity {
                    target: TargetIdentity {
                        pid: hello.session.target.pid + 1,
                        ..hello.session.target
                    },
                    ..hello.session
                },
                ..hello
            },
            capture_hook_prototype::transport::Hello {
                session: SessionIdentity {
                    target: TargetIdentity {
                        creation_time: hello.session.target.creation_time + 1,
                        ..hello.session.target
                    },
                    ..hello.session
                },
                ..hello
            },
            capture_hook_prototype::transport::Hello {
                adapter_luid: hello.adapter_luid ^ 1,
                ..hello
            },
            capture_hook_prototype::transport::Hello {
                qpc_frequency: hello.qpc_frequency + 1,
                ..hello
            },
            capture_hook_prototype::transport::Hello {
                width: hello.width + 2,
                ..hello
            },
        ] {
            if unsafe { ConsumerRing::from_child(peer.handle(), peer.pid(), changed, warp) }.is_ok()
            {
                return Err(
                    "a forged producer identity/adapter/clock/descriptor was accepted".into(),
                );
            }
        }
    }
    // Borrow the exact process handle from std::process::Child; never reopen by PID.
    let mut ring = unsafe { ConsumerRing::from_child(peer.handle(), peer.pid(), hello, warp) }?;
    let mut held = Vec::with_capacity(3);
    for (color, expected) in [
        (0, [0, 0, 255, 255]),
        (1, [255, 0, 0, 255]),
        (2, [0, 255, 0, 255]),
    ] {
        let (mut frames, _) = render(&mut peer, &mut ring, color, 1)?;
        if frames.len() != 1 {
            return Err("producer failed to fill a free shared slot".into());
        }
        let frame = frames.remove(0);
        pixels(&frame, expected)?;
        held.push(frame);
    }
    let (frames, report) = render(&mut peer, &mut ring, 3, 32)?;
    if !frames.is_empty() || report != (35, 3, 32) {
        return Err(format!("shared-ring backpressure failed: {report:?}"));
    }
    pixels(&held[0], [0, 0, 255, 255])?;
    let blue = held.remove(1);
    let released_slot = blue.header.slot;
    ack(&mut peer, blue)?;
    let (mut frames, report) = render(&mut peer, &mut ring, 3, 1)?;
    if frames.len() != 1 || report != (36, 4, 32) || frames[0].header.slot != released_slot {
        return Err("acknowledged slot was not reused correctly".into());
    }
    let yellow = frames.remove(0);
    pixels(&yellow, [0, 255, 255, 255])?;
    pixels(&held[0], [0, 0, 255, 255])?;
    ack(&mut peer, yellow)?;
    let green = held.pop().ok_or("missing retained green frame")?;
    ack(&mut peer, green)?;
    if crash {
        peer.kill()?;
        if unsafe { WaitForSingleObject(peer.handle(), 5000) } != WAIT_OBJECT_0 {
            return Err("crashed producer did not exit".into());
        }
        let _ = peer.wait_status(Duration::from_secs(5));
    } else {
        peer.send(Message::Stop)?;
        if peer.receive(Duration::from_secs(5))? != Message::Bye {
            return Err("producer did not acknowledge shutdown".into());
        }
        peer.expect_exit()?;
    }
    // The consumer's duplicated/opened resource remains valid after the
    // producer exits, even while the consumer retains key-1 ownership.
    pixels(&held[0], [0, 0, 255, 255])?;
    held.pop().ok_or("missing retained red frame")?.release()?;
    Ok(())
}

pub fn fault(mode: &str) -> Result<(), String> {
    match mode {
        "eof" => Ok(()),
        "malformed" => std::io::stdout()
            .write_all(&[0; PACKET_SIZE])
            .map_err(|error| error.to_string()),
        "truncated" => std::io::stdout()
            .write_all(&[0; 16])
            .map_err(|error| error.to_string()),
        "stall" => {
            std::thread::sleep(Duration::from_secs(30));
            Ok(())
        }
        "partial-stall" => {
            std::io::stdout()
                .write_all(&[0; 16])
                .map_err(|error| error.to_string())?;
            std::io::stdout()
                .flush()
                .map_err(|error| error.to_string())?;
            std::thread::sleep(Duration::from_secs(30));
            Ok(())
        }
        _ => Err("unknown controlled transport fault".into()),
    }
}

pub fn parent(warp: bool) -> Result<(), String> {
    scenario(warp, false)?;
    scenario(warp, true)?;
    for premature in [false, true] {
        let mut peer = Peer::launch(warp, Some("bad-ack"))?;
        let hello = match peer.receive(Duration::from_secs(10))? {
            Message::Hello(hello) => hello,
            _ => return Err("stale-ACK fixture missing greeting".into()),
        };
        let mut ring = unsafe { ConsumerRing::from_child(peer.handle(), peer.pid(), hello, warp) }?;
        let (mut frames, _) = render(&mut peer, &mut ring, 0, 1)?;
        let frame = frames.pop().ok_or("stale-ACK fixture missing frame")?;
        peer.send(Message::Ack {
            session: frame.header.session,
            slot: frame.header.slot,
            sequence: frame.header.sequence + u64::from(!premature),
        })?;
        if premature {
            // Even an exact ACK cannot permit an overwrite while the consumer
            // still owns key 1. Producer's zero-timeout acquire must fail closed.
            peer.send(Message::Render { count: 1, color: 1 })?;
        }
        if peer.receive(Duration::from_secs(5)).is_ok() {
            return Err("producer accepted a stale or premature ACK".into());
        }
        if unsafe { WaitForSingleObject(peer.handle(), 5000) } != WAIT_OBJECT_0 {
            return Err("producer did not terminate invalid-ACK session".into());
        }
        if peer.wait_status(Duration::from_secs(5))?.success() {
            return Err("invalid ACK did not fail the producer session".into());
        }
        pixels(&frame, [0, 0, 255, 255])?;
    }
    // A valid-looking header must not turn WAIT_TIMEOUT into a texture lease.
    {
        let mut peer = Peer::launch(warp, None)?;
        let hello = match peer.receive(Duration::from_secs(10))? {
            Message::Hello(hello) => hello,
            _ => return Err("timeout fixture missing greeting".into()),
        };
        let mut ring = unsafe { ConsumerRing::from_child(peer.handle(), peer.pid(), hello, warp) }?;
        let header = FrameHeader {
            version: PROTOCOL_VERSION,
            session: hello.session,
            slot: 0,
            sequence: 1,
            qpc_ticks: 0,
            width: hello.width,
            height: hello.height,
        };
        match ring.receive(header) {
            Err(error) if error.contains("0x102") => {}
            _ => {
                return Err("keyed-mutex timeout was incorrectly accepted or misclassified".into())
            }
        }
        if ring
            .receive(FrameHeader {
                sequence: 2,
                ..header
            })
            .is_ok()
        {
            return Err(
                "consumer recovered from a fatal ownership failure without a new session".into(),
            );
        }
        peer.send(Message::Stop)?;
        if peer.receive(Duration::from_secs(5))? != Message::Bye {
            return Err("timeout producer did not stop".into());
        }
        peer.expect_exit()?;
    }
    for fault in ["eof", "malformed", "truncated", "stall", "partial-stall"] {
        let peer = Peer::launch(warp, Some(fault))?;
        if peer.receive(Duration::from_millis(750)).is_ok() {
            return Err(format!("{fault} peer was accepted"));
        }
        // Drop kills/waits for this exact child and joins its pipe-reader.
        drop(peer);
    }
    println!("PASS: cross-process NT-handle D3D11 pixels; adapter/PID/creation-time/window binding; 3-slot drops and exact ACK reuse; stale ACK and keyed-mutex timeout rejection; retained resource after exit/crash; EOF/malformed/truncated/stalled peer cleanup. driver={}",if warp {"WARP"} else {"hardware"});
    Ok(())
}
