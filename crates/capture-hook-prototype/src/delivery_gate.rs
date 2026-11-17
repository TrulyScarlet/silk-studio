//! Reject queued work from retired epochs and terminal sources.
use std::sync::atomic::{AtomicU64, Ordering};

pub struct DeliveryGate {
    epoch: AtomicU64,
}
impl Default for DeliveryGate {
    fn default() -> Self {
        Self {
            epoch: AtomicU64::new(1),
        }
    }
}
impl DeliveryGate {
    pub fn current(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
    pub fn accepts(&self, epoch: u64) -> bool {
        epoch != 0 && self.current() == epoch
    }
    pub fn advance(&self, next: u64) -> bool {
        self.epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current != 0 && next > current).then_some(next)
            })
            .is_ok()
    }
    pub fn close(&self) {
        self.epoch.store(0, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_current_epoch_can_deliver_and_terminal_cannot_reopen() {
        let gate = DeliveryGate::default();
        assert!(gate.accepts(1));
        assert!(!gate.accepts(0));
        assert!(!gate.advance(1));
        assert!(gate.advance(2));
        assert!(!gate.accepts(1));
        assert!(gate.accepts(2));
        assert!(!gate.advance(1));
        assert!(!gate.advance(0));
        gate.close();
        assert!(!gate.accepts(2));
        assert!(!gate.advance(3));
    }
    #[test]
    fn closed_source_discards_every_already_queued_epoch() {
        let gate = std::sync::Arc::new(DeliveryGate::default());
        let other = gate.clone();
        std::thread::spawn(move || other.close()).join().unwrap();
        for epoch in [0, 1, 2, u64::MAX] {
            assert!(!gate.accepts(epoch));
        }
    }
}
