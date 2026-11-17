//! Bounded fault probes compiled ONLY with the explicit fixture-probes feature.
//! Never enable these checkpoints for a production rendering callback.
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct FixtureProbe {
    armed: AtomicBool,
    release: AtomicBool,
    entered: SyncSender<()>,
    injected_return: AtomicI32,
    pub timed_out: AtomicBool,
}
impl FixtureProbe {
    pub fn new() -> (Arc<Self>, Receiver<()>) {
        let (entered, receiver) = sync_channel(1);
        (
            Arc::new(Self {
                armed: AtomicBool::new(false),
                release: AtomicBool::new(true),
                entered,
                injected_return: AtomicI32::new(0),
                timed_out: AtomicBool::new(false),
            }),
            receiver,
        )
    }
    pub fn arm_checkpoint(&self) {
        self.release.store(false, Ordering::Release);
        self.armed.store(true, Ordering::Release);
    }
    pub fn release_checkpoint(&self) {
        self.release.store(true, Ordering::Release);
    }
    pub fn inject_next_present_result(&self, hresult: i32) {
        self.injected_return.store(hresult, Ordering::Release);
    }
    pub(crate) fn take_present_result(&self) -> i32 {
        self.injected_return.swap(0, Ordering::AcqRel)
    }
    pub(crate) fn checkpoint(&self) -> bool {
        if !self.armed.swap(false, Ordering::AcqRel) {
            return true;
        }
        let _ = self.entered.try_send(());
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.release.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        let released = self.release.load(Ordering::Acquire);
        if !released {
            self.timed_out.store(true, Ordering::Release);
        }
        released
    }
}
