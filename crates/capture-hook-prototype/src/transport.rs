//! Fixed-size cooperative fixture protocol. Not an injected-helper ABI.

use crate::{
    FrameHeader, SessionGate, SessionIdentity, TargetIdentity, MAX_HEIGHT, MAX_WIDTH,
    PROTOCOL_VERSION, SLOT_COUNT,
};

pub const PACKET_SIZE: usize = 128;
pub const MAX_RENDER_COUNT: u32 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hello {
    pub session: SessionIdentity,
    pub adapter_luid: u64,
    pub qpc_frequency: u64,
    pub width: u32,
    pub height: u32,
    /// Values in the producer process. Duplicate from the verified process handle.
    pub handles: [u64; SLOT_COUNT],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    Hello(Hello),
    Frame(FrameHeader),
    Ack {
        session: SessionIdentity,
        slot: u32,
        sequence: u64,
    },
    Render {
        count: u32,
        color: u32,
    },
    Report {
        presented: u64,
        published: u64,
        dropped: u64,
    },
    Stop,
    Bye,
    /// Cooperative helper offer/approval, before any hook is installed.
    Select(SessionIdentity),
    /// Owned fixture only: detach, resize/recreate, offer a fresh epoch.
    Rearm {
        epoch: u64,
        width: u32,
        height: u32,
        recreate: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    Length,
    Magic,
    Version,
    Kind,
    Reserved,
    Fields,
    Identity,
    Busy,
    Stale,
    Closed,
}

struct Writer {
    bytes: [u8; PACKET_SIZE],
    at: usize,
}
impl Writer {
    fn u32(&mut self, value: u32) {
        self.bytes[self.at..self.at + 4].copy_from_slice(&value.to_le_bytes());
        self.at += 4;
    }
    fn u64(&mut self, value: u64) {
        self.bytes[self.at..self.at + 8].copy_from_slice(&value.to_le_bytes());
        self.at += 8;
    }
    fn session(&mut self, session: SessionIdentity) {
        self.u32(session.target.pid);
        self.u64(session.target.creation_time);
        self.u64(session.target.window);
        self.u64(session.generation);
        self.u64(session.epoch);
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn u32(&mut self) -> u32 {
        let value = u32::from_le_bytes(
            self.bytes[self.at..self.at + 4]
                .try_into()
                .expect("fixed packet layout"),
        );
        self.at += 4;
        value
    }
    fn u64(&mut self) -> u64 {
        let value = u64::from_le_bytes(
            self.bytes[self.at..self.at + 8]
                .try_into()
                .expect("fixed packet layout"),
        );
        self.at += 8;
        value
    }
    fn session(&mut self) -> SessionIdentity {
        SessionIdentity {
            target: TargetIdentity {
                pid: self.u32(),
                creation_time: self.u64(),
                window: self.u64(),
            },
            generation: self.u64(),
            epoch: self.u64(),
        }
    }
}

impl Message {
    pub fn encode(self) -> [u8; PACKET_SIZE] {
        let mut writer = Writer {
            bytes: [0; PACKET_SIZE],
            at: 0,
        };
        writer.bytes[..4].copy_from_slice(b"SKH1");
        writer.at = 4;
        writer.u32(match self {
            Self::Frame(frame) => frame.version,
            _ => PROTOCOL_VERSION,
        });
        writer.u32(match self {
            Self::Hello(_) => 1,
            Self::Frame(_) => 2,
            Self::Ack { .. } => 3,
            Self::Render { .. } => 4,
            Self::Report { .. } => 5,
            Self::Stop => 6,
            Self::Bye => 7,
            Self::Select(_) => 8,
            Self::Rearm { .. } => 9,
        });
        match self {
            Self::Hello(hello) => {
                writer.session(hello.session);
                writer.u64(hello.adapter_luid);
                writer.u64(hello.qpc_frequency);
                writer.u32(hello.width);
                writer.u32(hello.height);
                for handle in hello.handles {
                    writer.u64(handle);
                }
            }
            Self::Frame(frame) => {
                writer.session(frame.session);
                writer.u32(frame.slot);
                writer.u64(frame.sequence);
                writer.u64(frame.qpc_ticks as u64);
                writer.u32(frame.width);
                writer.u32(frame.height);
            }
            Self::Ack {
                session,
                slot,
                sequence,
            } => {
                writer.session(session);
                writer.u32(slot);
                writer.u64(sequence);
            }
            Self::Render { count, color } => {
                writer.u32(count);
                writer.u32(color);
            }
            Self::Report {
                presented,
                published,
                dropped,
            } => {
                writer.u64(presented);
                writer.u64(published);
                writer.u64(dropped);
            }
            Self::Stop | Self::Bye => {}
            Self::Select(session) => writer.session(session),
            Self::Rearm {
                epoch,
                width,
                height,
                recreate,
            } => {
                writer.u64(epoch);
                writer.u32(width);
                writer.u32(height);
                writer.u32(u32::from(recreate));
            }
        }
        writer.bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != PACKET_SIZE {
            return Err(ProtocolError::Length);
        }
        if &bytes[..4] != b"SKH1" {
            return Err(ProtocolError::Magic);
        }
        let mut reader = Reader { bytes, at: 4 };
        if reader.u32() != PROTOCOL_VERSION {
            return Err(ProtocolError::Version);
        }
        let message = match reader.u32() {
            1 => {
                let hello = Hello {
                    session: reader.session(),
                    adapter_luid: reader.u64(),
                    qpc_frequency: reader.u64(),
                    width: reader.u32(),
                    height: reader.u32(),
                    handles: std::array::from_fn(|_| reader.u64()),
                };
                if SessionGate::new(hello.session).is_none()
                    || hello.qpc_frequency == 0
                    || hello.qpc_frequency > i64::MAX as u64
                    || hello.width == 0
                    || hello.height == 0
                    || hello.width > MAX_WIDTH
                    || hello.height > MAX_HEIGHT
                    || hello
                        .handles
                        .iter()
                        .any(|handle| *handle == 0 || *handle > i64::MAX as u64)
                    || hello.handles[0] == hello.handles[1]
                    || hello.handles[0] == hello.handles[2]
                    || hello.handles[1] == hello.handles[2]
                {
                    return Err(ProtocolError::Fields);
                }
                Self::Hello(hello)
            }
            2 => {
                let frame = FrameHeader {
                    version: PROTOCOL_VERSION,
                    session: reader.session(),
                    slot: reader.u32(),
                    sequence: reader.u64(),
                    qpc_ticks: reader.u64() as i64,
                    width: reader.u32(),
                    height: reader.u32(),
                };
                let mut gate = SessionGate::new(frame.session).ok_or(ProtocolError::Fields)?;
                gate.accept(frame).map_err(|_| ProtocolError::Fields)?;
                Self::Frame(frame)
            }
            3 => {
                let session = reader.session();
                let slot = reader.u32();
                let sequence = reader.u64();
                if SessionGate::new(session).is_none() || slot >= SLOT_COUNT as u32 || sequence == 0
                {
                    return Err(ProtocolError::Fields);
                }
                Self::Ack {
                    session,
                    slot,
                    sequence,
                }
            }
            4 => {
                let count = reader.u32();
                let color = reader.u32();
                if count == 0 || count > MAX_RENDER_COUNT || color > 3 {
                    return Err(ProtocolError::Fields);
                }
                Self::Render { count, color }
            }
            5 => Self::Report {
                presented: reader.u64(),
                published: reader.u64(),
                dropped: reader.u64(),
            },
            6 => Self::Stop,
            7 => Self::Bye,
            8 => {
                let session = reader.session();
                if SessionGate::new(session).is_none() {
                    return Err(ProtocolError::Fields);
                }
                Self::Select(session)
            }
            9 => {
                let epoch = reader.u64();
                let width = reader.u32();
                let height = reader.u32();
                let recreate = reader.u32();
                if epoch == 0
                    || width == 0
                    || height == 0
                    || width > MAX_WIDTH
                    || height > MAX_HEIGHT
                    || recreate > 1
                {
                    return Err(ProtocolError::Fields);
                }
                Self::Rearm {
                    epoch,
                    width,
                    height,
                    recreate: recreate != 0,
                }
            }
            _ => return Err(ProtocolError::Kind),
        };
        if bytes[reader.at..].iter().any(|byte| *byte != 0) {
            return Err(ProtocolError::Reserved);
        }
        Ok(message)
    }
}

/// Producer ownership ledger. Only a matching ACK frees an in-flight slot.
pub struct SlotLedger {
    session: SessionIdentity,
    gate: SessionGate,
    pending: [Option<u64>; SLOT_COUNT],
    closed: bool,
}
impl SlotLedger {
    pub fn new(session: SessionIdentity) -> Option<Self> {
        Some(Self {
            session,
            gate: SessionGate::new(session)?,
            pending: [None; SLOT_COUNT],
            closed: false,
        })
    }
    pub fn available(&self) -> Option<usize> {
        (!self.closed)
            .then(|| self.pending.iter().position(Option::is_none))
            .flatten()
    }
    pub fn publish(&mut self, frame: FrameHeader) -> Result<(), ProtocolError> {
        if self.closed {
            return Err(ProtocolError::Closed);
        }
        if frame.session != self.session {
            self.close();
            return Err(ProtocolError::Identity);
        }
        if frame.slot >= SLOT_COUNT as u32 {
            self.close();
            return Err(ProtocolError::Fields);
        }
        if self.pending[frame.slot as usize].is_some() {
            return Err(ProtocolError::Busy);
        }
        self.gate.accept(frame).map_err(|_| {
            self.closed = true;
            ProtocolError::Fields
        })?;
        self.pending[frame.slot as usize] = Some(frame.sequence);
        Ok(())
    }
    pub fn acknowledge(
        &mut self,
        session: SessionIdentity,
        slot: u32,
        sequence: u64,
    ) -> Result<(), ProtocolError> {
        if self.closed {
            return Err(ProtocolError::Closed);
        }
        if session != self.session {
            self.close();
            return Err(ProtocolError::Identity);
        }
        if slot >= SLOT_COUNT as u32 || self.pending[slot as usize] != Some(sequence) {
            self.close();
            return Err(ProtocolError::Stale);
        }
        self.pending[slot as usize] = None;
        Ok(())
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.gate.close();
    }
}

/// The generated Windows Result<()> binding treats positive timeout/abandoned
/// HRESULTs as success. Native callers must compare the raw result to S_OK.
pub fn keyed_mutex_acquired(raw_hresult: i32) -> bool {
    raw_hresult == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> SessionIdentity {
        SessionIdentity {
            target: TargetIdentity {
                pid: 7,
                creation_time: 123,
                window: 456,
            },
            generation: 8,
            epoch: 9,
        }
    }
    fn frame(slot: u32, sequence: u64) -> FrameHeader {
        FrameHeader {
            version: PROTOCOL_VERSION,
            session: session(),
            slot,
            sequence,
            qpc_ticks: sequence as i64,
            width: 640,
            height: 480,
        }
    }
    #[test]
    fn all_messages_round_trip_and_truncations_fail() {
        for message in [
            Message::Hello(Hello {
                session: session(),
                adapter_luid: 111,
                qpc_frequency: 10_000_000,
                width: 640,
                height: 480,
                handles: [4, 8, 12],
            }),
            Message::Frame(frame(0, 1)),
            Message::Ack {
                session: session(),
                slot: 1,
                sequence: 2,
            },
            Message::Render {
                count: 120,
                color: 3,
            },
            Message::Report {
                presented: 10,
                published: 3,
                dropped: 7,
            },
            Message::Stop,
            Message::Bye,
            Message::Select(session()),
            Message::Rearm {
                epoch: 10,
                width: 800,
                height: 600,
                recreate: false,
            },
            Message::Rearm {
                epoch: 11,
                width: 960,
                height: 540,
                recreate: true,
            },
        ] {
            let encoded = message.encode();
            assert_eq!(Message::decode(&encoded), Ok(message));
            for length in 0..PACKET_SIZE {
                assert_eq!(
                    Message::decode(&encoded[..length]),
                    Err(ProtocolError::Length)
                );
            }
            let mut dirty = encoded;
            dirty[127] = 1;
            assert_eq!(Message::decode(&dirty), Err(ProtocolError::Reserved));
        }
    }
    #[test]
    fn malformed_packets_are_rejected_without_panicking() {
        let mut packet = Message::Stop.encode();
        packet[0] = 0;
        assert_eq!(Message::decode(&packet), Err(ProtocolError::Magic));
        packet = Message::Stop.encode();
        packet[4] = 99;
        assert_eq!(Message::decode(&packet), Err(ProtocolError::Version));
        packet = Message::Stop.encode();
        packet[8] = 99;
        assert_eq!(Message::decode(&packet), Err(ProtocolError::Kind));
        for message in [
            Message::Select(SessionIdentity {
                epoch: 0,
                ..session()
            }),
            Message::Rearm {
                epoch: 0,
                width: 800,
                height: 600,
                recreate: false,
            },
            Message::Rearm {
                epoch: 10,
                width: MAX_WIDTH + 1,
                height: 600,
                recreate: false,
            },
            Message::Rearm {
                epoch: 10,
                width: 800,
                height: 0,
                recreate: false,
            },
            Message::Render { count: 0, color: 0 },
            Message::Render {
                count: 121,
                color: 0,
            },
            Message::Render { count: 1, color: 4 },
            Message::Frame(frame(3, 1)),
            Message::Frame(FrameHeader {
                qpc_ticks: -1,
                ..frame(0, 1)
            }),
            Message::Hello(Hello {
                session: session(),
                adapter_luid: 1,
                qpc_frequency: 0,
                width: 640,
                height: 480,
                handles: [4, 8, 12],
            }),
            Message::Hello(Hello {
                session: session(),
                adapter_luid: 1,
                qpc_frequency: 1,
                width: 640,
                height: 480,
                handles: [4, 4, 12],
            }),
        ] {
            assert_eq!(
                Message::decode(&message.encode()),
                Err(ProtocolError::Fields)
            );
        }
        let mut state = 12345_u64;
        let mut packet = Message::Rearm {
            epoch: 10,
            width: 800,
            height: 600,
            recreate: false,
        }
        .encode();
        packet[28] = 2;
        assert_eq!(Message::decode(&packet), Err(ProtocolError::Fields));
        for _ in 0..1000 {
            let packet = std::array::from_fn::<_, PACKET_SIZE, _>(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            });
            assert!(Message::decode(&packet).is_err());
        }
    }
    #[test]
    fn bounded_slots_need_exact_acks_and_never_accept_replay() {
        let mut ledger = SlotLedger::new(session()).unwrap();
        for slot in 0..3 {
            ledger.publish(frame(slot, u64::from(slot) + 1)).unwrap();
        }
        assert_eq!(ledger.available(), None);
        assert_eq!(ledger.publish(frame(0, 4)), Err(ProtocolError::Busy));
        ledger.acknowledge(session(), 1, 2).unwrap();
        assert_eq!(ledger.available(), Some(1));
        ledger.publish(frame(1, 4)).unwrap();
        assert_eq!(
            ledger.acknowledge(session(), 1, 2),
            Err(ProtocolError::Stale)
        );
        assert_eq!(ledger.available(), None);
        assert_eq!(
            ledger.acknowledge(session(), 1, 4),
            Err(ProtocolError::Closed)
        );
    }
    #[test]
    fn changed_epoch_or_session_closes_transport() {
        let mut ledger = SlotLedger::new(session()).unwrap();
        ledger.publish(frame(0, 1)).unwrap();
        assert_eq!(
            ledger.acknowledge(
                SessionIdentity {
                    epoch: 10,
                    ..session()
                },
                0,
                1
            ),
            Err(ProtocolError::Identity)
        );
        assert_eq!(ledger.available(), None);
        let mut ledger = SlotLedger::new(session()).unwrap();
        ledger.close();
        assert_eq!(ledger.publish(frame(0, 1)), Err(ProtocolError::Closed));
    }
    #[test]
    fn positive_keyed_mutex_statuses_are_not_success() {
        assert!(keyed_mutex_acquired(0));
        for status in [0x102, 0x80, 1, -1, i32::MIN] {
            assert!(!keyed_mutex_acquired(status));
        }
    }
}
