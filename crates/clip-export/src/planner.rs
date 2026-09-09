use std::convert::TryFrom;

use thiserror::Error;

/// Decimal byte target used by the small preset.
pub const TARGET_BYTES_20M: u64 = 20_000_000;
/// Decimal byte target used by the medium preset.
pub const TARGET_BYTES_50M: u64 = 50_000_000;
/// Decimal byte target used by the large preset.
pub const TARGET_BYTES_500M: u64 = 500_000_000;

const BITS_PER_BYTE: u128 = 8;
const MICROS_PER_SECOND: u128 = 1_000_000;

/// Alias for [`TARGET_BYTES_20M`] that makes the unit explicit.
pub const TARGET_BYTES_20_MB: u64 = TARGET_BYTES_20M;
/// Alias for [`TARGET_BYTES_50M`] that makes the unit explicit.
pub const TARGET_BYTES_50_MB: u64 = TARGET_BYTES_50M;
/// Alias for [`TARGET_BYTES_500M`] that makes the unit explicit.
pub const TARGET_BYTES_500_MB: u64 = TARGET_BYTES_500M;

/// Named decimal byte targets or a caller-provided byte count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TargetSize {
    Small,
    #[default]
    Medium,
    Large,
    Custom(u64),
}

impl TargetSize {
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Small => TARGET_BYTES_20M,
            Self::Medium => TARGET_BYTES_50M,
            Self::Large => TARGET_BYTES_500M,
            Self::Custom(bytes) => bytes,
        }
    }

    pub const fn custom(bytes: u64) -> Self {
        Self::Custom(bytes)
    }
}

impl From<u64> for TargetSize {
    fn from(bytes: u64) -> Self {
        Self::Custom(bytes)
    }
}

/// An exact rational ratio used for the target-size reserve.
///
/// The planner requires `numerator <= denominator` and rounds the ratio
/// reserve upward. Keeping the ratio integral avoids floating-point behavior
/// in the byte budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    pub numerator: u64,
    pub denominator: u64,
}

impl Default for Ratio {
    fn default() -> Self {
        Self::zero()
    }
}

impl Ratio {
    pub const fn new(numerator: u64, denominator: u64) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    pub const fn zero() -> Self {
        Self::new(0, 1)
    }

    pub const fn percent(percent: u64) -> Self {
        Self::new(percent, 100)
    }
}

/// Input video metadata used for no-upscale ladder selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoSource {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl VideoSource {
    pub const fn new(width: u32, height: u32, fps: u32) -> Self {
        Self { width, height, fps }
    }
}

/// One resolution/FPS choice in a planner ladder.
///
/// A ladder is evaluated in its declared order. The built-in ladder is
/// ordered from highest to lowest output quality; custom ladders can use a
/// different deterministic preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoRung {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl VideoRung {
    pub const fn new(width: u32, height: u32, fps: u32) -> Self {
        Self { width, height, fps }
    }

    pub const fn supports_no_upscale(self, source: VideoSource) -> bool {
        self.width > 0
            && self.height > 0
            && self.fps > 0
            && self.width <= source.width
            && self.height <= source.height
            && self.fps <= source.fps
    }
}

/// Default high-to-low resolution/FPS choices.
pub const DEFAULT_VIDEO_LADDER: &[VideoRung] = &[
    VideoRung::new(3_840, 2_160, 120),
    VideoRung::new(3_840, 2_160, 60),
    VideoRung::new(3_840, 2_160, 30),
    VideoRung::new(2_560, 1_440, 120),
    VideoRung::new(2_560, 1_440, 60),
    VideoRung::new(2_560, 1_440, 30),
    VideoRung::new(1_920, 1_080, 120),
    VideoRung::new(1_920, 1_080, 60),
    VideoRung::new(1_920, 1_080, 30),
    VideoRung::new(1_280, 720, 120),
    VideoRung::new(1_280, 720, 60),
    VideoRung::new(1_280, 720, 30),
    VideoRung::new(854, 480, 30),
    VideoRung::new(640, 360, 30),
];

