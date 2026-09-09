//! Geometry, layout constants, DPI scaling, and screen anchor calculations.

use crate::types::{HudAnchor, HudMode};

/// Canvas width in Device-Independent Pixels (DIPs).
pub const CANVAS_WIDTH_DIP: f32 = 320.0;

/// Canvas height in Device-Independent Pixels (DIPs).
pub const CANVAS_HEIGHT_DIP: f32 = 56.0;

/// Outer container corner radius in DIPs.
pub const CONTAINER_RADIUS_DIP: f32 = 12.0;

/// Outer container border stroke width in DIPs.
pub const CONTAINER_BORDER_STROKE_DIP: f32 = 1.0;

/// Screen edge margin in DIPs.
pub const MARGIN_DIP: f32 = 32.0;

/// Badge width in DIPs.
pub const BADGE_WIDTH_DIP: f32 = 34.0;

/// Badge height in DIPs.
pub const BADGE_HEIGHT_DIP: f32 = 34.0;

/// Badge corner radius in DIPs.
pub const BADGE_RADIUS_DIP: f32 = 8.0;

/// Badge left margin in DIPs.
pub const BADGE_LEFT_DIP: f32 = 14.0;

/// Badge top margin in DIPs.
pub const BADGE_TOP_DIP: f32 = 11.0;

/// Badge center X coordinate in DIPs.
pub const BADGE_CENTER_X_DIP: f32 = 31.0;

/// Badge center Y coordinate in DIPs.
pub const BADGE_CENTER_Y_DIP: f32 = 28.0;

/// Text block left coordinate in DIPs.
pub const TEXT_BLOCK_LEFT_DIP: f32 = 60.0;

/// Text block top coordinate in DIPs.
pub const TEXT_BLOCK_TOP_DIP: f32 = 12.0;

/// Text block right coordinate in DIPs.
pub const TEXT_BLOCK_RIGHT_DIP: f32 = 304.0;

/// Text block bottom coordinate in DIPs.
pub const TEXT_BLOCK_BOTTOM_DIP: f32 = 44.0;

/// Title font size in DIPs.
pub const TITLE_FONT_SIZE_DIP: f32 = 13.5;

/// Subtitle font size in DIPs.
pub const SUBTITLE_FONT_SIZE_DIP: f32 = 11.0;

/// Title line height in DIPs.
pub const TITLE_LINE_HEIGHT_DIP: f32 = 17.0;

/// Subtitle line height in DIPs.
pub const SUBTITLE_LINE_HEIGHT_DIP: f32 = 15.0;

/// Entrance slide and fade duration in milliseconds.
pub const ENTRANCE_DURATION_MS: u64 = 180;

/// Exit fade duration in milliseconds.
pub const EXIT_DURATION_MS: u64 = 200;

/// Hold duration for saved cue in milliseconds.
pub const HOLD_SAVED_DURATION_MS: u64 = 2200;

/// Hold duration for failed cue in milliseconds.
pub const HOLD_FAILED_DURATION_MS: u64 = 3200;

/// Maximum auto-hold duration for queued state in milliseconds before auto-dismissal.
pub const MAX_QUEUED_HOLD_MS: u64 = 5000;

/// Reduced-motion fade duration in milliseconds.
pub const REDUCED_MOTION_FADE_DURATION_MS: u64 = 60;

/// Entrance translation distance in DIPs for standard motion.
pub const ENTRANCE_TRANSLATION_DIP: f32 = 8.0;

/// Title maximum character limit.
pub const TITLE_MAX_CHARS: usize = 28;

/// Subtitle maximum character limit before trimming.
pub const SUBTITLE_MAX_CHARS: usize = 38;

// --- Compact Mode Layout Constants (Section 2.3 & 5.3) ---

/// Compact pill visible width in Device-Independent Pixels (DIPs).
pub const COMPACT_WIDTH_DIP: f32 = 180.0;

/// Compact pill visible height in Device-Independent Pixels (DIPs).
pub const COMPACT_HEIGHT_DIP: f32 = 38.0;

/// Compact pill corner radius in DIPs (half height for full capsule).
pub const COMPACT_RADIUS_DIP: f32 = 19.0;

/// Compact pill border stroke width in DIPs.
pub const COMPACT_BORDER_STROKE_DIP: f32 = 1.0;

/// Compact badge container width and height in DIPs.
pub const COMPACT_BADGE_SIZE_DIP: f32 = 24.0;

/// Compact badge corner radius in DIPs.
pub const COMPACT_BADGE_RADIUS_DIP: f32 = 6.0;

/// Compact badge inset margin in DIPs from pill left and top.
pub const COMPACT_BADGE_MARGIN_DIP: f32 = 7.0;

