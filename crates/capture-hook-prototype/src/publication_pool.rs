//! Fixed-capacity publication ownership; no native APIs needed for these tests.
use std::sync::{Arc, Mutex, Weak};

struct Entry<T> {
    surface: Arc<T>,
    handle: Option<u64>,
}
struct State<T> {
    slots: [Entry<T>; crate::SLOT_COUNT],
    active: bool,
    last_handle: u64,
}
pub struct PublicationPool<T> {
    state: Mutex<State<T>>,
}

pub struct Publication<T> {
    pool: Arc<PublicationPool<T>>,
    handle: u64,
    surface: Arc<T>,
}
impl<T> Publication<T> {
    pub fn handle(&self) -> u64 {
        self.handle
    }
    pub fn surface(&self) -> &T {
        &self.surface
    }
}
impl<T> Drop for Publication<T> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.pool.state.lock() {
            if let Some(entry) = state
                .slots
                .iter_mut()
                .find(|entry| entry.handle == Some(self.handle))
            {
                entry.handle = None;
            }
        }
    }
}
impl<T> PublicationPool<T> {
    pub fn new(surfaces: [T; crate::SLOT_COUNT]) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                slots: surfaces.map(|surface| Entry {
                    surface: Arc::new(surface),
                    handle: None,
                }),
                active: true,
                last_handle: 0,
            }),
        })
    }
    /// Producer-only. Handles must be nonzero and globally unique across all
    /// pools that share an encoder context; callers issue them monotonically.
    pub fn try_publish(self: &Arc<Self>, handle: u64) -> Option<Publication<T>> {
        if handle == 0 {
            return None;
        }
        let mut state = self.state.try_lock().ok()?;
        if !state.active || handle <= state.last_handle {
            return None;
        }
        let index = state.slots.iter().position(|entry| {
            entry.handle.is_none()
                && Arc::strong_count(&entry.surface) == 1
                && Arc::weak_count(&entry.surface) == 0
        })?;
        state.last_handle = handle;
        let entry = &mut state.slots[index];
        entry.handle = Some(handle);
        Some(Publication {
            pool: self.clone(),
            handle,
            surface: entry.surface.clone(),
        })
    }
    pub fn resolve(&self, handle: u64) -> Option<Arc<T>> {
        let state = self.state.lock().ok()?;
        if !state.active {
            return None;
        }
        state
            .slots
            .iter()
            .find(|entry| entry.handle == Some(handle))
            .map(|entry| entry.surface.clone())
    }
    pub fn invalidate(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.active = false;
            for entry in &mut state.slots {
                entry.handle = None;
            }
        }
    }
    pub fn retirement_tokens(&self) -> Option<[Weak<T>; crate::SLOT_COUNT]> {
        self.state.lock().ok().map(|state| {
            state
                .slots
                .each_ref()
                .map(|entry| Arc::downgrade(&entry.surface))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_and_delivered_publications_hold_slots_until_resolved_work_finishes() {
        let pool = PublicationPool::new([10, 20, 30]);
        let a = pool.try_publish(1).unwrap();
        let b = pool.try_publish(2).unwrap();
        let c = pool.try_publish(3).unwrap();
        for _ in 0..1000 {
            assert!(pool.try_publish(4).is_none());
        }
        let retained = pool.resolve(a.handle()).unwrap();
        drop(a);
        assert!(pool.resolve(1).is_none());
        assert!(pool.try_publish(4).is_none());
        assert_eq!(*retained, 10);
        drop(retained);
        let reused = pool.try_publish(4).unwrap();
        assert_eq!(*reused.surface(), 10);
        drop((b, c, reused));
        assert!(pool.try_publish(5).is_some());
    }
    #[test]
    fn dropped_events_stale_handles_and_shutdown_release_only_ownership_not_live_surfaces() {
        let pool = PublicationPool::new([10, 20, 30]);
        let a = pool.try_publish(1).unwrap();
        let retained = pool.resolve(1).unwrap();
        pool.invalidate();
        assert!(pool.resolve(1).is_none());
        assert!(pool.try_publish(2).is_none());
        drop(a);
        drop(pool);
        assert_eq!(*retained, 10);
    }
    #[test]
    fn weak_encoder_leases_cannot_resurrect_a_reused_surface() {
        let pool = PublicationPool::new([10, 20, 30]);
        let a = pool.try_publish(1).unwrap();
        let weak = Arc::downgrade(&pool.resolve(1).unwrap());
        drop(a);
        let b = pool.try_publish(2).unwrap();
        let c = pool.try_publish(3).unwrap();
        assert!(pool.try_publish(4).is_none());
        assert_eq!(*weak.upgrade().unwrap(), 10);
        drop(weak);
        assert_eq!(*pool.try_publish(4).unwrap().surface(), 10);
        drop((b, c));
    }
    #[test]
    fn contention_does_not_block_the_producer_and_duplicate_handles_are_rejected() {
        let pool = PublicationPool::new([1, 2, 3]);
        assert!(pool.try_publish(0).is_none());
        let a = pool.try_publish(1).unwrap();
        assert!(pool.try_publish(1).is_none());
        let guard = pool.state.lock().unwrap();
        assert!(pool.try_publish(2).is_none());
        drop(guard);
        drop(a);
        assert!(pool.try_publish(2).is_some());
    }

    #[test]
    fn publication_gap_and_retained_leases_are_safe_on_distinct_threads() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::mpsc::sync_channel;
        let pool = PublicationPool::new(std::array::from_fn(|_| AtomicU64::new(0)));
        let (sender, receiver) = sync_channel::<Publication<AtomicU64>>(2);
        let consumer_pool = pool.clone();
        let consumer = std::thread::spawn(move || {
            for publication in receiver {
                let handle = publication.handle();
                std::thread::yield_now(); // next_event -> resolve gap, still published
                let lease = consumer_pool
                    .resolve(handle)
                    .expect("queued handle became stale");
                assert_eq!(lease.load(Ordering::Acquire), handle);
                drop(publication);
                for _ in 0..3 {
                    std::thread::yield_now();
                    assert_eq!(lease.load(Ordering::Acquire), handle);
                }
            }
        });
        for handle in 1..10_000 {
            if let Some(publication) = pool.try_publish(handle) {
                publication.surface().store(handle, Ordering::Release);
                let _ = sender.try_send(publication); // full queue drops the token
            }
        }
        drop(sender);
        consumer.join().unwrap();
        assert!(pool.try_publish(10_001).is_some());
    }
}