/// Request for a deterministic target-byte plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannerRequest {
    pub target: TargetSize,
    pub duration_us: u64,
    /// Encoded bitrate in bits per second for each selected audio track.
    pub audio_bitrate_bps: u64,
    /// Number of selected audio tracks that will be written to the export.
    pub audio_track_count: u32,
    pub reserve_ratio: Ratio,
    pub fixed_reserve_bytes: u64,
    /// The smallest permitted calculated video bitrate in bits per second.
    pub video_bitrate_floor_bps: u64,
    pub source_width: u32,
    pub source_height: u32,
    pub source_fps: u32,
}

impl Default for PlannerRequest {
    fn default() -> Self {
        Self {
            target: TargetSize::default(),
            duration_us: 60_000_000,
            audio_bitrate_bps: 192_000,
            audio_track_count: 1,
            reserve_ratio: Ratio::zero(),
            fixed_reserve_bytes: 0,
            video_bitrate_floor_bps: 1_000_000,
            source_width: 1_920,
            source_height: 1_080,
            source_fps: 60,
        }
    }
}

impl PlannerRequest {
    pub fn new(
        target: TargetSize,
        duration_us: u64,
        audio_bitrate_bps: u64,
        source: VideoSource,
    ) -> Self {
        Self {
            target,
            duration_us,
            audio_bitrate_bps,
            source_width: source.width,
            source_height: source.height,
            source_fps: source.fps,
            ..Self::default()
        }
    }

    pub const fn source(self) -> VideoSource {
        VideoSource::new(self.source_width, self.source_height, self.source_fps)
    }

    pub const fn with_reserve_ratio(mut self, reserve_ratio: Ratio) -> Self {
        self.reserve_ratio = reserve_ratio;
        self
    }

    pub const fn with_fixed_reserve_bytes(mut self, fixed_reserve_bytes: u64) -> Self {
        self.fixed_reserve_bytes = fixed_reserve_bytes;
        self
    }

    pub const fn with_video_bitrate_floor_bps(mut self, video_bitrate_floor_bps: u64) -> Self {
        self.video_bitrate_floor_bps = video_bitrate_floor_bps;
        self
    }

    pub const fn with_audio_track_count(mut self, audio_track_count: u32) -> Self {
        self.audio_track_count = audio_track_count;
        self
    }

    pub const fn with_source(mut self, source: VideoSource) -> Self {
        self.source_width = source.width;
        self.source_height = source.height;
        self.source_fps = source.fps;
        self
    }
}

/// Short aliases for callers that prefer the input/plan vocabulary.
pub type PlannerInput = PlannerRequest;
pub type SourceVideo = VideoSource;

/// Result of target-byte planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportPlan {
    pub target_bytes: u64,
    pub reserve_bytes: u64,
    pub audio_bytes: u64,
    pub video_budget_bytes: u64,
    /// Maximum planned video bitrate that stays within the non-reserve budget.
    pub video_bitrate_bps: u64,
    pub duration_us: u64,
    /// Number of audio tracks accounted for by [`Self::audio_bytes`].
    pub audio_track_count: u32,
    /// Encoded bitrate in bits per second for each selected audio track.
    pub audio_bitrate_bps: u64,
    pub rung: VideoRung,
}

impl ExportPlan {
    pub const fn selected_width(self) -> u32 {
        self.rung.width
    }

    pub const fn selected_height(self) -> u32 {
        self.rung.height
    }

    pub const fn selected_fps(self) -> u32 {
        self.rung.fps
    }
}

pub type TargetBytePlan = ExportPlan;

