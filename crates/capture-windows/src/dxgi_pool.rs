//! Fixed-size texture leases. A texture cannot be overwritten while an encoder
//! holds it, and an old handle cannot resolve to a newer frame.
use std::sync::{Arc, Mutex, Weak};

/// Counts live resources across active and retired pools, without retaining them.
pub(crate) struct SurfaceBudget<T> {
    limit: usize,
    live: Vec<Weak<T>>,
}
impl<T> SurfaceBudget<T> {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            live: Vec::new(),
        }
    }
    pub fn allocate<E>(
        &mut self,
        count: usize,
        mut create: impl FnMut() -> Result<T, E>,
    ) -> Result<Option<Vec<Arc<T>>>, E> {
        self.live.retain(|slot| slot.strong_count() != 0);
        if count > self.limit.saturating_sub(self.live.len()) {
            return Ok(None);
        }
        let slots = (0..count)
            .map(|_| create().map(Arc::new))
            .collect::<Result<Vec<_>, _>>()?;
        self.live.extend(slots.iter().map(Arc::downgrade));
        Ok(Some(slots))
    }
}

pub(crate) struct LeasePool<T> {
    slots: Vec<Arc<T>>,
    handles: Mutex<Vec<Option<u64>>>,
}

impl<T> LeasePool<T> {
    pub fn new(slots: Vec<T>) -> Self {
        Self::from_shared(slots.into_iter().map(Arc::new).collect())
    }
    pub fn from_shared(slots: Vec<Arc<T>>) -> Self {
        Self {
            handles: Mutex::new(vec![None; slots.len()]),
            slots,
        }
    }

    pub fn acquire(&self) -> Option<(usize, Arc<T>)> {
        let mut handles = self.handles.lock().ok()?;
        let index = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| Arc::strong_count(slot) == 1)
            .min_by_key(|(i, _)| handles[*i].unwrap_or(0))
            .map(|(i, _)| i)?;
        handles[index] = None;
        Some((index, Arc::clone(&self.slots[index])))
    }

    pub fn publish(&self, index: usize, handle: u64) {
        if let Ok(mut handles) = self.handles.lock() {
            handles[index] = Some(handle);
        }
    }

    pub fn resolve(&self, handle: u64) -> Option<Arc<T>> {
        let handles = self.handles.lock().ok()?;
        let index = handles.iter().position(|h| *h == Some(handle))?;
        Some(Arc::clone(&self.slots[index]))
    }

    pub fn clear(&self) {
        if let Ok(mut handles) = self.handles.lock() {
            handles.fill(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_delivered_and_resolved_owners_prevent_overwrite_and_budget_growth() {
        let mut budget = SurfaceBudget::new(4);
        let slots = budget.allocate(2, || Ok::<_, ()>(17)).unwrap().unwrap();
        let first = LeasePool::from_shared(slots);
        let (index, queued) = first.acquire().unwrap();
        first.publish(index, 1);
        let encoder = first.resolve(1).unwrap();
        let (second, delivered) = first.acquire().unwrap();
        first.publish(second, 2);
        assert!(first.acquire().is_none());
        drop(queued);
        assert!(first.acquire().is_none());
        let replacement = budget.allocate(2, || Ok::<_, ()>(99)).unwrap().unwrap();
        assert!(budget.allocate(1, || Ok::<_, ()>(0)).unwrap().is_none());
        first.clear();
        assert!(first.resolve(1).is_none());
        assert_eq!(*encoder, 17);
        drop(first);
        drop(delivered);
        assert!(budget.allocate(2, || Ok::<_, ()>(0)).unwrap().is_none());
        drop(encoder);
        assert!(budget.allocate(2, || Ok::<_, ()>(0)).unwrap().is_some());
        drop(replacement);
    }
    #[test]
    fn failed_surface_allocation_does_not_consume_budget() {
        let mut budget = SurfaceBudget::<u8>::new(2);
        let mut calls = 0;
        assert!(budget
            .allocate(2, || {
                calls += 1;
                if calls == 2 {
                    Err(())
                } else {
                    Ok(1)
                }
            })
            .is_err());
        assert!(budget.allocate(2, || Ok::<_, ()>(2)).unwrap().is_some());
    }

    #[test]
    fn leases_bound_memory_and_prevent_overwrite() {
        let pool = LeasePool::new(vec![1, 2]);
        let (a, first) = pool.acquire().unwrap();
        pool.publish(a, 10);
        let leased = pool.resolve(10).unwrap();
        drop(first);
        let (b, second) = pool.acquire().unwrap();
        assert_ne!(a, b);
        assert!(pool.acquire().is_none());
        drop(leased);
        let (reused, _) = pool.acquire().unwrap();
        assert_eq!(reused, a);
        assert!(pool.resolve(10).is_none());
        drop(second);
    }

    #[test]
    fn shutdown_invalidates_handles_but_keeps_existing_leases_alive() {
        let pool = LeasePool::new(vec![42]);
        let (index, frame) = pool.acquire().unwrap();
        pool.publish(index, 1);
        pool.clear();
        assert!(pool.resolve(1).is_none());
        assert_eq!(*frame, 42);
    }
}
