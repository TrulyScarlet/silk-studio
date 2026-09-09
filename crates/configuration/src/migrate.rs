use serde::Deserialize;
use serde_json::Value;

use crate::error::ConfigError;
use crate::model::{AppConfig, AudioSettings, AudioSourceKind, AudioTrackSettings, ConfigEnvelope};

pub const CURRENT_CONFIG_VERSION: u32 = 6;

/// Normalize a parsed configuration document to the current version.
///
/// Returns the normalized settings subtree, the version found in the file
/// before migration, and — when a migration ran — that same original version.
/// Version 0 means "no version marker present".
pub fn migrate_document(mut document: Value) -> Result<(Value, u32, Option<u32>), ConfigError> {
    let original = read_version(&document)?;
    let mut current = original;

    while current < CURRENT_CONFIG_VERSION {
        document = match current {
            // Version 0 is the pre-versioning form of the original schema.
            // Its additive defaults are retained before the semantic v1 step.
            0 => apply_unversioned_defaults(document),
            1 => migrate_v1_to_v2(document)?,
            2 => migrate_v2_to_v3(document)?,
            3 => migrate_v3_to_v4(document)?,
            4 => migrate_v4_to_v5(document)?,
            5 => migrate_v5_to_v6(document)?,
            _ => unreachable!("unsupported migration source version {current}"),
        };
        current += 1;
    }

    set_version(&mut document);
    let envelope: ConfigEnvelope =
        serde_json::from_value(document).map_err(|err| ConfigError::InvalidJson {
            path: std::path::PathBuf::from("<in-memory>"),
            message: err.to_string(),
        })?;
    let settings = serde_json::to_value(envelope.settings)
        .map_err(|err| ConfigError::Serialization(err.to_string()))?;
    let migrated_from = if original == CURRENT_CONFIG_VERSION {
        None
    } else {
        Some(original)
    };
    Ok((settings, original, migrated_from))
}

fn read_version(document: &Value) -> Result<u32, ConfigError> {
    match document.get("version") {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Number(n)) => match n.as_u64() {
            Some(v) if v <= CURRENT_CONFIG_VERSION as u64 => Ok(v as u32),
            Some(v) => Err(ConfigError::UnsupportedVersion {
                found: v.min(u64::from(u32::MAX)) as u32,
                supported: CURRENT_CONFIG_VERSION,
            }),
            None => Err(ConfigError::UnsupportedVersion {
                found: 0,
                supported: CURRENT_CONFIG_VERSION,
            }),
        },
        Some(_) => Err(ConfigError::InvalidJson {
            path: std::path::PathBuf::from("<in-memory>"),
            message: "'version' must be an unsigned integer".to_string(),
        }),
    }
}

/// Fill missing keys in an unversioned document with the original v1
/// defaults. This is deliberately limited to version 0; v1 to v2 changes the
/// audio shape and is handled by the explicit migration below.
fn apply_unversioned_defaults(mut document: Value) -> Value {
    let mut defaults = serde_json::to_value(crate::model::ConfigEnvelope {
        version: 1,
        settings: AppConfig::default(),
    })
    .expect("default config serializes");

    if let Some(settings) = defaults.get_mut("settings").and_then(Value::as_object_mut) {
        settings.insert("audio".to_string(), legacy_audio_defaults());
    }

    if let (Some(base), Some(target)) = (defaults.get_mut("settings"), document.get_mut("settings"))
    {
        merge_defaults(base, target);
    }
    document
}

