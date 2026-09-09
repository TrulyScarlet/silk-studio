use serde::{Deserialize, Serialize};

pub const DEFAULT_FILE_NAME_PATTERN: &str = "Clip_{date}_{time}_{source}";
pub const DEFAULT_SAVE_HOTKEY: &str = "Ctrl+Shift+F10";
pub const DEFAULT_AUDIO_TRACK_GAIN: f32 = 1.0;

/// Hard safety limit for configured audio tracks. Disabled tracks count toward
/// the total, and the same limit applies to enabled tracks.
pub const MAX_AUDIO_TRACKS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ConfigEnvelope {
    pub version: u32,
    #[serde(default)]
    pub settings: AppConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct AppConfig {
    pub capture: CaptureSettings,
    pub audio: AudioSettings,
    pub encoding: EncodingSettings,
    pub replay: ReplaySettings,
    pub output: OutputSettings,
    pub hotkeys: HotkeysSettings,
    pub storage: StorageSettings,
    pub application: ApplicationSettings,
    pub diagnostics: DiagnosticsSettings,
    pub overlay: OverlaySettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CaptureSettings {
    pub source_type: String,
    pub source_id: String,
    pub frame_rate: u32,
    pub output_resolution: OutputResolution,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            source_type: "display".to_string(),
            source_id: "display-1".to_string(),
            frame_rate: 60,
            output_resolution: OutputResolution::Native,
        }
    }
}

/// `"native"`, `"3840x2160"`, `"2560x1440"`, `"1920x1080"` or `"1280x720"` in the JSON representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputResolution {
    #[serde(rename = "native")]
    Native,
    #[serde(rename = "3840x2160")]
    Res3840x2160,
    #[serde(rename = "2560x1440")]
    Res2560x1440,
    #[serde(rename = "1920x1080")]
    Res1920x1080,
    #[serde(rename = "1280x720")]
    Res1280x720,
}

impl OutputResolution {
    pub fn fixed_size(self) -> Option<(u32, u32)> {
        match self {
            Self::Native => None,
            Self::Res3840x2160 => Some((3840, 2160)),
            Self::Res2560x1440 => Some((2560, 1440)),
            Self::Res1920x1080 => Some((1920, 1080)),
            Self::Res1280x720 => Some((1280, 720)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct AudioSettings {
    /// Ordered independently captured and muxed sources.
    pub tracks: Vec<AudioTrackSettings>,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            tracks: vec![
                AudioTrackSettings {
                    id: "desktop".to_string(),
                    name: "Desktop".to_string(),
                    enabled: true,
                    source_kind: AudioSourceKind::OutputLoopback,
                    device_id: None,
                    gain: DEFAULT_AUDIO_TRACK_GAIN,
                },
                AudioTrackSettings {
                    id: "microphone".to_string(),
                    name: "Microphone".to_string(),
                    enabled: false,
                    source_kind: AudioSourceKind::Input,
                    device_id: None,
                    gain: DEFAULT_AUDIO_TRACK_GAIN,
                },
            ],
        }
    }
}

/// One ordered audio source. The `id` is stable across renames and ordering
/// changes so saved preferences can continue to refer to the same source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AudioTrackSettings {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub source_kind: AudioSourceKind,
    /// `None` selects the system default device for this source kind.
    #[serde(default)]
    pub device_id: Option<String>,
    /// Linear gain multiplier applied by the audio processing layer.
    #[serde(default = "default_audio_track_gain")]
    pub gain: f32,
}

impl Default for AudioTrackSettings {
    fn default() -> Self {
        Self {
            id: "track".to_string(),
            name: "Audio".to_string(),
            enabled: true,
            source_kind: AudioSourceKind::OutputLoopback,
            device_id: None,
            gain: DEFAULT_AUDIO_TRACK_GAIN,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSourceKind {
    OutputLoopback,
    Input,
}

fn default_audio_track_gain() -> f32 {
    DEFAULT_AUDIO_TRACK_GAIN
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EncodingSettings {
    pub video_codec: String,
    pub encoder: EncoderSelection,
    pub quality_preset: QualityPreset,
    #[serde(default)]
    pub video_bitrate_kbps: Option<u32>,
    pub keyframe_interval_seconds: f64,
    pub audio_codec: String,
    pub audio_bitrate_kbps: u32,
    #[serde(default)]
    pub fidelity_mode: VideoFidelityMode,
}

impl Default for EncodingSettings {
    fn default() -> Self {
        Self {
            video_codec: "h264".to_string(),
            encoder: EncoderSelection::Auto,
            quality_preset: QualityPreset::High,
            video_bitrate_kbps: None,
            keyframe_interval_seconds: 2.0,
            audio_codec: "aac".to_string(),
            audio_bitrate_kbps: 192,
            fidelity_mode: VideoFidelityMode::Standard,
        }
    }
}

/// Recording fidelity intent. Defaults to `Standard`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoFidelityMode {
    #[default]
    Standard,
    Clarity,
    #[serde(rename = "archival_444")]
    Archival444,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderSelection {
    Auto,
    Hardware,
    Software,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityPreset {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReplaySettings {
    pub duration_seconds: u32,
}

impl Default for ReplaySettings {
    fn default() -> Self {
        Self {
            duration_seconds: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OutputSettings {
    pub directory: String,
    pub container: ContainerFormat,
    pub file_name_pattern: String,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            directory: default_output_directory().unwrap_or_else(|| "clips".to_string()),
            container: ContainerFormat::Mp4,
            file_name_pattern: DEFAULT_FILE_NAME_PATTERN.to_string(),
        }
    }
}

fn default_output_directory() -> Option<String> {
    let home = std::env::var_os("USERPROFILE")?;
    Some(
        std::path::PathBuf::from(home)
            .join("Videos")
            .to_string_lossy()
            .into_owned(),
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerFormat {
    #[default]
    Mp4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HotkeysSettings {
    pub save_replay: String,
    pub start_capture: Option<String>,
    pub stop_capture: Option<String>,
}

impl Default for HotkeysSettings {
    fn default() -> Self {
        Self {
            save_replay: DEFAULT_SAVE_HOTKEY.to_string(),
            start_capture: None,
            stop_capture: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StorageSettings {
    pub quota_enabled: bool,
    pub quota_gigabytes: u64,
    pub automatic_deletion_enabled: bool,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            quota_enabled: false,
            quota_gigabytes: 100,
            automatic_deletion_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DiagnosticsSettings {
    pub logging_level: DiagnosticLogLevel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Classic,
    Ember,
    Vamp,
}

#[allow(dead_code)]
pub type AppTheme = Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ApplicationSettings {
    pub start_with_windows: bool,
    pub minimize_to_tray: bool,
    pub notifications_enabled: bool,
    #[serde(default = "default_clip_sound_enabled")]
    pub clip_sound_enabled: bool,
    #[serde(default)]
    pub theme: Theme,
}

fn default_clip_sound_enabled() -> bool {
    true
}

impl Default for ApplicationSettings {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            minimize_to_tray: true,
            notifications_enabled: true,
            clip_sound_enabled: true,
            theme: Theme::Classic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OverlaySettings {
    pub enabled: bool,
    pub mode: OverlayMode,
    pub position: OverlayPosition,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMode {
    #[default]
    Full,
    Compact,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPosition {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    CenterRight,
    BottomLeft,
    #[default]
    BottomCenter,
    BottomRight,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: OverlayMode::Full,
            position: OverlayPosition::BottomCenter,
        }
    }
}
