use serde::{Deserialize, Serialize};

/// Rational time base used by every timestamp in the pipeline.
///
/// A packet timestamp is only meaningful together with its time base:
/// `seconds = pts * num / den`. All components must convert through
/// [`TimeBase::rescale`] rather than assuming units (spec §13.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeBase {
    pub num: i32,
    pub den: i32,
}

impl TimeBase {
    pub const MILLISECOND: TimeBase = TimeBase { num: 1, den: 1000 };
    pub const MICROSECOND: TimeBase = TimeBase {
        num: 1,
        den: 1_000_000,
    };
    pub const NANOSECOND: TimeBase = TimeBase {
        num: 1,
        den: 1_000_000_000,
    };

    pub const fn new(num: i32, den: i32) -> Self {
        assert!(num > 0 && den > 0, "time base components must be positive");
        Self { num, den }
    }

    pub const fn from_hz(hz: u32) -> Self {
        Self {
            num: 1,
            den: hz as i32,
        }
    }

    /// Rescale `value` from this time base into `target`.
    ///
    /// Values are truncated toward zero; callers that need exact rounding
    /// must pick time bases that divide evenly (the engine uses
    /// millisecond and sample-rate bases which interconvert exactly for
    /// the durations in use).
    pub fn rescale(&self, value: i64, target: TimeBase) -> i64 {
        let numerator = value as i128 * self.num as i128 * target.den as i128;
        let denominator = self.den as i128 * target.num as i128;
        (numerator / denominator) as i64
    }
}

impl Default for TimeBase {
    fn default() -> Self {
        Self::MILLISECOND
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescales_between_common_bases() {
        let ms = TimeBase::MILLISECOND;
        let ns = TimeBase::NANOSECOND;
        let khz90 = TimeBase::from_hz(90_000);
        let audio48k = TimeBase::from_hz(48_000);

        assert_eq!(ms.rescale(1_500, ns), 1_500_000_000);
        assert_eq!(ms.rescale(1_000, khz90), 90_000);
        assert_eq!(khz90.rescale(4_320_000, ms), 48_000);
        assert_eq!(audio48k.rescale(96_000, ms), 2_000);
        assert_eq!(ns.rescale(16_666_667, ms), 16);
    }

    #[test]
    fn rescale_truncates_toward_zero() {
        let ms = TimeBase::MILLISECOND;
        assert_eq!(ms.rescale(1_999, TimeBase::new(1, 1)), 1);
        assert_eq!(ms.rescale(-1_999, TimeBase::new(1, 1)), -1);
    }

    #[test]
    fn rejects_non_positive_components() {
        assert!(std::panic::catch_unwind(|| TimeBase::new(0, 100)).is_err());
        assert!(std::panic::catch_unwind(|| TimeBase::new(1, 0)).is_err());
    }
}