/// Convert the v1 desktop and microphone fields into the ordered v2 track
/// list. No defaults are merged here: the old values are mapped explicitly so
/// the migration cannot silently change the meaning of an existing setting.
fn migrate_v1_to_v2(mut document: Value) -> Result<Value, ConfigError> {
    let Some(settings) = document.get_mut("settings").and_then(Value::as_object_mut) else {
        return Ok(document);
    };

    let audio = settings
        .remove("audio")
        .unwrap_or_else(legacy_audio_defaults);
    let legacy: LegacyAudioSettings =
        serde_json::from_value(audio).map_err(|err| ConfigError::InvalidJson {
            path: std::path::PathBuf::from("<in-memory>"),
            message: err.to_string(),
        })?;
    // v2 represents independent tracks directly; the old mixed/separate flag
    // had no effect on the existing engine boundary.
    let _ = legacy.separate_tracks;

    let audio = AudioSettings {
        tracks: vec![
            AudioTrackSettings {
                id: "desktop".to_string(),
                name: "Desktop".to_string(),
                enabled: legacy.desktop_enabled,
                source_kind: AudioSourceKind::OutputLoopback,
                device_id: normalize_legacy_device_id(legacy.desktop_device_id),
                gain: crate::model::DEFAULT_AUDIO_TRACK_GAIN,
            },
            AudioTrackSettings {
                id: "microphone".to_string(),
                name: "Microphone".to_string(),
                enabled: legacy.microphone_enabled,
                source_kind: AudioSourceKind::Input,
                device_id: normalize_legacy_device_id(legacy.microphone_device_id),
                gain: crate::model::DEFAULT_AUDIO_TRACK_GAIN,
            },
        ],
    };
    let audio =
        serde_json::to_value(audio).map_err(|err| ConfigError::Serialization(err.to_string()))?;
    settings.insert("audio".to_string(), audio);
    Ok(document)
}

/// Migrate v2 settings to v3:
/// Normalizes video codec to "h264", output container to "mp4",
/// remuxToMp4 to false, and postSaveExport to false, while preserving
/// all other settings (bitrate, resolution, fps, directory, audio tracks, etc.).
fn migrate_v2_to_v3(mut document: Value) -> Result<Value, ConfigError> {
    let Some(settings) = document.get_mut("settings").and_then(Value::as_object_mut) else {
        return Ok(document);
    };

    if let Some(encoding) = settings.get_mut("encoding").and_then(Value::as_object_mut) {
        encoding.insert("videoCodec".to_string(), Value::from("h264"));
    }

    if let Some(output) = settings.get_mut("output").and_then(Value::as_object_mut) {
        output.insert("container".to_string(), Value::from("mp4"));
        output.insert("remuxToMp4".to_string(), Value::from(false));
        output.insert("postSaveExport".to_string(), Value::from(false));
    }

    Ok(document)
}

/// Migrate v3 settings to v4:
/// Normalizes output container to "mp4" and removes obsolete keys
/// `remuxToMp4`, `postSaveExport`, and `postSaveExportPreset`.
fn migrate_v3_to_v4(mut document: Value) -> Result<Value, ConfigError> {
    let Some(settings) = document.get_mut("settings").and_then(Value::as_object_mut) else {
        return Ok(document);
    };

    if let Some(output) = settings.get_mut("output").and_then(Value::as_object_mut) {
        output.insert("container".to_string(), Value::from("mp4"));
        output.remove("remuxToMp4");
        output.remove("postSaveExport");
        output.remove("postSaveExportPreset");
    }

    Ok(document)
}

/// Migrate v4 settings to v5:
/// Inserts `fidelityMode: "standard"` under `encoding` if not present,
/// preserving all existing fields.
fn migrate_v4_to_v5(mut document: Value) -> Result<Value, ConfigError> {
    let Some(settings) = document.get_mut("settings").and_then(Value::as_object_mut) else {
        return Ok(document);
    };

    if let Some(encoding) = settings.get_mut("encoding").and_then(Value::as_object_mut) {
        if !encoding.contains_key("fidelityMode") {
            encoding.insert("fidelityMode".to_string(), Value::from("standard"));
        }
    }

    Ok(document)
}

