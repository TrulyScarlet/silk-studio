//! Monotonic media clock and shared-timeline conversion (spec §13.4).
//!
//! Rules enforced here and nowhere else:
//! - Packet ordering uses the monotonic timeline only; wall-clock time is
//!   metadata (captured by callers, never used for ordering).
//! - Every source timestamp enters the pipeline through
//!   [`MediaTimeline::to_shared`], which compensates the source's initial
//!   offset so all streams share one origin.
//! - Large jumps are surfaced as discontinuities instead of being silently
//!   folded into the timeline.

use std::time::Instant;

use media_types::TimeBase;
use thiserror::Error;

/// Monotonic clock abstraction so tests can inject deterministic time.
pub trait MonotonicClock: Send {
    /// Nanoseconds since an arbitrary fixed origin.
    fn now_nanos(&self) -> u64;
}

/// Real clock backed by [`Instant`]; origin is first construction.
#[derive(Debug)]
pub struct InstantClock {
    start: Instant,
}

impl InstantClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for InstantClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock for InstantClock {
    fn now_nanos(&self) -> u64 {
        self.start.elapsed().as_nanos() as u64
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("timestamp discontinuity: previous {previous_shared_ms} ms -> next {next_shared_ms} ms (jump {jump_ms} ms exceeds ±{allowed_ms} ms)")]
pub struct Discontinuity {
    pub previous_shared_ms: i64,
    pub next_shared_ms: i64,
    pub jump_ms: i64,
    pub allowed_ms: i64,
}

/// Converts source-local timestamps into the shared millisecond timeline.
///
/// The shared value is `source_ticks rescaled to ms − offset_ms`, where
/// `offset_ms` is captured from the first stamp each source delivers.
#[derive(Debug, Clone)]
pub struct MediaTimeline {
    time_base: TimeBase,
    offset_shared_ms: i64,
    accepted_stamps: u64,
    last_shared_ms: i64,
    max_jump_ms: i64,
}

impl MediaTimeline {
    /// Timeline rescaling from `time_base` into milliseconds; jumps beyond
    /// `max_jump_ms` are rejected as discontinuities.
    ///
    /// The first two stamps calibrate the timeline (origin + cadence) and
    /// are always accepted; the detector judges jumps from the third on.
    pub fn new(time_base: TimeBase, max_jump_ms: i64) -> Self {
        Self {
            time_base,
            offset_shared_ms: 0,
            accepted_stamps: 0,
            last_shared_ms: 0,
            max_jump_ms,
        }
    }

    /// Convert one source-local stamp into shared milliseconds.
    pub fn to_shared(&mut self, source_ticks: i64) -> Result<i64, Discontinuity> {
        let absolute_ms = self.time_base.rescale(source_ticks, TimeBase::MILLISECOND);

        if self.accepted_stamps == 0 {
            self.offset_shared_ms = absolute_ms;
            self.accepted_stamps = 1;
            self.last_shared_ms = 0;
            return Ok(0);
        }

        let shared = absolute_ms - self.offset_shared_ms;

        // From the third stamp on, the cadence is established and large
        // jumps are real discontinuities rather than stream start-up.
        let calibrated = self.accepted_stamps >= 2;
        if calibrated {
            let jump = shared - self.last_shared_ms;
            if jump.abs() > self.max_jump_ms {
                return Err(Discontinuity {
                    previous_shared_ms: self.last_shared_ms,
                    next_shared_ms: shared,
                    jump_ms: jump,
                    allowed_ms: self.max_jump_ms,
                });
            }
        }

        self.accepted_stamps += 1;
        self.last_shared_ms = shared;
        Ok(shared)
    }

    /// Last shared stamp accepted on this timeline.
    pub fn last_shared_ms(&self) -> i64 {
        self.last_shared_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_clock_advances_monotonically() {
        let clock = InstantClock::new();
        let first = clock.now_nanos();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = clock.now_nanos();
        assert!(second > first);
    }

    #[test]
    fn first_stamp_becomes_origin() {
        let mut tl = MediaTimeline::new(TimeBase::from_hz(90_000), 500);
        assert_eq!(tl.to_shared(900_000).expect("first"), 0);
        assert_eq!(tl.to_shared(990_000).expect("second"), 1000);
        assert_eq!(tl.last_shared_ms(), 1000);
    }

    #[test]
    fn detects_backward_jump_beyond_threshold() {
        let mut tl = MediaTimeline::new(TimeBase::MILLISECOND, 50);
        tl.to_shared(0).expect("t0");
        tl.to_shared(1_000).expect("t1");
        let err = tl.to_shared(100).expect_err("backward jump");
        assert_eq!(err.jump_ms, -900);
        assert_eq!(err.previous_shared_ms, 1_000);
        assert_eq!(err.next_shared_ms, 100);
        assert_eq!(
            tl.last_shared_ms(),
            1_000,
            "rejected stamp must not advance"
        );
    }

    #[test]
    fn allows_jumps_within_threshold() {
        let mut tl = MediaTimeline::new(TimeBase::MILLISECOND, 50);
        tl.to_shared(0).expect("t0");
        tl.to_shared(1_000).expect("t1");
        assert_eq!(tl.to_shared(1_040).expect("small forward"), 1_040);
    }

    #[test]
    fn repeated_stamps_do_not_trip_detector() {
        let mut tl = MediaTimeline::new(TimeBase::MILLISECOND, 10);
        assert_eq!(tl.to_shared(5).expect("a"), 0);
        assert_eq!(tl.to_shared(5).expect("repeat"), 0);
    }

    #[test]
    fn second_stamp_calibrates_third_is_judged() {
        let mut tl = MediaTimeline::new(TimeBase::MILLISECOND, 50);
        tl.to_shared(0).expect("first");
        tl.to_shared(1_000).expect("second always accepted");
        let err = tl.to_shared(1_200).expect_err("third judged");
        assert_eq!(err.jump_ms, 200);
    }

    #[test]
    fn different_sources_get_independent_offsets() {
        let mut video = MediaTimeline::new(TimeBase::MILLISECOND, 500);
        let mut audio = MediaTimeline::new(TimeBase::from_hz(48_000), 500);
        assert_eq!(video.to_shared(10_000).expect("v"), 0);
        assert_eq!(audio.to_shared(480_000).expect("a"), 0);
        assert_eq!(video.to_shared(10_480).expect("v+480ms"), 480);
        // 480 ms at 48 kHz = 23 040 ticks.
        assert_eq!(audio.to_shared(503_040).expect("a+480ms"), 480);
    }
}
