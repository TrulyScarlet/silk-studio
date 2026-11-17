//! One-word admission closure plus bounded in-flight accounting.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub struct AdmissionGate {
    state: AtomicU64,
}
pub struct WorkPermit {
    gate: Arc<AdmissionGate>,
}
impl AdmissionGate {
    pub fn try_enter(self: &Arc<Self>) -> Option<WorkPermit> {
        self.state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                if state & 1 != 0 {
                    None
                } else {
                    state.checked_add(2)
                }
            })
            .ok()?;
        Some(WorkPermit { gate: self.clone() })
    }
    pub fn close(&self) {
        self.state.fetch_or(1, Ordering::AcqRel);
    }
    pub fn is_closed(&self) -> bool {
        self.state.load(Ordering::Acquire) & 1 != 0
    }
    pub fn in_flight(&self) -> u64 {
        self.state.load(Ordering::Acquire) >> 1
    }
    pub fn is_drained(&self) -> bool {
        self.state.load(Ordering::Acquire) == 1
    }
    /// A controller may start a new cooperative phase only after ALL its callers
    /// have acknowledged pause. This CAS alone does not establish quiescence of
    /// instructions before try_enter, and cannot authorize detour/DLL unloading.
    pub fn reopen_cooperative_phase(&self) -> bool {
        self.state
            .compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}
impl Drop for WorkPermit {
    fn drop(&mut self) {
        self.gate.state.fetch_sub(2, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_prevents_new_work_but_retains_each_existing_permit() {
        let gate = Arc::new(AdmissionGate::default());
        let a = gate.try_enter().unwrap();
        let b = gate.try_enter().unwrap();
        gate.close();
        gate.close();
        assert!(gate.try_enter().is_none());
        assert_eq!(gate.in_flight(), 2);
        assert!(!gate.is_drained());
        assert!(!gate.reopen_cooperative_phase());
        drop(a);
        assert_eq!(gate.in_flight(), 1);
        drop(b);
        assert!(gate.is_drained());
        assert!(gate.reopen_cooperative_phase());
        assert!(!gate.is_closed());
        assert!(gate.try_enter().is_some());
    }
    #[test]
    fn close_during_inflight_work_does_not_report_a_false_drain() {
        let gate = Arc::new(AdmissionGate::default());
        let (entered, ready) = std::sync::mpsc::sync_channel(1);
        let (release, wait) = std::sync::mpsc::sync_channel(1);
        let other = gate.clone();
        let worker = std::thread::spawn(move || {
            let _permit = other.try_enter().unwrap();
            entered.send(()).unwrap();
            wait.recv().unwrap();
            assert!(other.is_closed());
        });
        ready.recv().unwrap();
        gate.close();
        assert!(!gate.is_drained());
        assert!(gate.try_enter().is_none());
        release.send(()).unwrap();
        worker.join().unwrap();
        assert!(gate.is_drained());
    }
    #[test]
    fn concurrent_entry_cannot_wrap_the_inflight_counter_or_reopen_a_live_gate() {
        let gate = Arc::new(AdmissionGate {
            state: AtomicU64::new(u64::MAX - 1),
        });
        assert!(gate.try_enter().is_none());
        gate.close();
        assert!(!gate.is_drained());
        let gate = Arc::new(AdmissionGate::default());
        assert!(!gate.reopen_cooperative_phase());
        let mut workers = Vec::new();
        for _ in 0..4 {
            let gate = gate.clone();
            workers.push(std::thread::spawn(move || {
                for _ in 0..1000 {
                    let _permit = gate.try_enter();
                    std::thread::yield_now();
                }
            }));
        }
        gate.close();
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(gate.is_drained());
    }
}
