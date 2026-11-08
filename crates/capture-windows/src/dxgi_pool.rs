//! Fixed-size texture leases. A texture cannot be overwritten while an encoder
//! holds it, and an old handle cannot resolve to a newer frame.
use std::sync::{Arc, Mutex};

pub(crate) struct LeasePool<T> {
    slots: Vec<Arc<T>>,
    handles: Mutex<Vec<Option<u64>>>,
}

impl<T> LeasePool<T> {
    pub fn new(slots: Vec<T>) -> Self {
        Self {
            handles: Mutex::new(vec![None; slots.len()]),
            slots: slots.into_iter().map(Arc::new).collect(),
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
