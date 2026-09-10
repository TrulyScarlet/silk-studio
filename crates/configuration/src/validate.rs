use crate::chords::normalize_chord;
use crate::error::FieldIssue;
use crate::model::{AppConfig, MAX_AUDIO_TRACKS};
use std::collections::BTreeSet;

const MIN_REPLAY_SECONDS: u32 = 15;
const MAX_REPLAY_SECONDS: u32 = 300;
const MIN_VIDEO_BITRATE_KBPS: u32 = 2_500;
const MAX_VIDEO_BITRATE_KBPS: u32 = 100_000;

/// Validate every setting; returns all violations (empty when valid).
pub fn validate_settings(config: &AppConfig) -> Vec<FieldIssue> {
    let mut issues = Vec::new();

    // Capture (VID-004/005)
    if !matches!(config.capture.frame_rate, 30 | 60 | 120) {
        issues.push(FieldIssue {
            field: "capture.frameRate".to_string(),
            reason: format!("must be 30, 60, or 120, got {}", config.capture.frame_rate),
        });
    }
    if config.capture.source_type.is_empty() {
        issues.push(FieldIssue {
            field: "capture.sourceType".to_string(),
            reason: "must not be empty".to_string(),
        });
    } else {
        let source_type = config.capture.source_type.to_ascii_lowercase();
        if !matches!(source_type.as_str(), "display" | "game" | "window") {
            issues.push(FieldIssue {
                field: "capture.sourceType".to_string(),
                reason: format!(
                    "only 'display', 'game', or 'window' are supported, got '{}'",
                    config.capture.source_type
                ),
            });
        } else if source_type != "game" && config.capture.source_id.trim().is_empty() {
            issues.push(FieldIssue {
                field: "capture.sourceId".to_string(),
                reason: "must not be empty".to_string(),
            });
        }
    }

    // Replay (BUF-002/003): MVP range must include 15–120 seconds.
    let dur = config.replay.duration_seconds;
    if !(MIN_REPLAY_SECONDS..=MAX_REPLAY_SECONDS).contains(&dur) {
        issues.push(FieldIssue {
            field: "replay.durationSeconds".to_string(),
            reason: format!(
                "must be within {MIN_REPLAY_SECONDS}..={MAX_REPLAY_SECONDS}, got {dur}"
            ),
        });
    }

    // Encoding (ENC-005/006)
    if let Some(vbr) = config.encoding.video_bitrate_kbps {
        if !(MIN_VIDEO_BITRATE_KBPS..=MAX_VIDEO_BITRATE_KBPS).contains(&vbr) {
            issues.push(FieldIssue {
                field: "encoding.videoBitrateKbps".to_string(),
                reason: format!(
                    "must be within {MIN_VIDEO_BITRATE_KBPS}..={MAX_VIDEO_BITRATE_KBPS} kbps, got {vbr}"
                ),
            });
        }
    }
    let kf = config.encoding.keyframe_interval_seconds;
    if !(1.0..=10.0).contains(&kf) {
        issues.push(FieldIssue {
            field: "encoding.keyframeIntervalSeconds".to_string(),
            reason: format!("must be within 1.0..=10.0, got {kf}"),
        });
    }
    let video_codec_lower = config.encoding.video_codec.to_ascii_lowercase();
    if !matches!(
        video_codec_lower.as_str(),
        "h264" | "avc" | "avc1" | "hevc" | "h265" | "hvc1" | "av1" | "av01"
    ) {
        issues.push(FieldIssue {
            field: "encoding.videoCodec".to_string(),
            reason: format!(
                "only H.264 ('h264', 'avc', 'avc1'), HEVC ('hevc', 'h265', 'hvc1'), or AV1 ('av1', 'av01') are supported for direct MP4, got '{}'",
                config.encoding.video_codec
            ),
        });
    }
    if config.encoding.audio_codec != "aac" {
        issues.push(FieldIssue {
            field: "encoding.audioCodec".to_string(),
            reason: format!(
                "only 'aac' is supported in the MVP, got '{}'",
                config.encoding.audio_codec
            ),
        });
    }
    let abr = config.encoding.audio_bitrate_kbps;
    if !(64..=320).contains(&abr) {
        issues.push(FieldIssue {
            field: "encoding.audioBitrateKbps".to_string(),
            reason: format!("must be within 64..=320 kbps, got {abr}"),
        });
    }

    // Audio (AUD-003/004): preserve the configured order and validate each
    // track independently. `None` is the only v2 representation of default.
    if config.audio.tracks.len() > MAX_AUDIO_TRACKS {
        issues.push(FieldIssue {
            field: "audio.tracks".to_string(),
            reason: format!(
                "must contain at most {MAX_AUDIO_TRACKS} tracks, got {}",
                config.audio.tracks.len()
            ),
        });
    }
    let mut enabled_tracks = 0_usize;
    let mut track_ids = BTreeSet::new();
    for (index, track) in config.audio.tracks.iter().enumerate() {
        let field = |name: &str| format!("audio.tracks[{index}].{name}");

        if track.id.trim().is_empty() {
            issues.push(FieldIssue {
                field: field("id"),
                reason: "must not be empty".to_string(),
            });
        } else if !track_ids.insert(&track.id) {
            issues.push(FieldIssue {
                field: field("id"),
                reason: "must be unique among audio tracks".to_string(),
            });
        }
        if track.name.trim().is_empty() {
            issues.push(FieldIssue {
                field: field("name"),
                reason: "must not be empty".to_string(),
            });
        }
        if let Some(device_id) = track.device_id.as_deref() {
            if device_id.trim().is_empty() {
                issues.push(FieldIssue {
                    field: field("deviceId"),
                    reason: "must be a non-empty device ID or null for the default device"
                        .to_string(),
                });
            } else if device_id == "default" {
                issues.push(FieldIssue {
                    field: field("deviceId"),
                    reason: "must use null for the default device".to_string(),
                });
            }
        }
        if !track.gain.is_finite() || !(0.0..=8.0).contains(&track.gain) {
            issues.push(FieldIssue {
                field: field("gain"),
                reason: "must be finite and between 0 and 8".to_string(),
            });
        }
        if track.enabled {
            enabled_tracks += 1;
        }
    }
    if enabled_tracks > MAX_AUDIO_TRACKS {
        issues.push(FieldIssue {
            field: "audio.tracks".to_string(),
            reason: format!(
                "must have at most {MAX_AUDIO_TRACKS} enabled tracks, got {enabled_tracks}"
            ),
        });
    }

    // Output (MUX-008/009/010). The direct MP4 muxer publishes MP4 only.
    validate_windows_directory(&config.output.directory, "output.directory", &mut issues);
    validate_file_name_pattern(
        &config.output.file_name_pattern,
        "output.fileNamePattern",
        &mut issues,
    );
    if config.output.container != crate::ContainerFormat::Mp4 {
        issues.push(FieldIssue {
            field: "output.container".to_string(),
            reason: "only 'mp4' is supported for direct recordings".to_string(),
        });
    }

    // Hotkeys (KEY-001)
    validate_chord(
        Some(&config.hotkeys.save_replay),
        "hotkeys.saveReplay",
        &mut issues,
    );
    validate_chord(
        config.hotkeys.start_capture.as_deref(),
        "hotkeys.startCapture",
        &mut issues,
    );
    validate_chord(
        config.hotkeys.stop_capture.as_deref(),
        "hotkeys.stopCapture",
        &mut issues,
    );
    validate_duplicate_chords(config, &mut issues);

    // Storage (STO-004)
    if config.storage.quota_gigabytes == 0 {
        issues.push(FieldIssue {
            field: "storage.quotaGigabytes".to_string(),
            reason: "must be at least 1 GiB".to_string(),
        });
    }
    if config.storage.automatic_deletion_enabled && !config.storage.quota_enabled {
        issues.push(FieldIssue {
            field: "storage.automaticDeletionEnabled".to_string(),
            reason: "requires storage quota to be enabled".to_string(),
        });
    }

    issues
}

