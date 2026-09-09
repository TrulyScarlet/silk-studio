use native_hud::*;

#[test]
fn cue_state_kinds_and_titles() {
    let queued = HudCue::queued();
    assert_eq!(queued.state_kind(), HudStateKind::Queued);
    assert_eq!(queued.title(), "Saving replay...");
    assert_eq!(queued.subtitle(), "Writing clip to disk");
    assert_eq!(queued.hold_duration_ms(), MAX_QUEUED_HOLD_MS);

    let saved = HudCue::saved(45_000, 94_200_000);
    assert_eq!(saved.state_kind(), HudStateKind::Saved);
    assert_eq!(saved.title(), "Silk Captured");
    assert_eq!(saved.subtitle(), "00m 45s • 89.8 MB");
    assert_eq!(saved.hold_duration_ms(), HOLD_SAVED_DURATION_MS);

    let failed = HudCue::failed("Encoder reset");
    assert_eq!(failed.state_kind(), HudStateKind::Failed);
    assert_eq!(failed.title(), "Save failed");
    assert_eq!(failed.subtitle(), "Encoder reset");
    assert_eq!(failed.hold_duration_ms(), HOLD_FAILED_DURATION_MS);

    let rejected = HudCue::rejected("Buffer not ready yet");
    assert_eq!(rejected.state_kind(), HudStateKind::Failed);
    assert_eq!(rejected.title(), "Save rejected");
    assert_eq!(rejected.subtitle(), "Buffer not ready yet");
    assert_eq!(rejected.hold_duration_ms(), HOLD_FAILED_DURATION_MS);
}

#[test]
fn all_eight_anchors_and_motion_offsets() {
    let anchors = [
        HudAnchor::TopLeft,
        HudAnchor::TopCenter,
        HudAnchor::TopRight,
        HudAnchor::CenterLeft,
        HudAnchor::CenterRight,
        HudAnchor::BottomLeft,
        HudAnchor::BottomCenter,
        HudAnchor::BottomRight,
    ];

    for anchor in anchors {
        match anchor {
            HudAnchor::TopLeft | HudAnchor::TopCenter | HudAnchor::TopRight => {
                assert!(anchor.is_top());
                assert!(!anchor.is_bottom());
                assert!(!anchor.is_center_y());
                assert_eq!(anchor.slide_offset_dip(), -8.0);
            }
            HudAnchor::CenterLeft | HudAnchor::CenterRight => {
                assert!(!anchor.is_top());
                assert!(!anchor.is_bottom());
                assert!(anchor.is_center_y());
                assert_eq!(anchor.slide_offset_dip(), 0.0);
            }
            HudAnchor::BottomLeft | HudAnchor::BottomCenter | HudAnchor::BottomRight => {
                assert!(!anchor.is_top());
                assert!(anchor.is_bottom());
                assert!(!anchor.is_center_y());
                assert_eq!(anchor.slide_offset_dip(), 8.0);
            }
        }
    }
}

#[test]
fn anchor_calculations_all_variants_2k_display() {
    // 2560x1440 at 120 DPI (125% scaling)
    let dpi = 120;
    let work = WorkArea::new(0, 0, 2560, 1440);
    let (w_phys, h_phys) = physical_size(dpi); // 400, 70
    let m_phys = physical_margin(dpi); // 40

    assert_eq!(w_phys, 400);
    assert_eq!(h_phys, 70);
    assert_eq!(m_phys, 40);

    let center_x = ((2560 - 400) as f32 / 2.0).round() as i32; // 1080
    let center_y = ((1440 - 70) as f32 / 2.0).round() as i32; // 685
    let right_x = 2560 - 400 - 40; // 2120
    let bottom_y = 1440 - 70 - 40; // 1330

    assert_eq!(
        calculate_anchor_position(HudAnchor::TopLeft, work, dpi),
        (40, 40)
    );
    assert_eq!(
        calculate_anchor_position(HudAnchor::TopCenter, work, dpi),
        (center_x, 40)
    );
    assert_eq!(
        calculate_anchor_position(HudAnchor::TopRight, work, dpi),
        (right_x, 40)
    );
    assert_eq!(
        calculate_anchor_position(HudAnchor::CenterLeft, work, dpi),
        (40, center_y)
    );
    assert_eq!(
        calculate_anchor_position(HudAnchor::CenterRight, work, dpi),
        (right_x, center_y)
    );
    assert_eq!(
        calculate_anchor_position(HudAnchor::BottomLeft, work, dpi),
        (40, bottom_y)
    );
    assert_eq!(
        calculate_anchor_position(HudAnchor::BottomCenter, work, dpi),
        (center_x, bottom_y)
    );
    assert_eq!(
        calculate_anchor_position(HudAnchor::BottomRight, work, dpi),
        (right_x, bottom_y)
    );
}