/// Compact badge-to-text gap in DIPs.
pub const COMPACT_BADGE_TO_TEXT_GAP_DIP: f32 = 7.0;

/// Compact badge center X in local pill coordinates (7.0 + 12.0).
pub const COMPACT_BADGE_CENTER_X_DIP: f32 = 19.0;

/// Compact badge center Y in local pill coordinates (7.0 + 12.0).
pub const COMPACT_BADGE_CENTER_Y_DIP: f32 = 19.0;

/// Compact title text left coordinate in local pill coordinates.
pub const COMPACT_TEXT_LEFT_DIP: f32 = 38.0;

/// Compact title text top coordinate in local pill coordinates.
pub const COMPACT_TEXT_TOP_DIP: f32 = 11.0;

/// Compact title text right coordinate in local pill coordinates.
pub const COMPACT_TEXT_RIGHT_DIP: f32 = 168.0;

/// Compact title text bottom coordinate in local pill coordinates.
pub const COMPACT_TEXT_BOTTOM_DIP: f32 = 27.0;

/// Compact title font size in DIPs.
pub const COMPACT_TITLE_FONT_SIZE_DIP: f32 = 12.0;

/// Compact title line spacing in DIPs.
pub const COMPACT_TITLE_LINE_SPACING_DIP: f32 = 16.0;

/// Compact title maximum character limit before trimming.
pub const COMPACT_TITLE_MAX_CHARS: usize = 20;

/// Represents a monitor work area rectangle in physical screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WorkArea {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Computes the DPI scale factor relative to standard 96 DPI.
pub fn dpi_scale_factor(dpi: u32) -> f32 {
    dpi as f32 / 96.0
}

/// Computes physical surface dimensions (width, height) in physical pixels for a given DPI.
pub fn physical_size(dpi: u32) -> (u32, u32) {
    physical_size_for_mode(HudMode::Full, dpi)
}

/// Computes physical surface dimensions (width, height) in physical pixels for a given HUD mode and DPI.
pub fn physical_size_for_mode(mode: HudMode, dpi: u32) -> (u32, u32) {
    let scale = dpi_scale_factor(dpi);
    let (w_dip, h_dip) = match mode {
        HudMode::Full => (CANVAS_WIDTH_DIP, CANVAS_HEIGHT_DIP),
        HudMode::Compact => (COMPACT_WIDTH_DIP, COMPACT_HEIGHT_DIP),
    };
    let width = (w_dip * scale).round() as u32;
    let height = (h_dip * scale).round() as u32;
    (width, height)
}

/// Computes physical margin in physical pixels for a given DPI.
pub fn physical_margin(dpi: u32) -> i32 {
    let scale = dpi_scale_factor(dpi);
    (MARGIN_DIP * scale).round() as i32
}

/// Calculates the physical top-left window coordinates `(x, y)` for a given anchor,
/// monitor work area, and DPI setting per Section 9.2 (Full mode).
pub fn calculate_anchor_position(anchor: HudAnchor, work_area: WorkArea, dpi: u32) -> (i32, i32) {
    calculate_anchor_position_for_mode(anchor, HudMode::Full, work_area, dpi)
}

/// Calculates the physical top-left window coordinates `(x, y)` for a given anchor,
/// mode, monitor work area, and DPI setting.
pub fn calculate_anchor_position_for_mode(
    anchor: HudAnchor,
    mode: HudMode,
    work_area: WorkArea,
    dpi: u32,
) -> (i32, i32) {
    let (w_phys, h_phys) = physical_size_for_mode(mode, dpi);
    let w_phys = w_phys as i32;
    let h_phys = h_phys as i32;
    let m_phys = physical_margin(dpi);

    let w_work = work_area.width as i32;
    let h_work = work_area.height as i32;
    let x_work = work_area.x;
    let y_work = work_area.y;

    let center_x = x_work + ((w_work - w_phys) as f32 / 2.0).round() as i32;
    let center_y = y_work + ((h_work - h_phys) as f32 / 2.0).round() as i32;
    let right_x = x_work + w_work - w_phys - m_phys;
    let bottom_y = y_work + h_work - h_phys - m_phys;
    let left_x = x_work + m_phys;
    let top_y = y_work + m_phys;

    match anchor {
        HudAnchor::TopLeft => (left_x, top_y),
        HudAnchor::TopCenter => (center_x, top_y),
        HudAnchor::TopRight => (right_x, top_y),
        HudAnchor::CenterLeft => (left_x, center_y),
        HudAnchor::CenterRight => (right_x, center_y),
        HudAnchor::BottomLeft => (left_x, bottom_y),
        HudAnchor::BottomCenter => (center_x, bottom_y),
        HudAnchor::BottomRight => (right_x, bottom_y),
    }
}

