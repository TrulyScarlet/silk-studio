//! Owned D3D12 child greeting; fixed-size validated control protocol, no IPC names.
use crate::transport::{Message, ProtocolError, PACKET_SIZE};
use crate::{SessionGate, SessionIdentity, TargetIdentity};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuHello {
    pub session: SessionIdentity,
    pub adapter_luid: u64,
    pub qpc_frequency: u64,
    pub width: u32,
    pub height: u32,
    pub handles: [u64; 5],
}
impl GpuHello {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if SessionGate::new(self.session).is_none()
            || self.adapter_luid == 0
            || self.qpc_frequency == 0
            || self.qpc_frequency > i64::MAX as u64
            || self.width == 0
            || self.height == 0
            || self.width > 1920
            || self.height > 1080
            || self
                .handles
                .iter()
                .any(|v| *v == 0 || *v > isize::MAX as u64)
            || self
                .handles
                .iter()
                .enumerate()
                .any(|(i, h)| self.handles[..i].contains(h))
        {
            return Err(ProtocolError::Fields);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    Hello(GpuHello),
    Control(Message),
    Window(WindowAction),
    Resize(ResizeRequest),
    ReplaceDevice(ResizeRequest),
    Retire(SessionIdentity),
    Retired(SessionIdentity),
    DeviceLost(DeviceLossNotice),
    TriggerOwnedLoss(SessionIdentity),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum DeviceLossReason {
    Removed = 0x887A0005,
    Hung = 0x887A0006,
    Reset = 0x887A0007,
    DriverInternal = 0x887A0020,
}
impl DeviceLossReason {
    pub fn from_hresult(value: u32) -> Option<Self> {
        match value {
            0x887A0005 => Some(Self::Removed),
            0x887A0006 => Some(Self::Hung),
            0x887A0007 => Some(Self::Reset),
            0x887A0020 => Some(Self::DriverInternal),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceLossNotice {
    pub session: SessionIdentity,
    pub reason: DeviceLossReason,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizeRequest {
    pub session: SessionIdentity,
    pub width: u32,
    pub height: u32,
}
impl ResizeRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if SessionGate::new(self.session).is_none()
            || self.width < 2
            || self.height < 2
            || self.width > 1920
            || self.height > 1080
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
        {
            return Err(ProtocolError::Fields);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum WindowAction {
    Minimize = 1,
    Restore = 2,
    Borderless = 3,
    Windowed = 4,
    ExcludeCapture = 5,
}
fn put<const N: usize>(bytes: &mut [u8], at: &mut usize, value: [u8; N]) {
    bytes[*at..*at + N].copy_from_slice(&value);
    *at += N;
}
fn get<const N: usize>(bytes: &[u8], at: &mut usize) -> [u8; N] {
    let value = bytes[*at..*at + N].try_into().unwrap();
    *at += N;
    value
}
impl Wire {
    pub fn encode(self) -> [u8; PACKET_SIZE] {
        if matches!(
            self,
            Self::Resize(_)
                | Self::ReplaceDevice(_)
                | Self::Retire(_)
                | Self::Retired(_)
                | Self::DeviceLost(_)
                | Self::TriggerOwnedLoss(_)
        ) {
            let (kind, session, width, height) = match self {
                Self::Resize(r) => (3u32, r.session, r.width, r.height),
                Self::ReplaceDevice(r) => (6u32, r.session, r.width, r.height),
                Self::Retire(s) => (4, s, 0, 0),
                Self::Retired(s) => (5, s, 0, 0),
                Self::DeviceLost(notice) => (7, notice.session, notice.reason as u32, 0),
                Self::TriggerOwnedLoss(session) => (8, session, 0, 0),
                _ => unreachable!(),
            };
            let mut bytes = [0; PACKET_SIZE];
            bytes[..4].copy_from_slice(b"SK12");
            let mut at = 4;
            put(&mut bytes, &mut at, 1u32.to_le_bytes());
            put(&mut bytes, &mut at, kind.to_le_bytes());
            put(&mut bytes, &mut at, session.target.pid.to_le_bytes());
            for value in [
                session.target.creation_time,
                session.target.window,
                session.generation,
                session.epoch,
            ] {
                put(&mut bytes, &mut at, value.to_le_bytes());
            }
            put(&mut bytes, &mut at, width.to_le_bytes());
            put(&mut bytes, &mut at, height.to_le_bytes());
            return bytes;
        }
        if let Self::Window(action) = self {
            let mut bytes = [0; PACKET_SIZE];
            bytes[..4].copy_from_slice(b"SK12");
            bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
            bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
            bytes[12..16].copy_from_slice(&(action as u32).to_le_bytes());
            return bytes;
        }
        let Self::Hello(h) = self else {
            let Self::Control(m) = self else {
                unreachable!()
            };
            return m.encode();
        };
        let mut bytes = [0; PACKET_SIZE];
        bytes[..4].copy_from_slice(b"SK12");
        let mut at = 4;
        put(&mut bytes, &mut at, 1u32.to_le_bytes());
        put(&mut bytes, &mut at, 1u32.to_le_bytes());
        put(&mut bytes, &mut at, h.session.target.pid.to_le_bytes());
        for v in [
            h.session.target.creation_time,
            h.session.target.window,
            h.session.generation,
            h.session.epoch,
            h.adapter_luid,
            h.qpc_frequency,
        ] {
            put(&mut bytes, &mut at, v.to_le_bytes());
        }
        put(&mut bytes, &mut at, h.width.to_le_bytes());
        put(&mut bytes, &mut at, h.height.to_le_bytes());
        for v in h.handles {
            put(&mut bytes, &mut at, v.to_le_bytes());
        }
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != PACKET_SIZE {
            return Err(ProtocolError::Length);
        }
        if &bytes[..4] != b"SK12" {
            let m = Message::decode(bytes)?;
            return if matches!(m, Message::Hello(_) | Message::Rearm { .. }) {
                Err(ProtocolError::Kind)
            } else {
                Ok(Self::Control(m))
            };
        }
        let mut at = 4;
        if u32::from_le_bytes(get(bytes, &mut at)) != 1 {
            return Err(ProtocolError::Version);
        }
        let kind = u32::from_le_bytes(get(bytes, &mut at));
        if (3..=8).contains(&kind) {
            let session = SessionIdentity {
                target: TargetIdentity {
                    pid: u32::from_le_bytes(get(bytes, &mut at)),
                    creation_time: u64::from_le_bytes(get(bytes, &mut at)),
                    window: u64::from_le_bytes(get(bytes, &mut at)),
                },
                generation: u64::from_le_bytes(get(bytes, &mut at)),
                epoch: u64::from_le_bytes(get(bytes, &mut at)),
            };
            let width = u32::from_le_bytes(get(bytes, &mut at));
            let height = u32::from_le_bytes(get(bytes, &mut at));
            if bytes[at..].iter().any(|v| *v != 0) {
                return Err(ProtocolError::Reserved);
            }
            if SessionGate::new(session).is_none() {
                return Err(ProtocolError::Fields);
            }
            if kind == 7 {
                if height != 0 {
                    return Err(ProtocolError::Reserved);
                }
                let reason = DeviceLossReason::from_hresult(width).ok_or(ProtocolError::Fields)?;
                return Ok(Self::DeviceLost(DeviceLossNotice { session, reason }));
            }
            if kind == 8 {
                if width != 0 || height != 0 {
                    return Err(ProtocolError::Reserved);
                }
                return Ok(Self::TriggerOwnedLoss(session));
            }
            return if kind == 3 || kind == 6 {
                let request = ResizeRequest {
                    session,
                    width,
                    height,
                };
                request.validate()?;
                Ok(if kind == 3 {
                    Self::Resize(request)
                } else {
                    Self::ReplaceDevice(request)
                })
            } else if width != 0 || height != 0 {
                Err(ProtocolError::Reserved)
            } else if kind == 4 {
                Ok(Self::Retire(session))
            } else {
                Ok(Self::Retired(session))
            };
        }
        if kind == 2 {
            let action = match u32::from_le_bytes(get(bytes, &mut at)) {
                1 => WindowAction::Minimize,
                2 => WindowAction::Restore,
                3 => WindowAction::Borderless,
                4 => WindowAction::Windowed,
                5 => WindowAction::ExcludeCapture,
                _ => return Err(ProtocolError::Fields),
            };
            if bytes[at..].iter().any(|v| *v != 0) {
                return Err(ProtocolError::Reserved);
            }
            return Ok(Self::Window(action));
        }
        if kind != 1 {
            return Err(ProtocolError::Kind);
        }
        let h = GpuHello {
            session: SessionIdentity {
                target: TargetIdentity {
                    pid: u32::from_le_bytes(get(bytes, &mut at)),
                    creation_time: u64::from_le_bytes(get(bytes, &mut at)),
                    window: u64::from_le_bytes(get(bytes, &mut at)),
                },
                generation: u64::from_le_bytes(get(bytes, &mut at)),
                epoch: u64::from_le_bytes(get(bytes, &mut at)),
            },
            adapter_luid: u64::from_le_bytes(get(bytes, &mut at)),
            qpc_frequency: u64::from_le_bytes(get(bytes, &mut at)),
            width: u32::from_le_bytes(get(bytes, &mut at)),
            height: u32::from_le_bytes(get(bytes, &mut at)),
            handles: std::array::from_fn(|_| u64::from_le_bytes(get(bytes, &mut at))),
        };
        if bytes[at..].iter().any(|v| *v != 0) {
            return Err(ProtocolError::Reserved);
        }
        h.validate()?;
        Ok(Self::Hello(h))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn hello() -> GpuHello {
        GpuHello {
            session: SessionIdentity {
                target: TargetIdentity {
                    pid: 1,
                    creation_time: 2,
                    window: 3,
                },
                generation: 1,
                epoch: 1,
            },
            adapter_luid: 4,
            qpc_frequency: 10_000_000,
            width: 318,
            height: 178,
            handles: [8, 12, 16, 20, 24],
        }
    }
    #[test]
    fn exact_roundtrip_and_truncation_validation() {
        for action in [
            WindowAction::Minimize,
            WindowAction::Restore,
            WindowAction::Borderless,
            WindowAction::Windowed,
            WindowAction::ExcludeCapture,
        ] {
            let wire = Wire::Window(action);
            assert_eq!(Wire::decode(&wire.encode()), Ok(wire));
        }
        let bytes = Wire::Hello(hello()).encode();
        assert_eq!(Wire::decode(&bytes), Ok(Wire::Hello(hello())));
        for n in 0..128 {
            assert!(Wire::decode(&bytes[..n]).is_err());
        }
        assert_eq!(
            Wire::decode(&Wire::Control(Message::Stop).encode()),
            Ok(Wire::Control(Message::Stop))
        );
        let mut bad = bytes;
        bad[127] = 1;
        assert_eq!(Wire::decode(&bad), Err(ProtocolError::Reserved));
    }
    #[test]
    fn all_five_handles_and_session_fields_are_bounded() {
        for i in 0..5 {
            for value in [0, u64::MAX] {
                let mut h = hello();
                h.handles[i] = value;
                assert!(h.validate().is_err());
            }
        }
        for i in 0..5 {
            for j in 0..i {
                let mut h = hello();
                h.handles[i] = h.handles[j];
                assert!(h.validate().is_err());
            }
        }
        let mut h = hello();
        h.session.target.creation_time = 0;
        assert!(h.validate().is_err());
        let mut h = hello();
        h.width = 1921;
        assert!(h.validate().is_err());
        let mut h = hello();
        h.qpc_frequency = u64::MAX;
        assert!(h.validate().is_err());
        for n in 0..128 {
            let mut bytes = Wire::Hello(hello()).encode();
            bytes[n] ^= 255;
            let _ = Wire::decode(&bytes);
        }
    }
    #[test]
    fn device_loss_notice_roundtrips_only_supported_reasons_with_exact_reserved_bytes() {
        for reason in [
            DeviceLossReason::Removed,
            DeviceLossReason::Hung,
            DeviceLossReason::Reset,
            DeviceLossReason::DriverInternal,
        ] {
            let wire = Wire::DeviceLost(DeviceLossNotice {
                session: hello().session,
                reason,
            });
            let bytes = wire.encode();
            assert_eq!(Wire::decode(&bytes), Ok(wire));
            for size in 0..PACKET_SIZE {
                assert!(Wire::decode(&bytes[..size]).is_err());
            }
            for value in [0u32, 1, 0x80004005, u32::MAX] {
                let mut bad = bytes;
                bad[48..52].copy_from_slice(&value.to_le_bytes());
                assert_eq!(Wire::decode(&bad), Err(ProtocolError::Fields));
            }
            for index in 52..PACKET_SIZE {
                let mut bad = bytes;
                bad[index] = 1;
                assert_eq!(Wire::decode(&bad), Err(ProtocolError::Reserved));
            }
            let mut bad = bytes;
            bad[12..16].fill(0);
            assert_eq!(Wire::decode(&bad), Err(ProtocolError::Fields));
        }
    }
    #[test]
    fn owned_loss_trigger_has_exact_identity_and_no_payload_or_reserved_data() {
        let wire = Wire::TriggerOwnedLoss(hello().session);
        let bytes = wire.encode();
        assert_eq!(Wire::decode(&bytes), Ok(wire));
        for index in 48..PACKET_SIZE {
            let mut bad = bytes;
            bad[index] = 1;
            assert_eq!(Wire::decode(&bad), Err(ProtocolError::Reserved));
        }
        let mut invalid = hello().session;
        invalid.target.creation_time = 0;
        assert_eq!(
            Wire::decode(&Wire::TriggerOwnedLoss(invalid).encode()),
            Err(ProtocolError::Fields)
        );
    }
}
