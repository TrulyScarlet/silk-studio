//! Development-only contract and controlled D3D11 hook experiment (ADR 0046).
//! Normal DLL loading is exact owned-fixture-only; no production hook backend.

use std::sync::{Arc, Weak};

pub const SLOT_COUNT: usize = 3;
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_WIDTH: u32 = 7680;
pub const MAX_HEIGHT: u32 = 4320;

pub mod admission_gate;
pub mod automatic_resize_gate;
#[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-late-resize-fixture"))]
pub mod d3d12_late_resize;
pub mod delivery_gate;
pub mod device_loss_epoch;
pub mod helper_abi;
pub mod late_queue_contract;
pub mod publication_pool;
pub mod timeline;
pub mod transport;

#[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-bootstrap-fixture"))]
pub mod d3d12_bootstrap;
#[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-cross-process"))]
pub mod d3d12_cross_capture;
#[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-cross-process"))]
pub mod d3d12_cross_process;
#[cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "d3d12-factory-hook-fixture"
))]
pub mod d3d12_factory_hook;
#[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-fixture"))]
pub mod d3d12_fixture;
#[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-gpu-fixture"))]
pub mod d3d12_gpu_bridge;
#[cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "d3d12-present-hook-fixture"
))]
mod d3d12_owned_hook;
#[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-fixture"))]
pub mod d3d12_present_fixture;
pub mod d3d12_protocol;
#[cfg(all(windows, target_arch = "x86_64", feature = "d3d12-fixture"))]
mod d3d12_queue_owner;
pub mod d3d12_queue_registry;
pub mod fence_slots;
pub mod resize_epoch;
pub mod shared_fence_slots;

#[cfg(all(windows, target_arch = "x86_64", feature = "experimental-hooks"))]
pub mod native;

#[cfg(all(windows, target_arch = "x86_64", feature = "resident-hooks"))]
pub mod owned_attachment;
#[cfg(all(windows, target_arch = "x86_64", feature = "resident-hooks"))]
pub mod resident;

#[cfg(all(windows, target_arch = "x86_64", feature = "fixture-probes"))]
pub mod fixture_probe;

#[cfg(all(windows, target_arch = "x86_64", feature = "experimental-hooks"))]
pub mod shared_transport;

#[cfg(all(windows, target_arch = "x86_64", feature = "experimental-hooks"))]
pub mod cooperative_peer;

#[cfg(all(windows, target_arch = "x86_64", feature = "experimental-hooks"))]
pub mod controlled_capture;

#[cfg(all(
    windows,
    target_arch = "x86_64",
    any(feature = "experimental-hooks", feature = "d3d12-gpu-fixture")
))]
pub mod local_gpu_bridge;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetIdentity {
    pub pid: u32,
    pub creation_time: u64,
    pub window: u64,
}