/// Returns the `[left, top, right, bottom]` in-canvas rectangle in DIPs for the compact pill
/// for the specified anchor per Section 2.3.
pub const fn compact_pill_rect(anchor: HudAnchor) -> (f32, f32, f32, f32) {
    let (x, y) = anchor.compact_offset_dip();
    (x, y, x + COMPACT_WIDTH_DIP, y + COMPACT_HEIGHT_DIP)
}

/// Calculates the physical screen margin (left/right, top/bottom) from the visible pill edges
/// to the display work area perimeter.
///
/// Returns `(horizontal_margin, vertical_margin)` in physical pixels.
/// For edge-anchored axes, this margin is identically equal to `physical_margin(dpi)`.
pub fn calculate_compact_screen_margins(
    anchor: HudAnchor,
    work_area: WorkArea,
    dpi: u32,
) -> (i32, i32) {
    let (win_x, win_y) = calculate_anchor_position(anchor, work_area, dpi);
    let scale = dpi_scale_factor(dpi);
    let (offset_x, offset_y) = anchor.compact_offset_dip();
    let pill_left = win_x + (offset_x * scale).round() as i32;
    let pill_right = win_x + ((offset_x + COMPACT_WIDTH_DIP) * scale).round() as i32;
    let pill_top = win_y + (offset_y * scale).round() as i32;
    let pill_bottom = win_y + ((offset_y + COMPACT_HEIGHT_DIP) * scale).round() as i32;

    let h_margin = if anchor.is_left() {
        pill_left - work_area.x
    } else if anchor.is_right() {
        (work_area.x + work_area.width as i32) - pill_right
    } else {
        pill_left - work_area.x
    };

    let v_margin = if anchor.is_top() {
        pill_top - work_area.y
    } else if anchor.is_bottom() {
        (work_area.y + work_area.height as i32) - pill_bottom
    } else {
        pill_top - work_area.y
    };

    (h_margin, v_margin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_offsets_all_eight_anchors() {
        assert_eq!(HudAnchor::TopLeft.compact_offset_dip(), (0.0, 0.0));
        assert_eq!(HudAnchor::TopCenter.compact_offset_dip(), (70.0, 0.0));
        assert_eq!(HudAnchor::TopRight.compact_offset_dip(), (140.0, 0.0));
        assert_eq!(HudAnchor::CenterLeft.compact_offset_dip(), (0.0, 9.0));
        assert_eq!(HudAnchor::CenterRight.compact_offset_dip(), (140.0, 9.0));
        assert_eq!(HudAnchor::BottomLeft.compact_offset_dip(), (0.0, 18.0));
        assert_eq!(HudAnchor::BottomCenter.compact_offset_dip(), (70.0, 18.0));
        assert_eq!(HudAnchor::BottomRight.compact_offset_dip(), (140.0, 18.0));

        assert_eq!(
            compact_pill_rect(HudAnchor::TopLeft),
            (0.0, 0.0, 180.0, 38.0)
        );
        assert_eq!(
            compact_pill_rect(HudAnchor::TopCenter),
            (70.0, 0.0, 250.0, 38.0)
        );
        assert_eq!(
            compact_pill_rect(HudAnchor::TopRight),
            (140.0, 0.0, 320.0, 38.0)
        );
        assert_eq!(
            compact_pill_rect(HudAnchor::CenterLeft),
            (0.0, 9.0, 180.0, 47.0)
        );
        assert_eq!(
            compact_pill_rect(HudAnchor::CenterRight),
            (140.0, 9.0, 320.0, 47.0)
        );
        assert_eq!(
            compact_pill_rect(HudAnchor::BottomLeft),
            (0.0, 18.0, 180.0, 56.0)
        );
        assert_eq!(
            compact_pill_rect(HudAnchor::BottomCenter),
            (70.0, 18.0, 250.0, 56.0)
        );
        assert_eq!(
            compact_pill_rect(HudAnchor::BottomRight),
            (140.0, 18.0, 320.0, 56.0)
        );
    }

    #[test]
    fn compact_screen_margin_invariance_across_dpi_scales() {
        let dpi_scales = [96, 120, 144, 168, 192];
        let work = WorkArea::new(0, 0, 3840, 2160);

        for &dpi in &dpi_scales {
            let m_phys = physical_margin(dpi);

            // Left anchors
            for anchor in [
                HudAnchor::TopLeft,
                HudAnchor::CenterLeft,
                HudAnchor::BottomLeft,
            ] {
                let (h_margin, _) = calculate_compact_screen_margins(anchor, work, dpi);
                assert_eq!(
                    h_margin, m_phys,
                    "Left anchor {anchor:?} at {dpi} DPI must match m_phys"
                );
            }

            // Right anchors
            for anchor in [
                HudAnchor::TopRight,
                HudAnchor::CenterRight,
                HudAnchor::BottomRight,
            ] {
                let (h_margin, _) = calculate_compact_screen_margins(anchor, work, dpi);
                assert_eq!(
                    h_margin, m_phys,
                    "Right anchor {anchor:?} at {dpi} DPI must match m_phys"
                );
            }

            // Top anchors
            for anchor in [
                HudAnchor::TopLeft,
                HudAnchor::TopCenter,
                HudAnchor::TopRight,
            ] {
                let (_, v_margin) = calculate_compact_screen_margins(anchor, work, dpi);
                assert_eq!(
                    v_margin, m_phys,
                    "Top anchor {anchor:?} at {dpi} DPI must match m_phys"
                );
            }

            // Bottom anchors
            for anchor in [
                HudAnchor::BottomLeft,
                HudAnchor::BottomCenter,
                HudAnchor::BottomRight,
            ] {
                let (_, v_margin) = calculate_compact_screen_margins(anchor, work, dpi);
                assert_eq!(
                    v_margin, m_phys,
                    "Bottom anchor {anchor:?} at {dpi} DPI must match m_phys"
                );
            }
        }
    }

    #[test]
    fn physical_dimensions_at_common_scales() {
        // 100% (96 DPI) -> 320x56, margin 32
        assert_eq!(physical_size(96), (320, 56));
        assert_eq!(physical_margin(96), 32);

        // 125% (120 DPI) -> 400x70, margin 40
        assert_eq!(physical_size(120), (400, 70));
        assert_eq!(physical_margin(120), 40);

        // 150% (144 DPI) -> 480x84, margin 48
        assert_eq!(physical_size(144), (480, 84));
        assert_eq!(physical_margin(144), 48);

        // 175% (168 DPI) -> 560x98, margin 56
        assert_eq!(physical_size(168), (560, 98));
        assert_eq!(physical_margin(168), 56);

        // 200% (192 DPI) -> 640x112, margin 64
        assert_eq!(physical_size(192), (640, 112));
        assert_eq!(physical_margin(192), 64);
    }

    #[test]
    fn anchor_positions_1080p_100_percent() {
        // 1920x1080 work area at 96 DPI
        let work = WorkArea::new(0, 0, 1920, 1080);
        let dpi = 96;

        // TopLeft: (32, 32)
        assert_eq!(
            calculate_anchor_position(HudAnchor::TopLeft, work, dpi),
            (32, 32)
        );

        // TopCenter: (round((1920 - 320)/2), 32) = (800, 32)
        assert_eq!(
            calculate_anchor_position(HudAnchor::TopCenter, work, dpi),
            (800, 32)
        );

        // TopRight: (1920 - 320 - 32, 32) = (1568, 32)
        assert_eq!(
            calculate_anchor_position(HudAnchor::TopRight, work, dpi),
            (1568, 32)
        );

        // CenterLeft: (32, round((1080 - 56)/2)) = (32, 512)
        assert_eq!(
            calculate_anchor_position(HudAnchor::CenterLeft, work, dpi),
            (32, 512)
        );

        // CenterRight: (1568, 512)
        assert_eq!(
            calculate_anchor_position(HudAnchor::CenterRight, work, dpi),
            (1568, 512)
        );

        // BottomLeft: (32, 1080 - 56 - 32) = (32, 992)
        assert_eq!(
            calculate_anchor_position(HudAnchor::BottomLeft, work, dpi),
            (32, 992)
        );

        // BottomCenter (default): (800, 992)
        assert_eq!(
            calculate_anchor_position(HudAnchor::BottomCenter, work, dpi),
            (800, 992)
        );

        // BottomRight: (1568, 992)
        assert_eq!(
            calculate_anchor_position(HudAnchor::BottomRight, work, dpi),
            (1568, 992)
        );
    }

    #[test]
    fn anchor_positions_with_offset_work_area_150_percent() {
        // Offset taskbar: e.g. x = 0, y = 40, w = 2560, h = 1400 at 144 DPI (150%)
        let work = WorkArea::new(0, 40, 2560, 1400);
        let dpi = 144;

        // TopLeft: (48, 40 + 48) = (48, 88)
        assert_eq!(
            calculate_anchor_position(HudAnchor::TopLeft, work, dpi),
            (48, 88)
        );

        // BottomCenter: (0 + round((2560 - 480)/2), 40 + 1400 - 84 - 48) = (1040, 1308)
        assert_eq!(
            calculate_anchor_position(HudAnchor::BottomCenter, work, dpi),
            (1040, 1308)
        );
    }
}