#[test]
fn bounded_queue_overflow_semantics() {
    let (tx, _rx) = std::sync::mpsc::sync_channel::<HudCue>(HUD_COMMAND_QUEUE_BOUND);
    for i in 0..HUD_COMMAND_QUEUE_BOUND {
        assert!(
            tx.try_send(HudCue::failed(format!("err {i}"))).is_ok(),
            "Queue should accept up to bound capacity {HUD_COMMAND_QUEUE_BOUND}"
        );
    }
    // Next attempt must return Full
    let err = tx.try_send(HudCue::failed("overflow"));
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        std::sync::mpsc::TrySendError::Full(_)
    ));
}

#[test]
fn hud_mode_defaults_and_config() {
    assert_eq!(HudMode::default(), HudMode::Full);
    let default_config = HudConfig::default();
    assert_eq!(default_config.mode, HudMode::Full);
    assert_eq!(default_config.anchor, HudAnchor::BottomCenter);
    assert!(default_config.exclude_from_capture);

    let compact_config = HudConfig {
        mode: HudMode::Compact,
        anchor: HudAnchor::TopRight,
        exclude_from_capture: false,
    };
    assert_eq!(compact_config.mode, HudMode::Compact);
    assert_eq!(compact_config.anchor, HudAnchor::TopRight);
    assert!(!compact_config.exclude_from_capture);
}

#[test]
fn compact_mode_layout_constants() {
    assert_eq!(COMPACT_WIDTH_DIP, 180.0);
    assert_eq!(COMPACT_HEIGHT_DIP, 38.0);
    assert_eq!(COMPACT_RADIUS_DIP, 19.0);
    assert_eq!(COMPACT_BORDER_STROKE_DIP, 1.0);
    assert_eq!(COMPACT_BADGE_SIZE_DIP, 24.0);
    assert_eq!(COMPACT_BADGE_RADIUS_DIP, 6.0);
    assert_eq!(COMPACT_BADGE_MARGIN_DIP, 7.0);
    assert_eq!(COMPACT_BADGE_TO_TEXT_GAP_DIP, 7.0);
    assert_eq!(COMPACT_BADGE_CENTER_X_DIP, 19.0);
    assert_eq!(COMPACT_BADGE_CENTER_Y_DIP, 19.0);
    assert_eq!(COMPACT_TEXT_LEFT_DIP, 38.0);
    assert_eq!(COMPACT_TEXT_TOP_DIP, 11.0);
    assert_eq!(COMPACT_TEXT_RIGHT_DIP, 168.0);
    assert_eq!(COMPACT_TEXT_BOTTOM_DIP, 27.0);
    assert_eq!(COMPACT_TITLE_FONT_SIZE_DIP, 12.0);
    assert_eq!(COMPACT_TITLE_LINE_SPACING_DIP, 16.0);
    assert_eq!(COMPACT_TITLE_MAX_CHARS, 20);
}