impl TargetIdentity {
    pub fn is_valid(self) -> bool {
        self.pid != 0 && self.creation_time != 0 && self.window != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionIdentity {
    pub target: TargetIdentity,
    /// Locally issued, nonzero session generation. Not an authentication secret.
    pub generation: u64,
    pub epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub version: u32,
    pub session: SessionIdentity,
    pub slot: u32,
    pub sequence: u64,
    pub qpc_ticks: i64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectedFrame {
    Closed,
    Version,
    Identity,
    Dimensions,
    Slot,
    Sequence,
    Timestamp,
}

/// Metadata gate for a single approved session. Rejections never advance state.
/// This is validation, not a serialized ABI or an authenticated IPC handshake.
pub struct SessionGate {
    identity: SessionIdentity,
    last: Option<(u64, i64)>,
    open: bool,
}

impl SessionGate {
    pub fn new(identity: SessionIdentity) -> Option<Self> {
        (identity.target.is_valid() && identity.generation != 0 && identity.epoch != 0).then_some(
            Self {
                identity,
                last: None,
                open: true,
            },
        )
    }

    pub fn accept(&mut self, header: FrameHeader) -> Result<(), RejectedFrame> {
        if !self.open {
            return Err(RejectedFrame::Closed);
        }
        if header.version != PROTOCOL_VERSION {
            return Err(RejectedFrame::Version);
        }
        if header.session != self.identity {
            return Err(RejectedFrame::Identity);
        }
        if header.width == 0
            || header.height == 0
            || header.width > MAX_WIDTH
            || header.height > MAX_HEIGHT
        {
            return Err(RejectedFrame::Dimensions);
        }
        if header.slot >= SLOT_COUNT as u32 {
            return Err(RejectedFrame::Slot);
        }
        if header.sequence == 0
            || self
                .last
                .is_some_and(|(sequence, _)| header.sequence <= sequence)
        {
            return Err(RejectedFrame::Sequence);
        }
        if header.qpc_ticks < 0 || self.last.is_some_and(|(_, ticks)| header.qpc_ticks < ticks) {
            return Err(RejectedFrame::Timestamp);
        }
        self.last = Some((header.sequence, header.qpc_ticks));
        Ok(())
    }

    pub fn close(&mut self) {
        self.open = false;
    }
}

/// A fixed producer-owned ring. Only the producer calls acquire; consumers hold
/// Arc leases. No mutex, allocation, or slot replacement occurs during acquire.
pub struct FrameSlots<T> {
    slots: [Arc<T>; SLOT_COUNT],
    cursor: usize,
}

impl<T> FrameSlots<T> {
    pub fn new(slots: [T; SLOT_COUNT]) -> Self {
        Self {
            slots: slots.map(Arc::new),
            cursor: 0,
        }
    }

    pub fn try_acquire(&mut self) -> Option<(usize, Arc<T>)> {
        for offset in 0..SLOT_COUNT {
            let slot = (self.cursor + offset) % SLOT_COUNT;
            if Arc::strong_count(&self.slots[slot]) == 1 && Arc::weak_count(&self.slots[slot]) == 0
            {
                self.cursor = (slot + 1) % SLOT_COUNT;
                return Some((slot, Arc::clone(&self.slots[slot])));
            }
        }
        None
    }

    /// Called at teardown, never from Present. Tokens let the owner bound GPU
    /// memory still retained by consumers from older capture epochs.
    pub fn retirement_tokens(&self) -> [Weak<T>; SLOT_COUNT] {
        std::array::from_fn(|slot| Arc::downgrade(&self.slots[slot]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> FrameHeader {
        FrameHeader {
            version: PROTOCOL_VERSION,
            session: SessionIdentity {
                target: TargetIdentity {
                    pid: 42,
                    creation_time: 123,
                    window: 456,
                },
                generation: 1,
                epoch: 1,
            },
            slot: 0,
            sequence: 1,
            qpc_ticks: 100,
            width: 1440,
            height: 1080,
        }
    }

    #[test]
    fn retained_frames_cannot_be_overwritten_and_memory_is_bounded() {
        let mut slots = FrameSlots::new([10, 20, 30]);
        let a = slots.try_acquire().unwrap();
        let b = slots.try_acquire().unwrap();
        let c = slots.try_acquire().unwrap();
        for _ in 0..10_000 {
            assert!(slots.try_acquire().is_none());
        }
        assert_eq!(*a.1, 10);
        let released = b.0;
        drop(b);
        assert_eq!(slots.try_acquire().unwrap().0, released);
        drop(slots);
        assert_eq!(*a.1, 10);
        assert_eq!(*c.1, 30);
    }

    #[test]
    fn changed_process_creation_time_generation_or_epoch_is_rejected() {
        let valid = frame();
        for changed in [
            FrameHeader {
                session: SessionIdentity {
                    target: TargetIdentity {
                        creation_time: 124,
                        ..valid.session.target
                    },
                    ..valid.session
                },
                ..valid
            },
            FrameHeader {
                session: SessionIdentity {
                    generation: 2,
                    ..valid.session
                },
                ..valid
            },
            FrameHeader {
                session: SessionIdentity {
                    epoch: 2,
                    ..valid.session
                },
                ..valid
            },
            FrameHeader {
                session: SessionIdentity {
                    target: TargetIdentity {
                        pid: 43,
                        ..valid.session.target
                    },
                    ..valid.session
                },
                ..valid
            },
            FrameHeader {
                session: SessionIdentity {
                    target: TargetIdentity {
                        window: 457,
                        ..valid.session.target
                    },
                    ..valid.session
                },
                ..valid
            },
        ] {
            let mut gate = SessionGate::new(valid.session).unwrap();
            assert_eq!(gate.accept(changed), Err(RejectedFrame::Identity));
            assert_eq!(gate.accept(valid), Ok(()));
        }
    }

    #[test]
    fn sequences_timestamps_bounds_and_disconnect_fail_closed() {
        let valid = frame();
        let mut gate = SessionGate::new(valid.session).unwrap();
        for (header, error) in [
            (
                FrameHeader {
                    version: 2,
                    ..valid
                },
                RejectedFrame::Version,
            ),
            (FrameHeader { width: 0, ..valid }, RejectedFrame::Dimensions),
            (
                FrameHeader {
                    height: MAX_HEIGHT + 1,
                    ..valid
                },
                RejectedFrame::Dimensions,
            ),
            (
                FrameHeader {
                    width: MAX_WIDTH + 1,
                    ..valid
                },
                RejectedFrame::Dimensions,
            ),
            (FrameHeader { slot: 3, ..valid }, RejectedFrame::Slot),
            (
                FrameHeader {
                    sequence: 0,
                    ..valid
                },
                RejectedFrame::Sequence,
            ),
            (
                FrameHeader {
                    qpc_ticks: -1,
                    ..valid
                },
                RejectedFrame::Timestamp,
            ),
        ] {
            assert_eq!(gate.accept(header), Err(error));
        }
        assert_eq!(gate.accept(valid), Ok(()));
        assert_eq!(gate.accept(valid), Err(RejectedFrame::Sequence));
        assert_eq!(
            gate.accept(FrameHeader {
                sequence: 3,
                qpc_ticks: 99,
                ..valid
            }),
            Err(RejectedFrame::Timestamp)
        );
        assert_eq!(
            gate.accept(FrameHeader {
                sequence: 3,
                qpc_ticks: 100,
                ..valid
            }),
            Ok(())
        );
        gate.close();
        assert_eq!(
            gate.accept(FrameHeader {
                sequence: 4,
                ..valid
            }),
            Err(RejectedFrame::Closed)
        );
    }

    #[test]
    fn invalid_identity_never_starts_a_session() {
        let valid = frame().session;
        assert!(SessionGate::new(SessionIdentity {
            generation: 0,
            ..valid
        })
        .is_none());
        assert!(SessionGate::new(SessionIdentity { epoch: 0, ..valid }).is_none());
        assert!(SessionGate::new(SessionIdentity {
            target: TargetIdentity {
                creation_time: 0,
                ..valid.target
            },
            ..valid
        })
        .is_none());
    }

    #[test]
    fn weak_leases_cannot_resurrect_a_surface_during_reuse() {
        let mut slots = FrameSlots::new([10, 20, 30]);
        let (_, a) = slots.try_acquire().unwrap();
        let weak = Arc::downgrade(&a);
        let b = slots.try_acquire().unwrap();
        let c = slots.try_acquire().unwrap();
        drop(a);
        assert!(slots.try_acquire().is_none());
        assert_eq!(*weak.upgrade().unwrap(), 10);
        drop(weak);
        assert_eq!(slots.try_acquire().unwrap().0, 0);
        drop((b, c));
    }
}