/// Errors returned before an export backend would be touched.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlannerError {
    #[error("clip export duration must be greater than zero")]
    ZeroDuration,

    #[error("target byte count must be greater than zero, got {target_bytes}")]
    InvalidTarget { target_bytes: u64 },

    #[error("reserve ratio is invalid: {numerator}/{denominator}")]
    InvalidReserveRatio { numerator: u64, denominator: u64 },

    #[error("reserved bytes {reserved_bytes} exceed target {target_bytes}")]
    ReserveExceedsTarget {
        target_bytes: u64,
        reserved_bytes: u128,
    },

    #[error("target-byte arithmetic overflow while {operation}")]
    ArithmeticOverflow { operation: &'static str },

    #[error(
        "audio budget is impossible: {audio_bytes} audio bytes do not fit in {available_bytes} available bytes"
    )]
    ImpossibleAudioBudget {
        target_bytes: u64,
        reserved_bytes: u128,
        audio_bytes: u128,
        available_bytes: u128,
    },

    #[error(
        "no feasible video rung for source {source_width}x{source_height}@{source_fps} with {available_video_bitrate_bps} bps available and {video_bitrate_floor_bps} bps required"
    )]
    NoFeasibleRung {
        source_width: u32,
        source_height: u32,
        source_fps: u32,
        available_video_bitrate_bps: u128,
        video_bitrate_floor_bps: u64,
    },
}

impl PlannerError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ZeroDuration => "CLIP_EXPORT_ZERO_DURATION",
            Self::InvalidTarget { .. } => "CLIP_EXPORT_INVALID_TARGET",
            Self::InvalidReserveRatio { .. } => "CLIP_EXPORT_INVALID_RESERVE_RATIO",
            Self::ReserveExceedsTarget { .. } => "CLIP_EXPORT_RESERVE_EXCEEDS_TARGET",
            Self::ArithmeticOverflow { .. } => "CLIP_EXPORT_ARITHMETIC_OVERFLOW",
            Self::ImpossibleAudioBudget { .. } => "CLIP_EXPORT_IMPOSSIBLE_AUDIO_BUDGET",
            Self::NoFeasibleRung { .. } => "CLIP_EXPORT_NO_FEASIBLE_RUNG",
        }
    }
}

/// Planner that evaluates one deterministic resolution/FPS ladder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetBytePlanner {
    ladder: Vec<VideoRung>,
}

impl Default for TargetBytePlanner {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetBytePlanner {
    pub fn new() -> Self {
        Self::with_ladder(DEFAULT_VIDEO_LADDER.iter().copied())
    }

    pub fn with_ladder(ladder: impl IntoIterator<Item = VideoRung>) -> Self {
        Self {
            ladder: ladder.into_iter().collect(),
        }
    }

    pub fn ladder(&self) -> &[VideoRung] {
        &self.ladder
    }

    /// Return the first valid rung that does not upscale the source.
    pub fn select_rung(&self, source: VideoSource) -> Option<VideoRung> {
        self.ladder
            .iter()
            .copied()
            .find(|rung| rung.supports_no_upscale(source))
    }

