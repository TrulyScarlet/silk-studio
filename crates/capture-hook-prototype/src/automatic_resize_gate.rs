//! One in-flight observed resize; parent approval is required before admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomaticResizePhase {
    Stable,
    Preparing,
    Offered,
    Rebuilding,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomaticResizeError {
    WrongPhase,
    InvalidDimensions,
    EpochExhausted,
    WrongEpoch,
}
pub struct AutomaticResizeGate {
    phase: AutomaticResizePhase,
    epoch: u64,
}
impl AutomaticResizeGate {
    pub fn new(epoch: u64) -> Option<Self> {
        (epoch != 0).then_some(Self {
            phase: AutomaticResizePhase::Stable,
            epoch,
        })
    }
    pub fn phase(&self) -> AutomaticResizePhase {
        self.phase
    }
    pub fn prepare(&mut self) -> Result<(), AutomaticResizeError> {
        if self.phase != AutomaticResizePhase::Stable {
            self.close();
            return Err(AutomaticResizeError::WrongPhase);
        }
        self.phase = AutomaticResizePhase::Preparing;
        Ok(())
    }
    pub fn offer(&mut self, width: u32, height: u32) -> Result<(), AutomaticResizeError> {
        if self.phase != AutomaticResizePhase::Preparing {
            self.close();
            return Err(AutomaticResizeError::WrongPhase);
        }
        if width < 2
            || height < 2
            || width > 1920
            || height > 1080
            || !width.is_multiple_of(2)
            || !height.is_multiple_of(2)
        {
            self.close();
            return Err(AutomaticResizeError::InvalidDimensions);
        }
        self.phase = AutomaticResizePhase::Offered;
        Ok(())
    }
    pub fn claim(&mut self) -> Result<u64, AutomaticResizeError> {
        if self.phase != AutomaticResizePhase::Offered {
            self.close();
            return Err(AutomaticResizeError::WrongPhase);
        }
        let Some(next) = self.epoch.checked_add(1) else {
            self.close();
            return Err(AutomaticResizeError::EpochExhausted);
        };
        self.phase = AutomaticResizePhase::Rebuilding;
        Ok(next)
    }
    pub fn approve(&mut self, epoch: u64) -> Result<(), AutomaticResizeError> {
        if self.phase != AutomaticResizePhase::Rebuilding {
            self.close();
            return Err(AutomaticResizeError::WrongPhase);
        }
        if self.epoch.checked_add(1) != Some(epoch) {
            self.close();
            return Err(AutomaticResizeError::WrongEpoch);
        }
        self.epoch = epoch;
        self.phase = AutomaticResizePhase::Stable;
        Ok(())
    }
    pub fn close(&mut self) {
        self.phase = AutomaticResizePhase::Closed;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_epochs_require_observation_retirement_and_exact_approval() {
        let mut gate = AutomaticResizeGate::new(1).unwrap();
        for epoch in 2..=50 {
            gate.prepare().unwrap();
            gate.offer(640, 360).unwrap();
            assert_eq!(gate.claim(), Ok(epoch));
            assert_eq!(gate.phase(), AutomaticResizePhase::Rebuilding);
            gate.approve(epoch).unwrap();
            assert_eq!(gate.phase(), AutomaticResizePhase::Stable);
        }
    }
    #[test]
    fn overlapping_resize_is_terminal_and_cannot_be_reopened() {
        let mut gate = AutomaticResizeGate::new(1).unwrap();
        gate.prepare().unwrap();
        assert!(gate.prepare().is_err());
        assert!(gate.offer(640, 360).is_err());
        assert!(gate.approve(2).is_err());
        assert_eq!(gate.phase(), AutomaticResizePhase::Closed);
    }
    #[test]
    fn wrong_approval_and_bad_dimensions_never_admit_media() {
        for dimensions in [(1, 360), (640, 0), (641, 360), (640, 361), (1922, 1080)] {
            let mut gate = AutomaticResizeGate::new(1).unwrap();
            gate.prepare().unwrap();
            assert!(gate.offer(dimensions.0, dimensions.1).is_err());
            assert_eq!(gate.phase(), AutomaticResizePhase::Closed);
        }
        let mut gate = AutomaticResizeGate::new(7).unwrap();
        gate.prepare().unwrap();
        gate.offer(640, 360).unwrap();
        assert_eq!(gate.claim(), Ok(8));
        assert!(gate.approve(7).is_err());
        assert!(gate.approve(8).is_err());
    }
    #[test]
    fn exhaustion_and_zero_never_wrap_epoch_identity() {
        assert!(AutomaticResizeGate::new(0).is_none());
        let mut gate = AutomaticResizeGate::new(u64::MAX).unwrap();
        gate.prepare().unwrap();
        gate.offer(640, 360).unwrap();
        assert_eq!(gate.claim(), Err(AutomaticResizeError::EpochExhausted));
        assert_eq!(gate.phase(), AutomaticResizePhase::Closed);
    }
}