fn validate_chord(raw: Option<&str>, field: &str, issues: &mut Vec<FieldIssue>) {
    match raw {
        None | Some("") => {
            if raw == Some("") {
                issues.push(FieldIssue {
                    field: field.to_string(),
                    reason: "must not be empty".to_string(),
                });
            }
        }
        Some(chord) => {
            if let Err(reason) = normalize_chord(chord) {
                issues.push(FieldIssue {
                    field: field.to_string(),
                    reason: reason.to_user_message(),
                });
            }
        }
    }
}

fn validate_duplicate_chords(config: &AppConfig, issues: &mut Vec<FieldIssue>) {
    let mut seen = BTreeSet::new();
    for (field, raw) in [
        (
            "hotkeys.saveReplay",
            Some(config.hotkeys.save_replay.as_str()),
        ),
        (
            "hotkeys.startCapture",
            config.hotkeys.start_capture.as_deref(),
        ),
        (
            "hotkeys.stopCapture",
            config.hotkeys.stop_capture.as_deref(),
        ),
    ] {
        let Some(raw) = raw else {
            continue;
        };
        let Ok(normalized) = normalize_chord(raw) else {
            continue;
        };
        if !seen.insert(normalized) {
            issues.push(FieldIssue {
                field: field.to_string(),
                reason: "duplicates another configured hotkey".to_string(),
            });
        }
    }
}

