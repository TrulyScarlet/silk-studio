//! Metadata-only gates for a future producer-observed device-loss transition.
//! No queue inference, allocation, target discovery or automatic runtime opt-in.
use crate::d3d12_protocol::{DeviceLossNotice, GpuHello};
use crate::SessionIdentity;

/// A local tail fence must exceed every sequence that may already have been
/// signaled, including a failed/uncertain copy attempt. MAX is removal, not work.
pub fn next_consumer_retirement_ticket(last_seen: u64, last_consumed: u64) -> Option<u64> {
    last_seen
        .max(last_consumed)
        .checked_add(1)
        .filter(|value| *value != u64::MAX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LossPhase {
    Stable,
    Retiring,
    Retired,
    Offered,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LossTransitionError {
    InvalidInitial,
    WrongPhase,
    WrongSession,
    EpochExhausted,
    InvalidOffer,
}
pub struct DeviceLossEpoch {
    current: GpuHello,
    next: Option<SessionIdentity>,
    offered: Option<GpuHello>,
    phase: LossPhase,
}
impl DeviceLossEpoch {
    pub fn new(current: GpuHello) -> Result<Self, LossTransitionError> {
        current
            .validate()
            .map_err(|_| LossTransitionError::InvalidInitial)?;
        Ok(Self {
            current,
            next: None,
            offered: None,
            phase: LossPhase::Stable,
        })
    }
    pub fn phase(&self) -> LossPhase {
        self.phase
    }
    pub fn current(&self) -> GpuHello {
        self.current
    }
    pub fn close(&mut self) {
        self.phase = LossPhase::Closed;
        self.next = None;
        self.offered = None;
    }
    fn reject<T>(&mut self, error: LossTransitionError) -> Result<T, LossTransitionError> {
        self.close();
        Err(error)
    }
    pub fn begin(
        &mut self,
        notice: DeviceLossNotice,
    ) -> Result<SessionIdentity, LossTransitionError> {
        if self.phase != LossPhase::Stable {
            return self.reject(LossTransitionError::WrongPhase);
        }
        if notice.session != self.current.session {
            return self.reject(LossTransitionError::WrongSession);
        }
        let Some(epoch) = self.current.session.epoch.checked_add(1) else {
            return self.reject(LossTransitionError::EpochExhausted);
        };
        let next = SessionIdentity {
            epoch,
            ..self.current.session
        };
        self.next = Some(next);
        self.phase = LossPhase::Retiring;
        Ok(next)
    }
    pub fn retired(&mut self, next: SessionIdentity) -> Result<(), LossTransitionError> {
        if self.phase != LossPhase::Retiring {
            return self.reject(LossTransitionError::WrongPhase);
        }
        if self.next != Some(next) {
            return self.reject(LossTransitionError::WrongSession);
        }
        self.phase = LossPhase::Retired;
        Ok(())
    }
    pub fn offer(&mut self, replacement: GpuHello) -> Result<(), LossTransitionError> {
        if self.phase != LossPhase::Retired {
            return self.reject(LossTransitionError::WrongPhase);
        }
        if self.next != Some(replacement.session) {
            return self.reject(LossTransitionError::WrongSession);
        }
        if replacement.validate().is_err()
            || replacement.adapter_luid != self.current.adapter_luid
            || replacement.qpc_frequency != self.current.qpc_frequency
            || replacement.width < 2
            || replacement.height < 2
            || !replacement.width.is_multiple_of(2)
            || !replacement.height.is_multiple_of(2)
        {
            return self.reject(LossTransitionError::InvalidOffer);
        }
        self.offered = Some(replacement);
        self.phase = LossPhase::Offered;
        Ok(())
    }
    pub fn approve(&mut self, next: SessionIdentity) -> Result<GpuHello, LossTransitionError> {
        if self.phase != LossPhase::Offered {
            return self.reject(LossTransitionError::WrongPhase);
        }
        if self.next != Some(next) {
            return self.reject(LossTransitionError::WrongSession);
        }
        let Some(replacement) = self.offered.take() else {
            return self.reject(LossTransitionError::InvalidOffer);
        };
        self.current = replacement;
        self.next = None;
        self.phase = LossPhase::Stable;
        Ok(replacement)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d3d12_protocol::DeviceLossReason;
    fn hello() -> GpuHello {
        GpuHello {
            session: SessionIdentity {
                target: crate::TargetIdentity {
                    pid: 7,
                    creation_time: 8,
                    window: 9,
                },
                generation: 1,
                epoch: 1,
            },
            adapter_luid: 10,
            qpc_frequency: 10_000_000,
            width: 318,
            height: 178,
            handles: [11, 12, 13, 14, 15],
        }
    }
    fn notice(hello: GpuHello) -> DeviceLossNotice {
        DeviceLossNotice {
            session: hello.session,
            reason: DeviceLossReason::Removed,
        }
    }
    #[test]
    fn exact_retirement_then_offer_then_approval_admits_each_fresh_epoch() {
        let mut gate = DeviceLossEpoch::new(hello()).unwrap();
        for epoch in 2..=50 {
            let old = gate.current();
            let next = gate.begin(notice(old)).unwrap();
            assert_eq!(next.epoch, epoch);
            assert_eq!(gate.current(), old);
            gate.retired(next).unwrap();
            let replacement = GpuHello {
                session: next,
                width: 640,
                height: 360,
                ..old
            };
            gate.offer(replacement).unwrap();
            assert_eq!(gate.current(), old);
            assert_eq!(gate.approve(next), Ok(replacement));
            assert_eq!(gate.phase(), LossPhase::Stable);
        }
    }
    #[test]
    fn stale_target_generation_epoch_and_retirement_fail_terminally() {
        for which in 0..5 {
            let mut gate = DeviceLossEpoch::new(hello()).unwrap();
            let mut loss = notice(hello());
            match which {
                0 => loss.session.target.pid += 1,
                1 => loss.session.target.creation_time += 1,
                2 => loss.session.target.window += 1,
                3 => loss.session.generation += 1,
                _ => loss.session.epoch += 1,
            }
            assert_eq!(gate.begin(loss), Err(LossTransitionError::WrongSession));
            assert_eq!(gate.phase(), LossPhase::Closed);
            assert!(gate.begin(notice(hello())).is_err());
        }
        let mut gate = DeviceLossEpoch::new(hello()).unwrap();
        let mut next = gate.begin(notice(hello())).unwrap();
        next.epoch += 1;
        assert!(gate.retired(next).is_err());
        assert_eq!(gate.phase(), LossPhase::Closed);
    }
    #[test]
    fn overlapping_loss_and_premature_approval_cannot_reopen_capture() {
        let mut gate = DeviceLossEpoch::new(hello()).unwrap();
        gate.begin(notice(hello())).unwrap();
        assert!(gate.begin(notice(hello())).is_err());
        assert_eq!(gate.phase(), LossPhase::Closed);
        let mut gate = DeviceLossEpoch::new(hello()).unwrap();
        let next = gate.begin(notice(hello())).unwrap();
        assert!(gate.approve(next).is_err());
        assert!(gate.retired(next).is_err());
    }
    #[test]
    fn invalid_graph_offer_or_wrong_approval_never_changes_current_media() {
        for which in 0..7 {
            let old = hello();
            let mut gate = DeviceLossEpoch::new(old).unwrap();
            let next = gate.begin(notice(old)).unwrap();
            gate.retired(next).unwrap();
            let mut replacement = GpuHello {
                session: next,
                ..old
            };
            match which {
                0 => replacement.adapter_luid += 1,
                1 => replacement.qpc_frequency += 1,
                2 => replacement.handles[0] = 0,
                3 => replacement.width = 641,
                4 => replacement.height = 1,
                5 => replacement.width = 1922,
                _ => replacement.session.target.window += 1,
            }
            assert!(gate.offer(replacement).is_err());
            assert_eq!(gate.current(), old);
            assert_eq!(gate.phase(), LossPhase::Closed);
        }
        let old = hello();
        let mut gate = DeviceLossEpoch::new(old).unwrap();
        let next = gate.begin(notice(old)).unwrap();
        gate.retired(next).unwrap();
        gate.offer(GpuHello {
            session: next,
            ..old
        })
        .unwrap();
        assert!(gate.approve(old.session).is_err());
        assert_eq!(gate.current(), old);
    }
    #[test]
    fn zero_initial_identity_and_exhausted_epoch_never_wrap() {
        let mut invalid = hello();
        invalid.session.epoch = 0;
        assert!(DeviceLossEpoch::new(invalid).is_err());
        let mut last = hello();
        last.session.epoch = u64::MAX;
        let mut gate = DeviceLossEpoch::new(last).unwrap();
        assert_eq!(
            gate.begin(notice(last)),
            Err(LossTransitionError::EpochExhausted)
        );
        assert_eq!(gate.phase(), LossPhase::Closed);
    }
    #[test]
    fn consumer_tail_ticket_exceeds_uncertain_attempts_and_never_uses_removal_sentinel() {
        assert_eq!(next_consumer_retirement_ticket(0, 0), Some(1));
        assert_eq!(next_consumer_retirement_ticket(7, 3), Some(8));
        assert_eq!(next_consumer_retirement_ticket(3, 7), Some(8));
        assert_eq!(
            next_consumer_retirement_ticket(u64::MAX - 2, 1),
            Some(u64::MAX - 1)
        );
        for last in [u64::MAX - 1, u64::MAX] {
            assert_eq!(next_consumer_retirement_ticket(last, 0), None);
            assert_eq!(next_consumer_retirement_ticket(0, last), None);
        }
    }
}