    pub fn plan(&self, request: &PlannerRequest) -> Result<ExportPlan, PlannerError> {
        if request.duration_us == 0 {
            return Err(PlannerError::ZeroDuration);
        }

        let target_bytes = request.target.bytes();
        if target_bytes == 0 {
            return Err(PlannerError::InvalidTarget { target_bytes });
        }

        let ratio = request.reserve_ratio;
        if ratio.denominator == 0 || ratio.numerator > ratio.denominator {
            return Err(PlannerError::InvalidReserveRatio {
                numerator: ratio.numerator,
                denominator: ratio.denominator,
            });
        }

        let target = u128::from(target_bytes);
        let ratio_numerator = checked_mul(
            target,
            u128::from(ratio.numerator),
            "multiplying target by reserve ratio",
        )?;
        let ratio_bytes = checked_ceil_div(
            ratio_numerator,
            u128::from(ratio.denominator),
            "rounding reserve ratio",
        )?;
        let reserved_bytes = checked_add(
            ratio_bytes,
            u128::from(request.fixed_reserve_bytes),
            "adding fixed reserve",
        )?;
        if reserved_bytes > target {
            return Err(PlannerError::ReserveExceedsTarget {
                target_bytes,
                reserved_bytes,
            });
        }

        let available_after_reserve =
            target
                .checked_sub(reserved_bytes)
                .ok_or(PlannerError::ArithmeticOverflow {
                    operation: "subtracting reserve from target",
                })?;
        let audio_track_numerator = checked_mul(
            u128::from(request.audio_bitrate_bps),
            u128::from(request.duration_us),
            "multiplying audio bitrate by duration",
        )?;
        let audio_numerator = checked_mul(
            audio_track_numerator,
            u128::from(request.audio_track_count),
            "multiplying audio bitrate by track count",
        )?;
        let audio_bytes = checked_ceil_div(
            audio_numerator,
            BITS_PER_BYTE * MICROS_PER_SECOND,
            "rounding audio bytes",
        )?;
        if audio_bytes > available_after_reserve {
            return Err(PlannerError::ImpossibleAudioBudget {
                target_bytes,
                reserved_bytes,
                audio_bytes,
                available_bytes: available_after_reserve,
            });
        }

        let video_budget = available_after_reserve.checked_sub(audio_bytes).ok_or(
            PlannerError::ArithmeticOverflow {
                operation: "subtracting audio from media budget",
            },
        )?;
        let video_numerator = checked_mul(
            video_budget,
            BITS_PER_BYTE,
            "converting video byte budget to bits",
        )?;
        let video_numerator = checked_mul(
            video_numerator,
            MICROS_PER_SECOND,
            "converting video budget to per-second bitrate",
        )?;
        let available_video_bitrate_bps = video_numerator / u128::from(request.duration_us);
        let source = request.source();
        let Some(rung) = self.select_rung(source) else {
            return Err(no_feasible_rung(
                source,
                available_video_bitrate_bps,
                request.video_bitrate_floor_bps,
            ));
        };
        if available_video_bitrate_bps == 0
            || available_video_bitrate_bps < u128::from(request.video_bitrate_floor_bps)
        {
            return Err(no_feasible_rung(
                source,
                available_video_bitrate_bps,
                request.video_bitrate_floor_bps,
            ));
        }

        let video_bitrate_bps = u64::try_from(available_video_bitrate_bps).map_err(|_| {
            PlannerError::ArithmeticOverflow {
                operation: "converting video bitrate to u64",
            }
        })?;

        Ok(ExportPlan {
            target_bytes,
            reserve_bytes: u64::try_from(reserved_bytes).map_err(|_| {
                PlannerError::ArithmeticOverflow {
                    operation: "converting reserve bytes to u64",
                }
            })?,
            audio_bytes: u64::try_from(audio_bytes).map_err(|_| {
                PlannerError::ArithmeticOverflow {
                    operation: "converting audio bytes to u64",
                }
            })?,
            video_budget_bytes: u64::try_from(video_budget).map_err(|_| {
                PlannerError::ArithmeticOverflow {
                    operation: "converting video budget to u64",
                }
            })?,
            video_bitrate_bps,
            duration_us: request.duration_us,
            audio_track_count: request.audio_track_count,
            audio_bitrate_bps: request.audio_bitrate_bps,
            rung,
        })
    }
}

/// Plan with the built-in ladder.
pub fn plan(request: &PlannerRequest) -> Result<ExportPlan, PlannerError> {
    TargetBytePlanner::new().plan(request)
}

fn no_feasible_rung(
    source: VideoSource,
    available_video_bitrate_bps: u128,
    video_bitrate_floor_bps: u64,
) -> PlannerError {
    PlannerError::NoFeasibleRung {
        source_width: source.width,
        source_height: source.height,
        source_fps: source.fps,
        available_video_bitrate_bps,
        video_bitrate_floor_bps,
    }
}

fn checked_mul(left: u128, right: u128, operation: &'static str) -> Result<u128, PlannerError> {
    left.checked_mul(right)
        .ok_or(PlannerError::ArithmeticOverflow { operation })
}

fn checked_add(left: u128, right: u128, operation: &'static str) -> Result<u128, PlannerError> {
    left.checked_add(right)
        .ok_or(PlannerError::ArithmeticOverflow { operation })
}

fn checked_ceil_div(
    numerator: u128,
    denominator: u128,
    operation: &'static str,
) -> Result<u128, PlannerError> {
    if denominator == 0 {
        return Err(PlannerError::ArithmeticOverflow { operation });
    }
    let quotient = numerator / denominator;
    if numerator.is_multiple_of(denominator) {
        Ok(quotient)
    } else {
        checked_add(quotient, 1, operation)
    }
}

pub const SMALL_TARGET_BYTES: u64 = TARGET_BYTES_20M;
pub const MEDIUM_TARGET_BYTES: u64 = TARGET_BYTES_50M;
pub const LARGE_TARGET_BYTES: u64 = TARGET_BYTES_500M;

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request() -> PlannerRequest {
        PlannerRequest::new(
            TargetSize::Medium,
            60_000_000,
            192_000,
            VideoSource::new(1_920, 1_080, 60),
        )
        .with_reserve_ratio(Ratio::new(1, 20))
        .with_fixed_reserve_bytes(1_000)
        .with_video_bitrate_floor_bps(1_000_000)
    }

