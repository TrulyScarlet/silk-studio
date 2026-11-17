//! Fixed D3D12 submission ownership; completion is not consumer release.
use crate::SLOT_COUNT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    Submitted(u64),
    Leased(u64),
}

pub struct FenceSlots {
    slots: [State; SLOT_COUNT],
    next: u64,
    open: bool,
}
impl Default for FenceSlots {
    fn default() -> Self {
        Self {
            slots: [State::Free; SLOT_COUNT],
            next: 1,
            open: true,
        }
    }
}
impl FenceSlots {
    /// Reserve before recording; caller must close on any recording/signal error.
    pub fn reserve(&mut self) -> Option<(usize, u64)> {
        if !self.open || self.next == u64::MAX {
            return None;
        }
        let slot = self.slots.iter().position(|s| *s == State::Free)?;
        let value = self.next;
        self.next += 1;
        self.slots[slot] = State::Submitted(value);
        Some((slot, value))
    }
    pub fn ready(&self, completed: u64) -> Option<(usize, u64)> {
        if !self.open || completed == u64::MAX {
            return None;
        }
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, state)| match state {
                State::Submitted(value) if *value <= completed => Some((slot, *value)),
                _ => None,
            })
            .min_by_key(|(_, value)| *value)
    }
    pub fn lease(&mut self, slot: usize, value: u64) -> bool {
        if !self.open || self.slots.get(slot) != Some(&State::Submitted(value)) {
            return false;
        }
        self.slots[slot] = State::Leased(value);
        true
    }
    pub fn release(&mut self, slot: usize, value: u64) -> bool {
        if self.slots.get(slot) != Some(&State::Leased(value)) {
            return false;
        }
        self.slots[slot] = State::Free;
        true
    }
    pub fn close(&mut self) {
        self.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fence_completion_never_releases_a_consumer_lease() {
        let mut slots = FenceSlots::default();
        for i in 0..SLOT_COUNT {
            assert_eq!(slots.reserve(), Some((i, i as u64 + 1)));
        }
        assert_eq!(slots.reserve(), None);
        assert_eq!(slots.ready(0), None);
        assert_eq!(slots.ready(u64::MAX), None); // D3D12 device-removed sentinel
        assert_eq!(slots.ready(3), Some((0, 1)));
        assert!(slots.lease(0, 1));
        assert!(!slots.release(0, 2));
        assert!(!slots.release(9, 1));
        assert_eq!(slots.reserve(), None);
        assert!(slots.release(0, 1));
        assert!(!slots.release(0, 1));
        assert_eq!(slots.reserve(), Some((0, 4)));
        assert_eq!(slots.ready(3), Some((1, 2)));
    }
    #[test]
    fn close_and_exhaustion_fail_closed() {
        let mut slots = FenceSlots::default();
        let (slot, value) = slots.reserve().unwrap();
        slots.close();
        assert_eq!(slots.reserve(), None);
        assert_eq!(slots.ready(1), None);
        assert!(!slots.lease(slot, value));
        let mut slots = FenceSlots {
            next: u64::MAX,
            ..Default::default()
        };
        assert_eq!(slots.reserve(), None);
    }
}
