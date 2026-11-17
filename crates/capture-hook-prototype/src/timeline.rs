//! Checked session-local QPC conversion for the controlled capture adapter.

pub struct QpcTimeline {
    frequency: i64,
    origin: Option<i64>,
    last: Option<i64>,
}

impl QpcTimeline {
    pub fn new(frequency: u64) -> Option<Self> {
        let frequency = i64::try_from(frequency)
            .ok()
            .filter(|frequency| *frequency > 0)?;
        Some(Self {
            frequency,
            origin: None,
            last: None,
        })
    }

    pub fn milliseconds(&mut self, ticks: i64) -> Result<i64, &'static str> {
        if ticks < 0 || self.last.is_some_and(|last| ticks < last) {
            return Err("QPC timestamp moved backwards or is negative");
        }
        let origin = self.origin.unwrap_or(ticks);
        let milliseconds =
            (i128::from(ticks) - i128::from(origin)) * 1000 / i128::from(self.frequency);
        let milliseconds = i64::try_from(milliseconds).map_err(|_| "QPC conversion overflow")?;
        self.origin = Some(origin);
        self.last = Some(ticks);
        Ok(milliseconds)
    }
}

/// Rational timestamp buckets: no fractional-period drift or catch-up bursts.
pub struct FrameCadence {
    fps: u32,
    last_pts: Option<i64>,
    last_bucket: Option<i128>,
}
impl FrameCadence {
    pub fn new(fps: u32) -> Option<Self> {
        matches!(fps, 30 | 60 | 120).then_some(Self {
            fps,
            last_pts: None,
            last_bucket: None,
        })
    }
    pub fn admit(&mut self, pts_ms: i64) -> Result<bool, &'static str> {
        if pts_ms < 0 || self.last_pts.is_some_and(|last| pts_ms < last) {
            return Err("cadence timestamp moved backwards/negative");
        }
        let bucket = i128::from(pts_ms) * i128::from(self.fps) / 1000;
        let admit = self.last_bucket.is_none_or(|last| bucket > last);
        self.last_pts = Some(pts_ms);
        if admit {
            self.last_bucket = Some(bucket);
        }
        Ok(admit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rational_cadence_has_no_drift_or_catchup_burst() {
        for fps in [30, 60, 120] {
            let mut gate = FrameCadence::new(fps).unwrap();
            let count = (0..10_000).filter(|pts| gate.admit(*pts).unwrap()).count();
            assert_eq!(count, fps as usize * 10);
            assert!(gate.admit(3_600_000).unwrap());
            assert!(!gate.admit(3_600_000).unwrap());
            assert!(gate.admit(3_599_999).is_err());
        }
        assert!(FrameCadence::new(0).is_none());
        assert!(FrameCadence::new(31).is_none());
    }

    #[test]
    fn origin_drops_fractional_ticks_without_accumulating_drift() {
        let mut clock = QpcTimeline::new(10_000_000).unwrap();
        assert_eq!(clock.milliseconds(123456789), Ok(0));
        assert_eq!(clock.milliseconds(123456789 + 166666), Ok(16));
        assert_eq!(clock.milliseconds(123456789 + 333333), Ok(33));
        assert_eq!(clock.milliseconds(123456789 + 10_000_000), Ok(1000));
        assert_eq!(
            clock.milliseconds(123456789 + 36_000_000_000),
            Ok(3_600_000)
        );
    }

    #[test]
    fn invalid_frequency_backward_values_and_overflow_never_advance_state() {
        assert!(QpcTimeline::new(0).is_none());
        assert!(QpcTimeline::new(u64::MAX).is_none());
        let mut clock = QpcTimeline::new(1).unwrap();
        assert!(clock.milliseconds(-1).is_err());
        assert_eq!(clock.milliseconds(10), Ok(0));
        assert!(clock.milliseconds(9).is_err());
        assert!(clock.milliseconds(i64::MAX).is_err());
        assert_eq!(clock.milliseconds(11), Ok(1000));
        assert_eq!(clock.milliseconds(11), Ok(1000));
    }
}