    #[test]
    fn presets_are_exact_decimal_byte_counts() {
        assert_eq!(TargetSize::Small.bytes(), 20_000_000);
        assert_eq!(TargetSize::Medium.bytes(), 50_000_000);
        assert_eq!(TargetSize::Large.bytes(), 500_000_000);
        assert_eq!(TargetSize::Custom(123_456_789).bytes(), 123_456_789);
        assert_eq!(SMALL_TARGET_BYTES, 20_000_000);
        assert_eq!(MEDIUM_TARGET_BYTES, 50_000_000);
        assert_eq!(LARGE_TARGET_BYTES, 500_000_000);
    }

    #[test]
    fn plan_uses_ceil_reserve_audio_and_floor_video_arithmetic() {
        let result = TargetBytePlanner::new()
            .plan(&base_request())
            .expect("plan");

        assert_eq!(result.target_bytes, 50_000_000);
        assert_eq!(result.reserve_bytes, 2_501_000);
        assert_eq!(result.audio_bytes, 1_440_000);
        assert_eq!(result.video_budget_bytes, 46_059_000);
        assert_eq!(result.video_bitrate_bps, 6_141_200);
        assert_eq!(result.rung, VideoRung::new(1_920, 1_080, 60));
    }

    #[test]
    fn custom_target_and_source_metadata_choose_a_no_upscale_rung() {
        let request = PlannerRequest::new(
            TargetSize::custom(20_000_000),
            10_000_000,
            128_000,
            VideoSource::new(1_280, 720, 30),
        )
        .with_video_bitrate_floor_bps(100_000);

        let result = plan(&request).expect("plan");

        assert_eq!(result.target_bytes, 20_000_000);
        assert_eq!(result.rung, VideoRung::new(1_280, 720, 30));
        assert!(result.rung.width <= request.source_width);
        assert!(result.rung.height <= request.source_height);
        assert!(result.rung.fps <= request.source_fps);
    }

    #[test]
    fn fps_is_selected_without_upscaling() {
        let thirty = PlannerRequest::new(
            TargetSize::Medium,
            60_000_000,
            192_000,
            VideoSource::new(1_920, 1_080, 30),
        );
        assert_eq!(plan(&thirty).expect("30 fps plan").rung.fps, 30);

        let small = PlannerRequest::new(
            TargetSize::Medium,
            60_000_000,
            192_000,
            VideoSource::new(640, 360, 30),
        );
        assert_eq!(
            plan(&small).expect("small plan").rung,
            VideoRung::new(640, 360, 30)
        );
    }