/// Migrate v5 settings to v6:
/// Inserts `clipSoundEnabled: true` and `theme: "classic"` under `application`
/// if not present, preserving all existing fields.
fn migrate_v5_to_v6(mut document: Value) -> Result<Value, ConfigError> {
    let Some(settings) = document.get_mut("settings").and_then(Value::as_object_mut) else {
        return Ok(document);
    };

    if let Some(application) = settings
        .get_mut("application")
        .and_then(Value::as_object_mut)
    {
        if !application.contains_key("clipSoundEnabled") {
            application.insert("clipSoundEnabled".to_string(), Value::from(true));
        }
        if !application.contains_key("theme") {
            application.insert("theme".to_string(), Value::from("classic"));
        }
    }

    Ok(document)
}

fn normalize_legacy_device_id(device_id: Option<String>) -> Option<String> {
    device_id.filter(|id| id != "default")
}

fn legacy_audio_defaults() -> Value {
    serde_json::json!({
        "desktopEnabled": true,
        "desktopDeviceId": "default",
        "microphoneEnabled": false,
        "microphoneDeviceId": null,
        "separateTracks": true,
    })
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct LegacyAudioSettings {
    desktop_enabled: bool,
    desktop_device_id: Option<String>,
    microphone_enabled: bool,
    microphone_device_id: Option<String>,
    separate_tracks: bool,
}

impl Default for LegacyAudioSettings {
    fn default() -> Self {
        Self {
            desktop_enabled: true,
            desktop_device_id: Some("default".to_string()),
            microphone_enabled: false,
            microphone_device_id: None,
            separate_tracks: true,
        }
    }
}

fn set_version(document: &mut Value) {
    if let Some(map) = document.as_object_mut() {
        map.insert("version".to_string(), Value::from(CURRENT_CONFIG_VERSION));
    }
}

fn merge_defaults(base: &mut Value, target: &mut Value) {
    if let (Value::Object(base_map), Value::Object(target_map)) = (base, target) {
        for (key, base_child) in base_map.iter_mut() {
            match target_map.get_mut(key) {
                Some(target_child) => merge_defaults(base_child, target_child),
                None => {
                    target_map.insert(key.clone(), base_child.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioSourceKind;

    fn migrated(document: Value) -> (AppConfig, u32, Option<u32>) {
        let (settings, file_version, migrated_from) =
            migrate_document(document).expect("migration succeeds");
        (
            serde_json::from_value(settings).expect("settings deserialize"),
            file_version,
            migrated_from,
        )
    }

    #[test]
    fn v1_audio_is_migrated_to_ordered_tracks() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 1,
            "settings": {
                "audio": {
                    "desktopEnabled": false,
                    "desktopDeviceId": "default",
                    "microphoneEnabled": true,
                    "microphoneDeviceId": "input-42",
                    "separateTracks": false
                }
            }
        }));

        assert_eq!(file_version, 1);
        assert_eq!(migrated_from, Some(1));
        assert_eq!(settings.audio.tracks.len(), 2);
        assert_eq!(settings.audio.tracks[0].id, "desktop");
        assert_eq!(settings.audio.tracks[0].name, "Desktop");
        assert!(!settings.audio.tracks[0].enabled);
        assert_eq!(
            settings.audio.tracks[0].source_kind,
            AudioSourceKind::OutputLoopback
        );
        assert_eq!(settings.audio.tracks[0].device_id, None);
        assert_eq!(settings.audio.tracks[1].id, "microphone");
        assert!(settings.audio.tracks[1].enabled);
        assert_eq!(settings.audio.tracks[1].source_kind, AudioSourceKind::Input);
        assert_eq!(
            settings.audio.tracks[1].device_id.as_deref(),
            Some("input-42")
        );
    }

    #[test]
    fn unversioned_audio_uses_v1_defaults_then_migrates() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "settings": {}
        }));

        assert_eq!(file_version, 0);
        assert_eq!(migrated_from, Some(0));
        assert_eq!(settings.audio, AudioSettings::default());
    }

    #[test]
    fn v2_audio_is_not_replaced_by_migration_defaults() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 2,
            "settings": {
                "audio": {
                    "tracks": [{
                        "id": "game",
                        "name": "Game",
                        "enabled": true,
                        "sourceKind": "output_loopback",
                        "deviceId": "endpoint-7",
                        "gain": 0.5
                    }]
                }
            }
        }));

        assert_eq!(file_version, 2);
        assert_eq!(migrated_from, Some(2));
        assert_eq!(settings.audio.tracks.len(), 1);
        assert_eq!(settings.audio.tracks[0].id, "game");
        assert_eq!(settings.audio.tracks[0].gain, 0.5);
    }

    #[test]
    fn v2_mkv_hevc_migrates_to_v4_mp4_h264_preserving_bitrate_and_other_fields() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 2,
            "settings": {
                "encoding": {
                    "videoCodec": "hevc",
                    "encoder": "auto",
                    "qualityPreset": "high",
                    "videoBitrateKbps": 45000,
                    "keyframeIntervalSeconds": 2.0,
                    "audioCodec": "aac",
                    "audioBitrateKbps": 192
                },
                "output": {
                    "directory": "D:/clips",
                    "container": "mkv",
                    "remuxToMp4": true,
                    "fileNamePattern": "Clip_{index}",
                    "postSaveExport": true,
                    "postSaveExportPreset": "large"
                }
            }
        }));

        assert_eq!(file_version, 2);
        assert_eq!(migrated_from, Some(2));
        assert_eq!(settings.encoding.video_codec, "h264");
        assert_eq!(settings.encoding.video_bitrate_kbps, Some(45000));
        assert_eq!(settings.output.container, crate::ContainerFormat::Mp4);
        assert_eq!(settings.output.directory, "D:/clips");
        assert_eq!(settings.output.file_name_pattern, "Clip_{index}");
    }

    #[test]
    fn v3_migrates_to_v4_removing_obsolete_keys() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 3,
            "settings": {
                "encoding": {
                    "videoCodec": "h264",
                    "encoder": "hardware",
                    "qualityPreset": "medium",
                    "videoBitrateKbps": 80000,
                    "keyframeIntervalSeconds": 1.5,
                    "audioCodec": "aac",
                    "audioBitrateKbps": 256
                },
                "output": {
                    "directory": "C:/my-clips",
                    "container": "mp4",
                    "remuxToMp4": true,
                    "fileNamePattern": "Clip_{date}_{time}",
                    "postSaveExport": true,
                    "postSaveExportPreset": "medium"
                }
            }
        }));

        assert_eq!(file_version, 3);
        assert_eq!(migrated_from, Some(3));
        assert_eq!(settings.encoding.video_codec, "h264");
        assert_eq!(settings.encoding.video_bitrate_kbps, Some(80000));
        assert_eq!(settings.output.container, crate::ContainerFormat::Mp4);
        assert_eq!(settings.output.directory, "C:/my-clips");
    }

    #[test]
    fn v4_config_migrates_to_v5_preserving_fields_and_inserting_standard_fidelity() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 4,
            "settings": {
                "encoding": {
                    "videoCodec": "h264",
                    "encoder": "hardware",
                    "qualityPreset": "medium",
                    "videoBitrateKbps": 80000,
                    "keyframeIntervalSeconds": 1.5,
                    "audioCodec": "aac",
                    "audioBitrateKbps": 256
                },
                "output": {
                    "directory": "C:/my-clips",
                    "container": "mp4",
                    "fileNamePattern": "Clip_{date}_{time}"
                }
            }
        }));

        assert_eq!(file_version, 4);
        assert_eq!(migrated_from, Some(4));
        assert_eq!(settings.encoding.video_codec, "h264");
        assert_eq!(settings.encoding.video_bitrate_kbps, Some(80000));
        assert_eq!(
            settings.encoding.fidelity_mode,
            crate::VideoFidelityMode::Standard
        );
        assert_eq!(settings.output.container, crate::ContainerFormat::Mp4);
    }

    #[test]
    fn v4_config_with_explicit_fidelity_mode_is_preserved() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 4,
            "settings": {
                "encoding": {
                    "videoCodec": "h264",
                    "encoder": "hardware",
                    "qualityPreset": "medium",
                    "videoBitrateKbps": 80000,
                    "keyframeIntervalSeconds": 1.5,
                    "audioCodec": "aac",
                    "audioBitrateKbps": 256,
                    "fidelityMode": "clarity"
                },
                "output": {
                    "directory": "C:/my-clips",
                    "container": "mp4",
                    "fileNamePattern": "Clip_{date}_{time}"
                }
            }
        }));

        assert_eq!(file_version, 4);
        assert_eq!(migrated_from, Some(4));
        assert_eq!(
            settings.encoding.fidelity_mode,
            crate::VideoFidelityMode::Clarity
        );
    }

    #[test]
    fn v5_config_migrates_to_v6_preserving_fields_and_inserting_application_defaults() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 5,
            "settings": {
                "encoding": {
                    "videoCodec": "h264",
                    "encoder": "hardware",
                    "qualityPreset": "medium",
                    "videoBitrateKbps": 80000,
                    "keyframeIntervalSeconds": 1.5,
                    "audioCodec": "aac",
                    "audioBitrateKbps": 256,
                    "fidelityMode": "archival_444"
                },
                "output": {
                    "directory": "C:/my-clips",
                    "container": "mp4",
                    "fileNamePattern": "Clip_{date}_{time}"
                },
                "application": {
                    "startWithWindows": false,
                    "minimizeToTray": true,
                    "notificationsEnabled": true
                }
            }
        }));

        assert_eq!(file_version, 5);
        assert_eq!(migrated_from, Some(5));
        assert_eq!(settings.encoding.video_codec, "h264");
        assert_eq!(settings.encoding.video_bitrate_kbps, Some(80000));
        assert_eq!(
            settings.encoding.fidelity_mode,
            crate::VideoFidelityMode::Archival444
        );
        assert_eq!(settings.output.container, crate::ContainerFormat::Mp4);
        assert!(settings.application.clip_sound_enabled);
        assert_eq!(settings.application.theme, crate::model::Theme::Classic);
    }

    #[test]
    fn v5_config_with_explicit_clip_sound_and_theme_is_preserved() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 5,
            "settings": {
                "application": {
                    "startWithWindows": true,
                    "minimizeToTray": false,
                    "notificationsEnabled": false,
                    "clipSoundEnabled": false,
                    "theme": "ember"
                }
            }
        }));

        assert_eq!(file_version, 5);
        assert_eq!(migrated_from, Some(5));
        assert!(settings.application.start_with_windows);
        assert!(!settings.application.minimize_to_tray);
        assert!(!settings.application.notifications_enabled);
        assert!(!settings.application.clip_sound_enabled);
        assert_eq!(settings.application.theme, crate::model::Theme::Ember);
    }

    #[test]
    fn v6_config_is_not_modified() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 6,
            "settings": {
                "encoding": {
                    "videoCodec": "h264",
                    "encoder": "hardware",
                    "qualityPreset": "medium",
                    "videoBitrateKbps": 80000,
                    "keyframeIntervalSeconds": 1.5,
                    "audioCodec": "aac",
                    "audioBitrateKbps": 256,
                    "fidelityMode": "archival_444"
                },
                "output": {
                    "directory": "C:/my-clips",
                    "container": "mp4",
                    "fileNamePattern": "Clip_{date}_{time}"
                },
                "application": {
                    "startWithWindows": false,
                    "minimizeToTray": true,
                    "notificationsEnabled": true,
                    "clipSoundEnabled": false,
                    "theme": "ember"
                }
            }
        }));

        assert_eq!(file_version, 6);
        assert_eq!(migrated_from, None);
        assert_eq!(settings.encoding.video_codec, "h264");
        assert_eq!(settings.encoding.video_bitrate_kbps, Some(80000));
        assert_eq!(
            settings.encoding.fidelity_mode,
            crate::VideoFidelityMode::Archival444
        );
        assert_eq!(settings.output.container, crate::ContainerFormat::Mp4);
        assert!(!settings.application.clip_sound_enabled);
        assert_eq!(settings.application.theme, crate::model::Theme::Ember);
    }

    #[test]
    fn theme_serde_and_invalid_values() {
        let themes = [
            (crate::model::Theme::Classic, "\"classic\""),
            (crate::model::Theme::Ember, "\"ember\""),
            (crate::model::Theme::Vamp, "\"vamp\""),
        ];

        for (theme, expected_json) in themes {
            let serialized = serde_json::to_string(&theme).expect("theme serializes");
            assert_eq!(serialized, expected_json);

            let deserialized: crate::model::Theme =
                serde_json::from_str(expected_json).expect("theme deserializes");
            assert_eq!(deserialized, theme);
        }

        for invalid in ["\"dark\"", "\"light\"", "\"custom\"", "123", "null"] {
            let result: Result<crate::model::Theme, _> = serde_json::from_str(invalid);
            assert!(result.is_err(), "expected '{invalid}' to be rejected");
        }
    }

    #[test]
    fn fidelity_mode_serde_and_invalid_values() {
        let modes = [
            (crate::VideoFidelityMode::Standard, "\"standard\""),
            (crate::VideoFidelityMode::Clarity, "\"clarity\""),
            (crate::VideoFidelityMode::Archival444, "\"archival_444\""),
        ];

        for (mode, expected_json) in modes {
            let serialized = serde_json::to_string(&mode).expect("mode serializes");
            assert_eq!(serialized, expected_json);

            let deserialized: crate::VideoFidelityMode =
                serde_json::from_str(expected_json).expect("mode deserializes");
            assert_eq!(deserialized, mode);
        }

        for invalid in [
            "\"archival\"",
            "\"archival444\"",
            "\"ultra\"",
            "\"unknown\"",
            "123",
        ] {
            let result: Result<crate::VideoFidelityMode, _> = serde_json::from_str(invalid);
            assert!(result.is_err(), "expected '{invalid}' to be rejected");
        }
    }

    #[test]
    fn v2_config_without_video_bitrate_defaults_to_none() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 2,
            "settings": {
                "encoding": {
                    "videoCodec": "h264",
                    "encoder": "auto",
                    "qualityPreset": "high",
                    "keyframeIntervalSeconds": 2.0,
                    "audioCodec": "aac",
                    "audioBitrateKbps": 192
                }
            }
        }));

        assert_eq!(file_version, 2);
        assert_eq!(migrated_from, Some(2));
        assert_eq!(settings.encoding.video_bitrate_kbps, None);
    }

    #[test]
    fn v2_config_with_explicit_video_bitrate_is_preserved() {
        let (settings, file_version, migrated_from) = migrated(serde_json::json!({
            "version": 2,
            "settings": {
                "encoding": {
                    "videoCodec": "h264",
                    "encoder": "auto",
                    "qualityPreset": "high",
                    "videoBitrateKbps": 15000,
                    "keyframeIntervalSeconds": 2.0,
                    "audioCodec": "aac",
                    "audioBitrateKbps": 192
                }
            }
        }));

        assert_eq!(file_version, 2);
        assert_eq!(migrated_from, Some(2));
        assert_eq!(settings.encoding.video_bitrate_kbps, Some(15000));
    }

    #[test]
    fn v1_unknown_audio_fields_are_rejected_instead_of_merged() {
        let result = migrate_document(serde_json::json!({
            "version": 1,
            "settings": {
                "audio": {
                    "desktopEnabled": true,
                    "desktopDeviceId": "default",
                    "microphoneEnabled": false,
                    "microphoneDeviceId": null,
                    "separateTracks": true,
                    "tracks": []
                }
            }
        }));

        assert!(matches!(result, Err(ConfigError::InvalidJson { .. })));
    }
}
