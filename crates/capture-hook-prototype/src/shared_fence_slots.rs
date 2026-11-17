//! Three-slot ownership across D3D12-produced and D3D11-consumed fences.
use crate::SLOT_COUNT;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Free,
    Produced(u64),
    Published(u64),
    Consuming(u64),
}
pub struct SharedFenceSlots {
    slots: [Slot; SLOT_COUNT],
    next: u64,
    open: bool,
}
impl Default for SharedFenceSlots {
    fn default() -> Self {
        Self {
            slots: [Slot::Free; SLOT_COUNT],
            next: 1,
            open: true,
        }
    }
}
impl SharedFenceSlots {
    pub fn reserve(&mut self, consumed: u64) -> Result<Option<(usize, u64)>, &'static str> {
        if !self.open || consumed == u64::MAX {
            self.close();
            return Err("shared-fence ring closed/device removed");
        }
        for slot in &mut self.slots {
            if matches!(slot, Slot::Consuming(value) if *value <= consumed) {
                *slot = Slot::Free;
            }
        }
        if self.next == u64::MAX {
            self.close();
            return Err("shared-fence sequence exhausted");
        }
        let Some(index) = self.slots.iter().position(|s| *s == Slot::Free) else {
            return Ok(None);
        };
        let sequence = self.next;
        self.next += 1;
        self.slots[index] = Slot::Produced(sequence);
        Ok(Some((index, sequence)))
    }
    pub fn ready(&mut self, produced: u64) -> Result<Option<(usize, u64)>, &'static str> {
        if !self.open || produced == u64::MAX {
            self.close();
            return Err("shared-fence ring closed/device removed");
        }
        Ok(self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s {
                Slot::Produced(v) if *v <= produced => Some((i, *v)),
                _ => None,
            })
            .min_by_key(|(_, v)| *v))
    }
    pub fn consuming(&mut self, slot: usize, sequence: u64) -> bool {
        if !self.open || self.slots.get(slot) != Some(&Slot::Produced(sequence)) {
            return false;
        }
        self.slots[slot] = Slot::Consuming(sequence);
        true
    }
    pub fn publish_remote(&mut self, slot: usize, sequence: u64) -> bool {
        if !self.open || self.slots.get(slot) != Some(&Slot::Produced(sequence)) {
            return false;
        }
        self.slots[slot] = Slot::Published(sequence);
        true
    }
    pub fn acknowledge_remote(&mut self, slot: usize, sequence: u64) -> bool {
        if !self.open || self.slots.get(slot) != Some(&Slot::Published(sequence)) {
            return false;
        }
        self.slots[slot] = Slot::Consuming(sequence);
        true
    }
    /// Only before any consumer GPU copy; producer completion is required.
    pub fn discard(&mut self, slot: usize, sequence: u64, completed: u64) -> bool {
        if !self.open
            || completed == u64::MAX
            || completed < sequence
            || self.slots.get(slot) != Some(&Slot::Produced(sequence))
        {
            return false;
        }
        self.slots[slot] = Slot::Free;
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
    fn remote_publication_needs_exact_ack_and_completed_consumer_fence() {
        let mut ring = SharedFenceSlots::default();
        for _ in 0..3 {
            ring.reserve(0).unwrap().unwrap();
        }
        assert!(!ring.acknowledge_remote(0, 1));
        assert!(ring.publish_remote(0, 1));
        assert!(!ring.publish_remote(0, 1));
        assert_eq!(ring.ready(3), Ok(Some((1, 2))));
        assert!(!ring.discard(0, 1, 3));
        assert!(!ring.acknowledge_remote(0, 2));
        assert_eq!(ring.reserve(3), Ok(None));
        assert!(ring.acknowledge_remote(0, 1));
        assert!(!ring.acknowledge_remote(0, 1));
        assert_eq!(ring.reserve(0), Ok(None));
        assert_eq!(ring.reserve(1), Ok(Some((0, 4))));
        ring.close();
        assert!(!ring.acknowledge_remote(1, 2));
    }
    #[test]
    fn producer_completion_is_not_consumer_completion() {
        let mut ring = SharedFenceSlots::default();
        for i in 0..3 {
            assert_eq!(ring.reserve(0), Ok(Some((i, i as u64 + 1))));
        }
        assert_eq!(ring.ready(0), Ok(None));
        assert_eq!(ring.ready(3), Ok(Some((0, 1))));
        assert_eq!(ring.reserve(100), Ok(None));
        assert!(!ring.consuming(0, 2));
        assert!(ring.consuming(0, 1));
        assert_eq!(ring.reserve(0), Ok(None));
        assert_eq!(ring.reserve(1), Ok(Some((0, 4))));
        assert!(!ring.discard(0, 1, 100));
        assert!(!ring.discard(1, 2, 1));
        assert!(ring.discard(1, 2, 3));
        assert!(!ring.discard(1, 2, 3));
        assert_eq!(ring.reserve(1), Ok(Some((1, 5))));
        assert_eq!(ring.ready(5), Ok(Some((2, 3))));
    }
    #[test]
    fn device_loss_close_and_overflow_never_reopen() {
        let mut ring = SharedFenceSlots::default();
        assert!(ring.ready(u64::MAX).is_err());
        assert!(ring.reserve(0).is_err());
        let mut ring = SharedFenceSlots::default();
        assert!(ring.reserve(u64::MAX).is_err());
        let mut ring = SharedFenceSlots {
            next: u64::MAX,
            ..Default::default()
        };
        assert!(ring.reserve(0).is_err());
        assert!(ring.ready(0).is_err());
    }
}
