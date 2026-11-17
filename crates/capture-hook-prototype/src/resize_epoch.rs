//! Deterministic exact-session contract for one in-flight resize replacement.
use crate::d3d12_protocol::{GpuHello, ResizeRequest};
use crate::SessionIdentity;

pub struct ResizeEpoch {
    current: GpuHello,
    pending: Option<ResizeRequest>,
    closed: bool,
}
impl ResizeEpoch {
    pub fn new(current: GpuHello) -> Result<Self, String> {
        current
            .validate()
            .map_err(|e| format!("invalid initial epoch: {e:?}"))?;
        Ok(Self {
            current,
            pending: None,
            closed: false,
        })
    }
    pub fn begin(&mut self, request: ResizeRequest) -> Result<SessionIdentity, String> {
        if self.closed
            || self.pending.is_some()
            || request.session != self.current.session
            || request.validate().is_err()
        {
            return Err("invalid/overlapping resize request".into());
        }
        let next = SessionIdentity {
            epoch: self
                .current
                .session
                .epoch
                .checked_add(1)
                .ok_or("resize epoch exhausted")?,
            ..self.current.session
        };
        self.pending = Some(request);
        Ok(next)
    }
    pub fn expected(&self) -> Result<SessionIdentity, String> {
        if self.closed {
            return Err("resize epoch closed".into());
        }
        let request = self.pending.ok_or("no resize pending")?;
        Ok(SessionIdentity {
            epoch: request
                .session
                .epoch
                .checked_add(1)
                .ok_or("resize epoch exhausted")?,
            ..request.session
        })
    }
    pub fn offer(&mut self, hello: GpuHello) -> Result<(), String> {
        let request = self.pending.ok_or("unsolicited replacement handles")?;
        if hello.validate().is_err()
            || hello.session != self.expected()?
            || (hello.width, hello.height) != (request.width, request.height)
            || hello.adapter_luid != self.current.adapter_luid
            || hello.qpc_frequency != self.current.qpc_frequency
        {
            return Err("replacement epoch/identity/format mismatch".into());
        }
        self.current = hello;
        self.pending = None;
        Ok(())
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TargetIdentity;
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
            qpc_frequency: 1000,
            width: 318,
            height: 178,
            handles: [1, 2, 3, 4, 5],
        }
    }
    #[test]
    fn exact_replacement_and_no_overlapping_or_unsolicited_offer() {
        let initial = hello();
        let mut gate = ResizeEpoch::new(initial).unwrap();
        assert!(gate.offer(initial).is_err());
        let request = ResizeRequest {
            session: initial.session,
            width: 640,
            height: 360,
        };
        let next = gate.begin(request).unwrap();
        assert_eq!(next.epoch, 2);
        assert!(gate.begin(request).is_err());
        let replacement = GpuHello {
            session: next,
            width: 640,
            height: 360,
            ..initial
        };
        for bad in [
            initial,
            GpuHello {
                width: 800,
                ..replacement
            },
            GpuHello {
                adapter_luid: 99,
                ..replacement
            },
            GpuHello {
                qpc_frequency: 99,
                ..replacement
            },
            GpuHello {
                session: SessionIdentity {
                    generation: 2,
                    ..next
                },
                ..replacement
            },
        ] {
            assert!(gate.offer(bad).is_err());
            assert_eq!(gate.expected().unwrap(), next);
        }
        gate.offer(replacement).unwrap();
        assert!(gate.offer(replacement).is_err());
        assert!(gate.begin(request).is_err());
        gate.close();
        assert!(gate
            .begin(ResizeRequest {
                session: next,
                ..request
            })
            .is_err());
    }
    #[test]
    fn replacement_wire_is_distinct_and_rejects_invalid_fields() {
        use crate::d3d12_protocol::Wire;
        let request = ResizeRequest {
            session: hello().session,
            width: 640,
            height: 360,
        };
        let bytes = Wire::ReplaceDevice(request).encode();
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 6);
        assert_ne!(bytes, Wire::Resize(request).encode());
        for (width, height) in [(0, 360), (641, 360), (640, 361), (1922, 360), (640, 1082)] {
            assert!(Wire::decode(
                &Wire::ReplaceDevice(ResizeRequest {
                    width,
                    height,
                    ..request
                })
                .encode()
            )
            .is_err());
        }
        for session in [
            SessionIdentity {
                epoch: 0,
                ..request.session
            },
            SessionIdentity {
                generation: 0,
                ..request.session
            },
            SessionIdentity {
                target: crate::TargetIdentity {
                    pid: 0,
                    ..request.session.target
                },
                ..request.session
            },
        ] {
            assert!(Wire::decode(
                &Wire::ReplaceDevice(ResizeRequest { session, ..request }).encode()
            )
            .is_err());
        }
    }
    #[test]
    fn bounds_overflow_and_fixed_wire_validation() {
        let initial = hello();
        for (width, height) in [(0, 2), (2, 0), (3, 2), (2, 3), (1922, 1080), (1920, 1082)] {
            assert!(ResizeRequest {
                session: initial.session,
                width,
                height
            }
            .validate()
            .is_err());
        }
        let mut gate = ResizeEpoch::new(GpuHello {
            session: SessionIdentity {
                epoch: u64::MAX,
                ..initial.session
            },
            ..initial
        })
        .unwrap();
        assert!(gate
            .begin(ResizeRequest {
                session: SessionIdentity {
                    epoch: u64::MAX,
                    ..initial.session
                },
                width: 640,
                height: 360
            })
            .is_err());
        use crate::d3d12_protocol::Wire;
        for wire in [
            Wire::ReplaceDevice(ResizeRequest {
                session: initial.session,
                width: 640,
                height: 360,
            }),
            Wire::Resize(ResizeRequest {
                session: initial.session,
                width: 640,
                height: 360,
            }),
            Wire::Retire(initial.session),
            Wire::Retired(initial.session),
        ] {
            let bytes = wire.encode();
            assert_eq!(Wire::decode(&bytes), Ok(wire));
            for len in 0..128 {
                assert!(Wire::decode(&bytes[..len]).is_err());
            }
            let mut bad = bytes;
            bad[127] = 1;
            assert!(Wire::decode(&bad).is_err());
        }
    }
}