const INVALID_WINDOWS_CHARS: [char; 9] = ['<', '>', ':', '"', '|', '?', '*', '/', '\\'];

fn validate_windows_directory(value: &str, field: &str, issues: &mut Vec<FieldIssue>) {
    if value.trim().is_empty() {
        issues.push(FieldIssue {
            field: field.to_string(),
            reason: "must not be empty".to_string(),
        });
        return;
    }

    let body = value
        .strip_prefix(char::is_alphabetic)
        .and_then(|rest| rest.strip_prefix(':'))
        .unwrap_or(value);
    for component in body.split(['\\', '/']).filter(|c| !c.is_empty()) {
        if component
            .chars()
            .any(|c| INVALID_WINDOWS_CHARS.contains(&c))
        {
            issues.push(FieldIssue {
                field: field.to_string(),
                reason: format!("directory component '{component}' contains characters that are invalid on Windows"),
            });
            return;
        }
        if component.ends_with('.') || component.ends_with(' ') {
            issues.push(FieldIssue {
                field: field.to_string(),
                reason: format!(
                    "directory component '{component}' must not end with a dot or space"
                ),
            });
            return;
        }
    }
}

fn validate_file_name_pattern(pattern: &str, field: &str, issues: &mut Vec<FieldIssue>) {
    if pattern.trim().is_empty() {
        issues.push(FieldIssue {
            field: field.to_string(),
            reason: "must not be empty".to_string(),
        });
        return;
    }
    const PATTERN_INVALID: [char; 9] = ['<', '>', ':', '"', '|', '?', '*', '/', '\\'];
    if pattern.chars().any(|c| PATTERN_INVALID.contains(&c)) {
        issues.push(FieldIssue {
            field: field.to_string(),
            reason: "pattern must not contain Windows-invalid characters".to_string(),
        });
        return;
    }
    let known_placeholders = ["{date}", "{time}", "{source}", "{index}"];
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            issues.push(FieldIssue {
                field: field.to_string(),
                reason: "unbalanced '{' in pattern".to_string(),
            });
            return;
        };
        let token = &rest[start..=start + end];
        if !known_placeholders.contains(&token) {
            issues.push(FieldIssue {
                field: field.to_string(),
                reason: format!(
                    "unknown placeholder '{token}' (known: {})",
                    known_placeholders.join(", ")
                ),
            });
            return;
        }
        rest = &rest[start + end + 1..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AppConfig, DEFAULT_FILE_NAME_PATTERN};

    fn base() -> AppConfig {
        AppConfig::default()
    }

    #[test]
    fn defaults_are_valid() {
        assert!(DEFAULT_FILE_NAME_PATTERN.starts_with("Clip_"));
        assert!(validate_settings(&base()).is_empty());
    }

    #[test]
    fn frame_rate_must_be_30_60_or_120() {
        let mut c = base();
        for valid_fps in [30, 60, 120] {
            c.capture.frame_rate = valid_fps;
            assert!(validate_settings(&c).is_empty());
        }
        c.capture.frame_rate = 45;
        let issues = validate_settings(&c);
        assert!(issues.iter().any(|i| i.field == "capture.frameRate"));
    }

    #[test]
    fn video_codec_supports_supported_aliases() {
        let mut c = base();
        for codec in [
            "avc", "AVC", "h264", "H264", "avc1", "AVC1", "hevc", "HEVC", "h265", "H265", "hvc1",
            "HVC1", "av1", "AV1", "av01", "AV01",
        ] {
            c.encoding.video_codec = codec.to_string();
            assert!(
                validate_settings(&c).is_empty(),
                "expected codec '{codec}' to be valid"
            );
        }
        for unsupported in [
            "vp9", "prores", "VP9", "PRORES", "vvc", "h266", "mpeg2", "mp4v",
        ] {
            c.encoding.video_codec = unsupported.to_string();
            let issues = validate_settings(&c);
            assert!(
                issues.iter().any(|i| i.field == "encoding.videoCodec"),
                "expected codec '{unsupported}' to be rejected"
            );
        }
    }

    #[test]
    fn output_resolutions_are_valid() {
        use crate::OutputResolution;
        assert_eq!(OutputResolution::Native.fixed_size(), None);
        assert_eq!(
            OutputResolution::Res3840x2160.fixed_size(),
            Some((3840, 2160))
        );
        assert_eq!(
            OutputResolution::Res2560x1440.fixed_size(),
            Some((2560, 1440))
        );
        assert_eq!(
            OutputResolution::Res1920x1080.fixed_size(),
            Some((1920, 1080))
        );
        assert_eq!(
            OutputResolution::Res1280x720.fixed_size(),
            Some((1280, 720))
        );
    }

    #[test]
    fn replay_bounds_match_spec_range() {
        let mut c = base();
        c.replay.duration_seconds = 301;
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "replay.durationSeconds"));
        c.replay.duration_seconds = MIN_REPLAY_SECONDS;
        assert!(validate_settings(&c).is_empty());
        c.replay.duration_seconds = MAX_REPLAY_SECONDS;
        assert!(validate_settings(&c).is_empty());
    }

    #[test]
    fn video_bitrate_bounds_are_validated() {
        let mut c = base();
        c.encoding.video_bitrate_kbps = None;
        assert!(validate_settings(&c).is_empty());

        c.encoding.video_bitrate_kbps = Some(2_500);
        assert!(validate_settings(&c).is_empty());

        c.encoding.video_bitrate_kbps = Some(100_000);
        assert!(validate_settings(&c).is_empty());

        c.encoding.video_bitrate_kbps = Some(8_000);
        assert!(validate_settings(&c).is_empty());

        c.encoding.video_bitrate_kbps = Some(2_499);
        let issues = validate_settings(&c);
        assert!(issues
            .iter()
            .any(|i| i.field == "encoding.videoBitrateKbps"));

        c.encoding.video_bitrate_kbps = Some(100_001);
        let issues = validate_settings(&c);
        assert!(issues
            .iter()
            .any(|i| i.field == "encoding.videoBitrateKbps"));
    }

    #[test]
    fn keyframe_interval_bounded() {
        let mut c = base();
        c.encoding.keyframe_interval_seconds = 0.5;
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "encoding.keyframeIntervalSeconds"));
        c.encoding.keyframe_interval_seconds = 11.0;
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "encoding.keyframeIntervalSeconds"));
    }

    #[test]
    fn directory_rejects_invalid_characters() {
        let mut c = base();
        c.output.directory = r"C:\bad<path".to_string();
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "output.directory"));

        c.output.directory = r"C:\ok\trailing dot.".to_string();
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "output.directory"));

        c.output.directory = r"C:\Users\User\Videos".to_string();
        assert!(validate_settings(&c).is_empty());
    }

    #[test]
    fn directory_accepts_fully_qualified_windows_roots_and_non_windows_paths() {
        let mut c = base();
        for directory in [r"C:\", r"E:\", r"C:\Users\User\Videos"] {
            c.output.directory = directory.to_string();
            assert!(
                validate_settings(&c).is_empty(),
                "expected directory to be valid: {directory}"
            );
        }

        c.output.directory = "/tmp/silk-output".to_string();
        assert!(validate_settings(&c).is_empty());
    }

    #[test]
    fn pattern_placeholders_validated() {
        let mut c = base();
        c.output.file_name_pattern = "Clip_{bogus}".to_string();
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "output.fileNamePattern"));

        c.output.file_name_pattern = "Clip_{date}".to_string();
        assert!(validate_settings(&c).is_empty());

        c.output.file_name_pattern = "Clip_{date".to_string();
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "output.fileNamePattern"));

        c.output.file_name_pattern = r"Clip_{source}\nested".to_string();
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "output.fileNamePattern"));
    }

    #[test]
    fn hotkey_chords_validated() {
        let mut c = base();
        c.hotkeys.save_replay = "Ctrl+F10".to_string();
        assert!(validate_settings(&c).is_empty());

        c.hotkeys.save_replay = "Ctrl+Ctrl+F10".to_string();
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "hotkeys.saveReplay"));
    }

    #[test]
    fn duplicate_hotkey_chords_are_rejected() {
        let mut c = base();
        c.hotkeys.start_capture = Some("shift+ctrl+f10".to_string());
        let issues = validate_settings(&c);
        assert!(issues.iter().any(|issue| {
            issue.field == "hotkeys.startCapture" && issue.reason.contains("duplicates another")
        }));
    }

    #[test]
    fn capture_source_and_audio_tracks_are_validated() {
        let mut c = base();
        c.capture.source_type = "invalid_type".to_string();
        c.capture.source_id.clear();
        c.audio.tracks[0].device_id = Some(String::new());
        let issues = validate_settings(&c);
        assert!(issues.iter().any(|i| i.field == "capture.sourceType"));
        assert!(issues.iter().any(|i| i.field == "audio.tracks[0].deviceId"));

        // display requires non-empty source_id
        c.capture.source_type = "display".to_string();
        c.capture.source_id = "display-1".to_string();
        c.audio.tracks[0].device_id = None;
        assert!(validate_settings(&c).is_empty());
        c.capture.source_id.clear();
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "capture.sourceId"));

        // window requires non-empty source_id
        c.capture.source_type = "window".to_string();
        c.capture.source_id = "hwnd:1234".to_string();
        assert!(validate_settings(&c).is_empty());
        c.capture.source_id.clear();
        assert!(validate_settings(&c)
            .iter()
            .any(|i| i.field == "capture.sourceId"));

        // game allows empty or "auto" source_id
        c.capture.source_type = "game".to_string();
        c.capture.source_id.clear();
        assert!(validate_settings(&c).is_empty());
        c.capture.source_id = "auto".to_string();
        assert!(validate_settings(&c).is_empty());
        c.capture.source_type = "GAME".to_string();
        assert!(validate_settings(&c).is_empty());
    }

    #[test]
    fn track_ids_and_indexed_fields_are_validated() {
        let mut c = base();
        c.audio.tracks[1].id = c.audio.tracks[0].id.clone();
        c.audio.tracks[1].name.clear();
        c.audio.tracks[1].gain = 9.0;
        let issues = validate_settings(&c);

        assert!(issues
            .iter()
            .any(|issue| issue.field == "audio.tracks[1].id"));
        assert!(issues
            .iter()
            .any(|issue| issue.field == "audio.tracks[1].name"));
        assert!(issues
            .iter()
            .any(|issue| issue.field == "audio.tracks[1].gain"));
    }

    #[test]
    fn default_device_sentinel_is_not_valid_in_v2() {
        let mut c = base();
        c.audio.tracks[0].device_id = Some("default".to_string());
        let issues = validate_settings(&c);
        assert!(issues
            .iter()
            .any(|issue| issue.field == "audio.tracks[0].deviceId"));
    }

    #[test]
    fn audio_track_count_is_bounded() {
        let mut c = base();
        c.audio.tracks.extend(
            (0..MAX_AUDIO_TRACKS).map(|index| crate::AudioTrackSettings {
                id: format!("extra-{index}"),
                name: format!("Extra {index}"),
                ..crate::AudioTrackSettings::default()
            }),
        );
        let issues = validate_settings(&c);
        assert!(issues.iter().any(|issue| issue.field == "audio.tracks"));
    }

    #[test]
    fn output_containers_are_validated() {
        let mut c = base();
        c.output.container = crate::ContainerFormat::Mp4;
        assert!(validate_settings(&c).is_empty());
    }

    #[test]
    fn automatic_deletion_requires_an_enabled_quota() {
        let mut c = base();
        c.storage.automatic_deletion_enabled = true;
        let issues = validate_settings(&c);
        assert!(issues
            .iter()
            .any(|i| i.field == "storage.automaticDeletionEnabled"));

        c.storage.quota_enabled = true;
        assert!(validate_settings(&c).is_empty());
    }

    #[test]
    fn overlay_settings_default_values() {
        let overlay = crate::OverlaySettings::default();
        assert!(!overlay.enabled);
        assert_eq!(overlay.mode, crate::OverlayMode::Full);
        assert_eq!(overlay.position, crate::OverlayPosition::BottomCenter);
        assert_eq!(base().overlay, overlay);
    }

    #[test]
    fn overlay_settings_round_trip_serialization() {
        let overlay = crate::OverlaySettings {
            enabled: false,
            mode: crate::OverlayMode::Compact,
            position: crate::OverlayPosition::TopRight,
        };

        let json = serde_json::to_string(&overlay).expect("overlay serializes");
        assert_eq!(
            json,
            r#"{"enabled":false,"mode":"compact","position":"top_right"}"#
        );

        let deserialized: crate::OverlaySettings =
            serde_json::from_str(&json).expect("overlay deserializes");
        assert_eq!(deserialized, overlay);
    }

    #[test]
    fn overlay_modes_serialization_and_deserialization() {
        let modes = [
            (crate::OverlayMode::Full, "\"full\""),
            (crate::OverlayMode::Compact, "\"compact\""),
        ];

        for (mode, expected_json) in modes {
            let serialized = serde_json::to_string(&mode).expect("mode serializes");
            assert_eq!(serialized, expected_json);

            let deserialized: crate::OverlayMode =
                serde_json::from_str(expected_json).expect("mode deserializes");
            assert_eq!(deserialized, mode);
        }
    }

    #[test]
    fn overlay_all_eight_positions_serialization_and_deserialization() {
        let positions = [
            (crate::OverlayPosition::TopLeft, "\"top_left\""),
            (crate::OverlayPosition::TopCenter, "\"top_center\""),
            (crate::OverlayPosition::TopRight, "\"top_right\""),
            (crate::OverlayPosition::CenterLeft, "\"center_left\""),
            (crate::OverlayPosition::CenterRight, "\"center_right\""),
            (crate::OverlayPosition::BottomLeft, "\"bottom_left\""),
            (crate::OverlayPosition::BottomCenter, "\"bottom_center\""),
            (crate::OverlayPosition::BottomRight, "\"bottom_right\""),
        ];

        for (position, expected_json) in positions {
            let serialized = serde_json::to_string(&position).expect("position serializes");
            assert_eq!(serialized, expected_json);

            let deserialized: crate::OverlayPosition =
                serde_json::from_str(expected_json).expect("position deserializes");
            assert_eq!(deserialized, position);
        }
    }

    #[test]
    fn overlay_settings_rejects_unknown_fields() {
        let invalid_json = r#"{
            "enabled": true,
            "mode": "full",
            "position": "bottom_center",
            "extraField": "unexpected"
        }"#;

        let result: Result<crate::OverlaySettings, _> = serde_json::from_str(invalid_json);
        assert!(result.is_err());
    }

    #[test]
    fn application_settings_defaults() {
        let app = crate::ApplicationSettings::default();
        assert!(!app.start_with_windows);
        assert!(app.minimize_to_tray);
        assert!(app.notifications_enabled);
        assert!(app.clip_sound_enabled);
        assert_eq!(app.theme, crate::model::Theme::Studio);
        assert_eq!(base().application, app);
    }

    #[test]
    fn application_settings_round_trip_serialization() {
        let app = crate::ApplicationSettings {
            start_with_windows: true,
            minimize_to_tray: false,
            notifications_enabled: false,
            clip_sound_enabled: false,
            theme: crate::model::Theme::Ember,
        };

        let json = serde_json::to_string(&app).expect("application settings serialize");
        assert_eq!(
            json,
            r#"{"startWithWindows":true,"minimizeToTray":false,"notificationsEnabled":false,"clipSoundEnabled":false,"theme":"ember"}"#
        );

        let deserialized: crate::ApplicationSettings =
            serde_json::from_str(&json).expect("application settings deserialize");
        assert_eq!(deserialized, app);
    }

    #[test]
    fn application_settings_rejects_unknown_fields() {
        let invalid_json = r#"{
            "startWithWindows": false,
            "minimizeToTray": true,
            "notificationsEnabled": true,
            "clipSoundEnabled": true,
            "theme": "classic",
            "unknownProperty": 42
        }"#;

        let result: Result<crate::ApplicationSettings, _> = serde_json::from_str(invalid_json);
        assert!(result.is_err());
    }
}