    #[test]
    fn custom_ladder_order_is_deterministic() {
        let planner = TargetBytePlanner::with_ladder([
            VideoRung::new(1_280, 720, 30),
            VideoRung::new(1_920, 1_080, 30),
        ]);
        let request = PlannerRequest::new(
            TargetSize::Medium,
            60_000_000,
            192_000,
            VideoSource::new(1_920, 1_080, 30),
        );

        assert_eq!(planner.plan(&request).expect("plan").rung.width, 1_280);
    }

    #[test]
    fn ratio_rounding_is_conservative() {
        let request = PlannerRequest::new(
            TargetSize::Small,
            10_000_000,
            0,
            VideoSource::new(1_920, 1_080, 60),
        )
        .with_reserve_ratio(Ratio::new(1, 3));

        assert_eq!(plan(&request).expect("plan").reserve_bytes, 6_666_667);
    }

    #[test]
    fn audio_budget_accounts_for_every_selected_track() {
        let request = PlannerRequest::new(
            TargetSize::Medium,
            60_000_000,
            192_000,
            VideoSource::new(1_920, 1_080, 60),
        )
        .with_audio_track_count(2);

        let result = plan(&request).expect("two-track plan");

        assert_eq!(result.audio_track_count, 2);
        assert_eq!(result.audio_bytes, 2_880_000);
        assert_eq!(result.video_budget_bytes, 47_120_000);
        assert_eq!(result.video_bitrate_bps, 6_282_666);
    }

    #[test]
    fn rejects_zero_duration() {
        let request = PlannerRequest {
            duration_us: 0,
            ..base_request()
        };

        assert!(matches!(plan(&request), Err(PlannerError::ZeroDuration)));
    }

    #[test]
    fn rejects_an_impossible_audio_budget() {
        let request = PlannerRequest::new(
            TargetSize::Small,
            60_000_000,
            3_000_000,
            VideoSource::new(1_920, 1_080, 60),
        );

        assert!(matches!(
            plan(&request),
            Err(PlannerError::ImpossibleAudioBudget { .. })
        ));
    }

    #[test]
    fn reports_no_feasible_rung_for_source_or_bitrate_floor() {
        let unsupported_source = PlannerRequest::new(
            TargetSize::Medium,
            60_000_000,
            192_000,
            VideoSource::new(320, 240, 24),
        );
        assert!(matches!(
            plan(&unsupported_source),
            Err(PlannerError::NoFeasibleRung { .. })
        ));

        let too_expensive = PlannerRequest {
            video_bitrate_floor_bps: 10_000_000,
            ..base_request()
        };
        assert!(matches!(
            plan(&too_expensive),
            Err(PlannerError::NoFeasibleRung { .. })
        ));
    }

    #[test]
    fn reports_invalid_target_and_reserve_ratio() {
        let zero_target = PlannerRequest {
            target: TargetSize::Custom(0),
            ..base_request()
        };
        assert!(matches!(
            plan(&zero_target),
            Err(PlannerError::InvalidTarget { .. })
        ));

        let invalid_ratio = PlannerRequest {
            reserve_ratio: Ratio::new(2, 1),
            ..base_request()
        };
        assert!(matches!(
            plan(&invalid_ratio),
            Err(PlannerError::InvalidReserveRatio { .. })
        ));
    }

    #[test]
    fn reports_checked_u128_overflow() {
        let reserve_exceeds_target = PlannerRequest {
            target: TargetSize::Custom(u64::MAX),
            reserve_ratio: Ratio::new(1, 1),
            fixed_reserve_bytes: u64::MAX,
            ..base_request()
        };
        assert!(matches!(
            plan(&reserve_exceeds_target),
            Err(PlannerError::ReserveExceedsTarget { .. })
        ));

        let bitrate_overflow = PlannerRequest {
            target: TargetSize::Custom(u64::MAX),
            duration_us: 1,
            reserve_ratio: Ratio::zero(),
            fixed_reserve_bytes: 0,
            audio_bitrate_bps: 0,
            ..base_request()
        };
        assert!(matches!(
            plan(&bitrate_overflow),
            Err(PlannerError::ArithmeticOverflow { .. })
        ));
    }
}