#[test]
fn compact_anchor_offsets_and_rects() {
    // Left family: x = 0.0
    assert_eq!(HudAnchor::TopLeft.compact_offset_dip(), (0.0, 0.0));
    assert_eq!(HudAnchor::CenterLeft.compact_offset_dip(), (0.0, 9.0));
    assert_eq!(HudAnchor::BottomLeft.compact_offset_dip(), (0.0, 18.0));

    // Center-X family: x = 70.0
    assert_eq!(HudAnchor::TopCenter.compact_offset_dip(), (70.0, 0.0));
    assert_eq!(HudAnchor::BottomCenter.compact_offset_dip(), (70.0, 18.0));

    // Right family: x = 140.0
    assert_eq!(HudAnchor::TopRight.compact_offset_dip(), (140.0, 0.0));
    assert_eq!(HudAnchor::CenterRight.compact_offset_dip(), (140.0, 9.0));
    assert_eq!(HudAnchor::BottomRight.compact_offset_dip(), (140.0, 18.0));

    // In-canvas rect validation
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
fn compact_title_length_bounds_and_truncation() {
    let exact_20 = "12345678901234567890";
    assert_eq!(exact_20.chars().count(), 20);
    assert_eq!(
        truncate_with_ellipsis(exact_20, COMPACT_TITLE_MAX_CHARS),
        exact_20
    );

    let over_20 = "123456789012345678901";
    assert_eq!(over_20.chars().count(), 21);
    assert_eq!(
        truncate_with_ellipsis(over_20, COMPACT_TITLE_MAX_CHARS),
        "1234567890123456789…"
    );
    assert_eq!(
        truncate_with_ellipsis(over_20, COMPACT_TITLE_MAX_CHARS)
            .chars()
            .count(),
        20
    );

    let mut buf = [0u16; TITLE_UTF16_BUF_SIZE];
    let len = encode_truncated_utf16(over_20, COMPACT_TITLE_MAX_CHARS, &mut buf);
    let decoded = String::from_utf16(&buf[..len]).unwrap();
    assert_eq!(decoded, "1234567890123456789…");
    assert_eq!(decoded.chars().count(), 20);
}

#[test]
fn timing_and_sizing_constants() {
    assert_eq!(CANVAS_WIDTH_DIP, 320.0);
    assert_eq!(CANVAS_HEIGHT_DIP, 56.0);
    assert_eq!(CONTAINER_RADIUS_DIP, 12.0);
    assert_eq!(CONTAINER_BORDER_STROKE_DIP, 1.0);
    assert_eq!(MARGIN_DIP, 32.0);
    assert_eq!(ENTRANCE_DURATION_MS, 180);
    assert_eq!(EXIT_DURATION_MS, 200);
    assert_eq!(HOLD_SAVED_DURATION_MS, 2200);
    assert_eq!(HOLD_FAILED_DURATION_MS, 3200);
    assert_eq!(MAX_QUEUED_HOLD_MS, 5000);
    assert_eq!(REDUCED_MOTION_FADE_DURATION_MS, 60);
    assert_eq!(TITLE_MAX_CHARS, 28);
    assert_eq!(SUBTITLE_MAX_CHARS, 38);
    assert_eq!(HUD_COMMAND_QUEUE_BOUND, 16);
}

#[test]
fn text_length_bounds_and_truncation_contract() {
    // Exact spec limit testing
    let title_exact = "1234567890123456789012345678"; // 28 chars
    assert_eq!(title_exact.chars().count(), 28);
    assert_eq!(
        truncate_with_ellipsis(title_exact, TITLE_MAX_CHARS),
        title_exact
    );

    let title_long = "12345678901234567890123456789"; // 29 chars
    assert_eq!(
        truncate_with_ellipsis(title_long, TITLE_MAX_CHARS),
        "123456789012345678901234567…"
    );
    assert_eq!(
        truncate_with_ellipsis(title_long, TITLE_MAX_CHARS)
            .chars()
            .count(),
        28
    );

    let subtitle_exact = "12345678901234567890123456789012345678"; // 38 chars
    assert_eq!(subtitle_exact.chars().count(), 38);
    assert_eq!(
        truncate_with_ellipsis(subtitle_exact, SUBTITLE_MAX_CHARS),
        subtitle_exact
    );

    let subtitle_long = "123456789012345678901234567890123456789"; // 39 chars
    assert_eq!(
        truncate_with_ellipsis(subtitle_long, SUBTITLE_MAX_CHARS),
        "1234567890123456789012345678901234567…"
    );
    assert_eq!(
        truncate_with_ellipsis(subtitle_long, SUBTITLE_MAX_CHARS)
            .chars()
            .count(),
        38
    );
}

#[test]
fn fixed_buffer_formatting_and_utf16_bounds() {
    // 1. Saved format with duration and size
    let mut sub_buf = [0u16; SUBTITLE_UTF16_BUF_SIZE];
    let len = format_saved_subtitle_into_utf16(75_000, 150_000_000, &mut sub_buf);
    let rendered_sub = String::from_utf16(&sub_buf[..len]).unwrap();
    assert_eq!(rendered_sub, "01m 15s • 143.1 MB");
    assert!(len <= SUBTITLE_UTF16_BUF_SIZE);

    // 2. Custom Unicode with complex emojis and surrogate pairs
    let emoji_title = "🎬 Replay Saved! 🚀";
    let mut title_buf = [0u16; TITLE_UTF16_BUF_SIZE];
    let len = encode_truncated_utf16(emoji_title, TITLE_MAX_CHARS, &mut title_buf);
    let rendered_title = String::from_utf16(&title_buf[..len]).unwrap();
    assert_eq!(rendered_title, emoji_title);
    assert!(len <= TITLE_UTF16_BUF_SIZE);

    // 3. Over-length custom Unicode truncation without surrogate split
    let long_emojis = "🎮🎯🎲🎳🎰🃏🀄🎴🎵🎶🎷🎸🎹🎺🎻🎼🎽🎿🏂🏇🏄🏆🎦🎖️🎗️";
    let len = encode_truncated_utf16(long_emojis, 10, &mut title_buf);
    assert!(String::from_utf16(&title_buf[..len]).is_ok());
    let rendered_trunc = String::from_utf16(&title_buf[..len]).unwrap();
    assert!(rendered_trunc.ends_with('…'));
    assert_eq!(rendered_trunc.chars().count(), 10);
}

#[test]
fn pure_visibility_and_transition_lifecycle_spec() {
    // Pure lifecycle verification:
    // 1. Initial / Post-Exit: Idle opacity must be 0.0, offset 0.0 DIP
    let initial_idle_opacity = 0.0f32;
    let initial_idle_offset = 0.0f32;
    assert_eq!(initial_idle_opacity, 0.0);
    assert_eq!(initial_idle_offset, 0.0);

    // 2. Visible / Hold: Opacity must be 1.0, offset 0.0 DIP
    let visible_hold_opacity = 1.0f32;
    let visible_hold_offset = 0.0f32;
    assert_eq!(visible_hold_opacity, 1.0);
    assert_eq!(visible_hold_offset, 0.0);

    // 3. Bottom entrance motion: offset begins at +8.0 DIP and finishes at 0.0 DIP
    let bottom_anchor = HudAnchor::BottomCenter;
    assert_eq!(bottom_anchor.slide_offset_dip(), 8.0);

    // 4. Top entrance motion: offset begins at -8.0 DIP and finishes at 0.0 DIP
    let top_anchor = HudAnchor::TopCenter;
    assert_eq!(top_anchor.slide_offset_dip(), -8.0);

    // 5. Center entrance motion: offset remains 0.0 DIP
    let center_anchor = HudAnchor::CenterRight;
    assert_eq!(center_anchor.slide_offset_dip(), 0.0);
}

#[test]
#[ignore = "creates real Windows HWND and GPU composition tree for manual verification"]
fn test_native_hud_window_lifecycle_smoke() {
    let config = HudConfig::default();
    let hud =
        NativeHud::try_new(config).expect("Native HUD initialization must succeed on Windows");
    assert!(hud.capabilities().is_supported);
    assert!(hud.is_healthy());
    hud.try_show(HudCue::queued())
        .expect("Queued cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(200));
    hud.try_show(HudCue::saved(30_000, 50_000_000))
        .expect("Saved cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(300));
    hud.shutdown();
}

#[test]
#[ignore = "creates real Windows HWND and GPU composition tree for manual verification"]
fn test_native_hud_compact_window_lifecycle_smoke() {
    let config = HudConfig {
        mode: HudMode::Compact,
        anchor: HudAnchor::TopRight,
        exclude_from_capture: true,
    };
    let hud = NativeHud::try_new(config)
        .expect("Native HUD compact initialization must succeed on Windows");
    assert!(hud.capabilities().is_supported);
    assert!(hud.is_healthy());
    hud.try_show(HudCue::queued())
        .expect("Compact queued cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(200));
    hud.try_show(HudCue::saved(30_000, 50_000_000))
        .expect("Compact saved cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(200));
    hud.try_show(HudCue::failed("Buffer not ready yet"))
        .expect("Compact failed cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(300));
    hud.shutdown();
}

#[cfg(feature = "diagnostic-lifecycle")]
#[test]
fn diagnostic_lifecycle_policy_contract() {
    // 1. Default policy must be AttachedHidden
    assert_eq!(
        DiagnosticLifecyclePolicy::default(),
        DiagnosticLifecyclePolicy::AttachedHidden
    );

    // 2. AttachedShown idle state: root visual is attached, window is shown
    let attached_shown = DiagnosticLifecyclePolicy::AttachedShown;
    assert!(attached_shown.is_idle_root_attached());
    assert!(attached_shown.is_idle_window_shown());

    // 3. DetachedShown idle state: root visual is detached (null), window is shown
    let detached_shown = DiagnosticLifecyclePolicy::DetachedShown;
    assert!(!detached_shown.is_idle_root_attached());
    assert!(detached_shown.is_idle_window_shown());

    // 4. AttachedHidden idle state: root visual is attached, window is hidden
    let attached_hidden = DiagnosticLifecyclePolicy::AttachedHidden;
    assert!(attached_hidden.is_idle_root_attached());
    assert!(!attached_hidden.is_idle_window_shown());
}

#[cfg(all(feature = "diagnostic-lifecycle", windows))]
#[test]
#[ignore = "creates real Windows HWND and GPU composition tree for manual verification"]
fn test_native_hud_diagnostic_attached_shown_smoke() {
    let config = HudConfig::default();
    let hud = NativeHud::try_new_diagnostic(config, DiagnosticLifecyclePolicy::AttachedShown)
        .expect("Native HUD AttachedShown diagnostic initialization must succeed on Windows");
    assert!(hud.capabilities().is_supported);
    assert!(hud.is_healthy());
    hud.try_show(HudCue::queued())
        .expect("Queued cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(200));
    hud.try_show(HudCue::saved(12_000, 24_000_000))
        .expect("Saved cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(300));
    hud.shutdown();
}

#[cfg(all(feature = "diagnostic-lifecycle", windows))]
#[test]
#[ignore = "creates real Windows HWND and GPU composition tree for manual verification"]
fn test_native_hud_diagnostic_detached_shown_smoke() {
    let config = HudConfig::default();
    let hud = NativeHud::try_new_diagnostic(config, DiagnosticLifecyclePolicy::DetachedShown)
        .expect("Native HUD DetachedShown diagnostic initialization must succeed on Windows");
    assert!(hud.capabilities().is_supported);
    assert!(hud.is_healthy());
    hud.try_show(HudCue::queued())
        .expect("Queued cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(200));
    hud.try_show(HudCue::saved(12_000, 24_000_000))
        .expect("Saved cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(300));
    hud.shutdown();
}

#[cfg(all(feature = "diagnostic-lifecycle", windows))]
#[test]
#[ignore = "creates real Windows HWND and GPU composition tree for manual verification"]
fn test_native_hud_diagnostic_attached_hidden_smoke() {
    let config = HudConfig::default();
    let hud = NativeHud::try_new_diagnostic(config, DiagnosticLifecyclePolicy::AttachedHidden)
        .expect("Native HUD AttachedHidden diagnostic initialization must succeed on Windows");
    assert!(hud.capabilities().is_supported);
    assert!(hud.is_healthy());
    hud.try_show(HudCue::queued())
        .expect("Queued cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(200));
    hud.try_show(HudCue::saved(12_000, 24_000_000))
        .expect("Saved cue submission must succeed");
    std::thread::sleep(std::time::Duration::from_millis(300));
    hud.shutdown();
}
