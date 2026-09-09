//! Versioned JSON configuration for the Silk recorder.
//!
//! Mirrors the configuration model in spec §18. Loading is tolerant about
//! file absence (defaults are returned) but strict about content:
//! unknown keys and out-of-range values are rejected with structured
//! errors (see ADR 0001).

mod chords;
mod error;
mod migrate;
mod model;
mod validate;

pub use chords::{normalize_chord, ChordError};
pub use error::{ConfigError, FieldIssue};
pub use migrate::{migrate_document, CURRENT_CONFIG_VERSION};
pub use model::{
    AppConfig, AppTheme, ApplicationSettings, AudioSettings, AudioSourceKind, AudioTrackSettings,
    ContainerFormat, DiagnosticLogLevel, DiagnosticsSettings, EncoderSelection, EncodingSettings,
    HotkeysSettings, OutputResolution, OutputSettings, OverlayMode, OverlayPosition,
    OverlaySettings, QualityPreset, ReplaySettings, StorageSettings, Theme, VideoFidelityMode,
    DEFAULT_AUDIO_TRACK_GAIN, DEFAULT_FILE_NAME_PATTERN, DEFAULT_SAVE_HOTKEY, MAX_AUDIO_TRACKS,
};
pub use validate::validate_settings;

use std::path::Path;

/// Result of a successful load.
#[derive(Debug)]
pub struct LoadedConfig {
    pub settings: AppConfig,
    /// Version found in the file before migration; 0 when the file had no
    /// version marker.
    pub file_version: u32,
    pub migrated_from: Option<u32>,
    /// Non-fatal notes such as "file was missing, defaults created".
    pub warnings: Vec<String>,
}

/// Load configuration from `path`. Missing files produce defaults.
///
/// Handles the S0 exit-criteria cases: missing file, corrupt JSON,
/// unknown keys, older versions (migrated), newer versions (rejected).
pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
    if !path.exists() {
        return Ok(LoadedConfig {
            settings: AppConfig::default(),
            file_version: CURRENT_CONFIG_VERSION,
            migrated_from: None,
            warnings: vec![format!(
                "configuration file {} not found; defaults created",
                path.display()
            )],
        });
    }

    let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let document: serde_json::Value =
        serde_json::from_str(&raw).map_err(|err| ConfigError::InvalidJson {
            path: path.to_path_buf(),
            message: err.to_string(),
        })?;

    let (settings_value, file_version, migrated_from) = migrate::migrate_document(document)?;

    let settings: model::AppConfig =
        serde_json::from_value(settings_value).map_err(|err| ConfigError::InvalidJson {
            path: path.to_path_buf(),
            message: err.to_string(),
        })?;

    let issues = validate_settings(&settings);
    if !issues.is_empty() {
        return Err(ConfigError::Validation(issues));
    }

    Ok(LoadedConfig {
        settings,
        file_version,
        migrated_from,
        warnings: Vec::new(),
    })
}

/// Save configuration atomically: write a sibling temp file, then rename
/// over the destination.
pub fn save(path: &Path, settings: &AppConfig) -> Result<(), ConfigError> {
    let issues = validate_settings(settings);
    if !issues.is_empty() {
        return Err(ConfigError::Validation(issues));
    }

    let envelope = model::ConfigEnvelope {
        version: CURRENT_CONFIG_VERSION,
        settings: settings.clone(),
    };

    let rendered = serde_json::to_string_pretty(&envelope)
        .map_err(|err| ConfigError::Serialization(err.to_string()))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let temp = sibling_temp_path(path);
    std::fs::write(&temp, rendered.as_bytes()).map_err(|source| ConfigError::Io {
        path: temp.clone(),
        source,
    })?;

    std::fs::rename(&temp, path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn sibling_temp_path(path: &Path) -> std::path::PathBuf {
    match (path.file_name(), path.parent()) {
        (Some(name), Some(parent)) => {
            let mut temp = name.to_os_string();
            temp.push(".tmp");
            parent.join(temp)
        }
        _ => path.with_extension("json.tmp"),
    }
}
