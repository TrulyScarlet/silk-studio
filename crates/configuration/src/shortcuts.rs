use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
pub(crate) struct ShortcutDefinition {
    pub id: String,
    pub default: Option<String>,
}

// The UI imports this same catalog, so defaults and action IDs cannot drift.
pub(crate) fn definitions() -> &'static [ShortcutDefinition] {
    static DEFINITIONS: OnceLock<Vec<ShortcutDefinition>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        serde_json::from_str(include_str!("shortcuts.json"))
            .expect("valid built-in shortcut catalog")
    })
}

#[cfg(test)]
mod tests {
    use crate::{validate_settings, AppConfig, HotkeysSettings};

    #[test]
    fn older_hotkeys_load_and_overrides_round_trip() {
        let old: HotkeysSettings = serde_json::from_str(
            r#"{"saveReplay":"Ctrl+Shift+F10","startCapture":null,"stopCapture":null}"#,
        )
        .unwrap();
        assert!(old.shortcuts.is_empty());
        let mut config = AppConfig::default();
        config
            .hotkeys
            .shortcuts
            .insert("fullscreen".into(), Some("Ctrl+F".into()));
        config
            .hotkeys
            .shortcuts
            .insert("playPauseAlternate".into(), None);
        assert!(validate_settings(&config).is_empty());
        let encoded = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<AppConfig>(&encoded).unwrap(), config);
    }

    #[test]
    fn validates_effective_defaults_conflicts_and_unknown_actions() {
        let mut config = AppConfig::default();
        assert!(validate_settings(&config).is_empty());
        config
            .hotkeys
            .shortcuts
            .insert("fullscreen".into(), Some("SPACE".into()));
        assert!(validate_settings(&config)
            .iter()
            .any(|issue| issue.reason.contains("duplicates")));
        config.hotkeys.shortcuts.insert("playPause".into(), None);
        assert!(validate_settings(&config).is_empty());
        config
            .hotkeys
            .shortcuts
            .insert("fullscreen".into(), Some("shift+ctrl+F10".into()));
        assert!(validate_settings(&config)
            .iter()
            .any(|issue| issue.field == "hotkeys.shortcuts.fullscreen"
                && issue.reason.contains("duplicates")));
        config
            .hotkeys
            .shortcuts
            .insert("fullscreen".into(), Some("Hyper+F".into()));
        assert!(validate_settings(&config)
            .iter()
            .any(|issue| issue.field == "hotkeys.shortcuts.fullscreen"));
        config
            .hotkeys
            .shortcuts
            .insert("unknownAction".into(), None);
        assert!(validate_settings(&config)
            .iter()
            .any(|issue| issue.reason == "unknown shortcut action"));
    }

    #[test]
    fn existing_global_hotkeys_take_precedence_over_new_inherited_defaults() {
        let mut config = AppConfig::default();
        config.hotkeys.save_replay = "F".into();
        assert!(validate_settings(&config).is_empty());
        config
            .hotkeys
            .shortcuts
            .insert("fullscreen".into(), Some("F".into()));
        assert!(validate_settings(&config)
            .iter()
            .any(|issue| issue.field == "hotkeys.shortcuts.fullscreen"));
    }
}
