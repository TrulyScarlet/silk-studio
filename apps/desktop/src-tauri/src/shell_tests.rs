//! Shell-logic tests: exercise the exact functions behind the Tauri
//! command handlers (`ping_message`, `execute_command`) with a collecting
//! sink, plus a lifecycle flow through the mock pipeline.

use std::time::{Duration, Instant};

use app_controller::{Command, ControllerEvent};
use configuration::{AppConfig, HotkeysSettings};
use hotkeys::{HotkeyAction, HotkeyEvent};
use test_support::TempDir;

use crate::shell;

#[derive(Default)]
struct CollectingSink(std::sync::Mutex<Vec<ControllerEvent>>);

impl shell::EventSink for CollectingSink {
    fn emit(&self, event: &ControllerEvent) {
        if let Ok(mut guard) = self.0.lock() {
            guard.push(event.clone());
        }
    }
}

impl CollectingSink {
    fn snapshot(&self) -> Vec<ControllerEvent> {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

#[test]
fn ping_round_trip_format() {
    assert_eq!(shell::ping_message("hello"), "pong:hello");
    assert_eq!(shell::ping_message(""), "pong:");
}

#[test]
fn close_defaults_to_tray_but_explicit_exit_is_allowed() {
    assert_eq!(shell::close_action(true, false), shell::CloseAction::Hide);
    assert_eq!(shell::close_action(true, true), shell::CloseAction::Exit);
    assert_eq!(shell::close_action(false, false), shell::CloseAction::Exit);
}

#[test]
fn configured_start_stop_bindings_use_stable_actions_and_ids() {
    let settings = HotkeysSettings {
        save_replay: "Ctrl+Shift+F10".to_string(),
        start_capture: Some("Ctrl+F9".to_string()),
        stop_capture: Some("Ctrl+F8".to_string()),
    };
    let bindings = shell::hotkey_bindings(&settings).expect("valid bindings");

    assert_eq!(bindings.len(), 3);
    assert_eq!(bindings[0].action, HotkeyAction::SaveReplay);
    assert_eq!(bindings[0].id, hotkeys::SAVE_REPLAY_HOTKEY_ID);
    assert_eq!(bindings[1].action, HotkeyAction::StartCapture);
    assert_eq!(bindings[1].id, hotkeys::START_CAPTURE_HOTKEY_ID);
    assert_eq!(bindings[2].action, HotkeyAction::StopCapture);
    assert_eq!(bindings[2].id, hotkeys::STOP_CAPTURE_HOTKEY_ID);
}

#[test]
fn invalid_configured_binding_is_rejected_before_registration() {
    let settings = HotkeysSettings {
        save_replay: "Ctrl+Hyper+F10".to_string(),
        ..HotkeysSettings::default()
    };

    assert!(shell::hotkey_bindings(&settings).is_err());
}

#[test]
fn duplicate_configured_chord_is_rejected_before_registration() {
    let settings = HotkeysSettings {
        save_replay: "Ctrl+Shift+F10".to_string(),
        start_capture: Some("shift+ctrl+f10".to_string()),
        stop_capture: None,
    };

    assert!(matches!(
        shell::hotkey_bindings(&settings),
        Err(hotkeys::HotkeyError::DuplicateChord { chord })
            if chord == "CTRL+SHIFT+F10"
    ));
}

#[test]
fn startup_loads_legacy_config_and_rewrites_current_version() {
    let dir = TempDir::new("shell-config-migration").expect("temp");
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{
  "version": 0,
  "settings": {
    "hotkeys": {
      "saveReplay": "ctrl+alt+f7",
      "startCapture": "ctrl+f9",
      "stopCapture": null
    }
  }
}"#,
    )
    .expect("write legacy config");

    let settings = shell::load_startup_config(&path);

    assert_eq!(settings.hotkeys.save_replay, "ctrl+alt+f7");
    let rewritten = configuration::load(&path).expect("read migrated config");
    assert_eq!(
        rewritten.file_version,
        configuration::CURRENT_CONFIG_VERSION
    );
    assert_eq!(rewritten.migrated_from, None);
    assert!(!path.with_file_name("config.json.tmp").exists());
}

#[test]
fn startup_uses_defaults_without_overwriting_invalid_config() {
    let dir = TempDir::new("shell-config-invalid").expect("temp");
    let path = dir.path().join("config.json");
    let invalid = r#"{"version":1,"settings":{"hotkeys":{"saveReplay":"Ctrl+Hyper+F10"}}}"#;
    std::fs::write(&path, invalid).expect("write invalid config");

    let settings = shell::load_startup_config(&path);

    assert_eq!(settings, AppConfig::default());
    assert_eq!(
        std::fs::read_to_string(&path).expect("read invalid config"),
        invalid
    );
}

#[test]
fn shell_state_exposes_loaded_hotkeys_without_media_data() {
    let dir = TempDir::new("shell-config-state").expect("temp");
    let mut settings = AppConfig::default();
    settings.hotkeys.save_replay = "CTRL+ALT+F7".to_string();
    let state = shell::ShellState::with_app_config(
        shell::build_controller(dir.path().to_path_buf()),
        None,
        settings,
        dir.path().join("config.json"),
        true,
    );

    assert_eq!(
        shell::hotkey_settings(&state)
            .expect("settings")
            .save_replay,
        "CTRL+ALT+F7"
    );
}

#[test]
fn shell_state_exposes_and_updates_overlay_settings() {
    let dir = TempDir::new("shell-overlay-settings").expect("temp");
    let mut initial_settings = AppConfig::default();
    initial_settings.overlay.enabled = true;
    initial_settings.overlay.mode = configuration::OverlayMode::Full;
    initial_settings.overlay.position = configuration::OverlayPosition::BottomCenter;

    let state = shell::ShellState::with_app_config(
        shell::build_controller(dir.path().to_path_buf()),
        None,
        initial_settings.clone(),
        dir.path().join("config.json"),
        true,
    );

    assert_eq!(
        shell::app_settings(&state).expect("settings").overlay,
        configuration::OverlaySettings {
            enabled: true,
            mode: configuration::OverlayMode::Full,
            position: configuration::OverlayPosition::BottomCenter,
        }
    );

    let mut next_settings = initial_settings;
    next_settings.overlay.enabled = false;
    next_settings.overlay.mode = configuration::OverlayMode::Compact;
    next_settings.overlay.position = configuration::OverlayPosition::TopRight;

    shell::apply_app_settings(&state, &next_settings).expect("apply settings");

    assert_eq!(
        shell::app_settings(&state).expect("settings").overlay,
        configuration::OverlaySettings {
            enabled: false,
            mode: configuration::OverlayMode::Compact,
            position: configuration::OverlayPosition::TopRight,
        }
    );
}

#[test]
fn shell_state_exposes_recorder_status_snapshot() {
    let dir = TempDir::new("shell-status").expect("temp");
    let state = shell::ShellState::with_app_config(
        shell::build_controller(dir.path().to_path_buf()),
        None,
        AppConfig::default(),
        dir.path().join("config.json"),
        true,
    );

    let status = shell::recorder_status(&state).expect("status");
    assert_eq!(status.state, "Stopped");
    assert_eq!(status.buffered_packet_count, 0);
    assert_eq!(status.buffer_duration_ms, 0);
    assert_eq!(status.audio_stream_count, 0);
    assert_eq!(
        status.fidelity,
        app_controller::FidelityStatus {
            requested: configuration::VideoFidelityMode::Standard,
            active: None,
            issue: None,
        }
    );
}

#[test]
fn invalid_full_settings_are_rejected_before_runtime_changes() {
    let dir = TempDir::new("shell-invalid-settings").expect("temp");
    let state = shell::ShellState::with_app_config(
        shell::build_controller(dir.path().to_path_buf()),
        None,
        AppConfig::default(),
        dir.path().join("config.json"),
        true,
    );
    let mut invalid = AppConfig::default();
    invalid.replay.duration_seconds = 1;

    let error = shell::apply_app_settings(&state, &invalid).expect_err("invalid settings");
    assert!(error.contains("replay.durationSeconds"));
    assert_eq!(
        shell::app_settings(&state).expect("settings"),
        AppConfig::default()
    );
}

#[test]
fn shell_library_scan_uses_the_configured_output_directory() {
    let dir = TempDir::new("shell-library").expect("temp");
    let output = dir.path().join("clips");
    let managed = output.join("Silk");
    std::fs::create_dir_all(&managed).expect("output");
    std::fs::write(managed.join("from-shell.mp4"), b"clip").expect("clip");
    let mut settings = AppConfig::default();
    settings.output.directory = output.to_string_lossy().into_owned();
    let state = shell::ShellState::with_app_config(
        shell::build_controller_from_config(&settings),
        None,
        settings,
        dir.path().join("config.json"),
        true,
    );

    let entries = shell::library_scan(&state).expect("scan");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "from-shell.mp4");
}

#[test]
fn managed_clip_directory_is_a_child_of_the_selected_base() {
    let base = std::path::Path::new(r"E:\captures");
    assert_eq!(
        shell::managed_clip_directory(base),
        std::path::PathBuf::from(r"E:\captures\Silk")
    );
}

#[test]
fn hotkey_activation_uses_controller_command_path() {
    let dir = TempDir::new("shell-hotkey").expect("temp");
    let mut controller = shell::build_controller(dir.path().to_path_buf());

    let events = shell::dispatch_hotkey_events(
        &mut controller,
        vec![HotkeyEvent::Activated(HotkeyAction::SaveReplay)],
        None,
    );

    assert!(events.iter().any(|event| {
        matches!(
            event,
            ControllerEvent::CommandRejected { command, .. } if command == "save_replay"
        )
    }));
}

#[test]
fn hotkey_event_dispatch_preserves_order_and_handles_warnings() {
    let dir = TempDir::new("shell-hotkey-order").expect("temp");
    let mut controller = shell::build_controller(dir.path().to_path_buf());

    let hotkey_events = vec![
        HotkeyEvent::RegistrationFailed {
            action: HotkeyAction::SaveReplay,
            chord: "Ctrl+Shift+F10".to_string(),
            os_error: 1400,
        },
        HotkeyEvent::EventQueueFull,
        HotkeyEvent::Activated(HotkeyAction::SaveReplay),
    ];

    let events = shell::dispatch_hotkey_events(&mut controller, hotkey_events, None);

    assert_eq!(events.len(), 3);
    assert!(matches!(
        &events[0],
        ControllerEvent::Warning { code, .. } if code == "HOTKEY_REGISTRATION_FAILED"
    ));
    assert!(matches!(
        &events[1],
        ControllerEvent::Warning { code, .. } if code == "HOTKEY_EVENT_QUEUE_FULL"
    ));
    assert!(matches!(
        &events[2],
        ControllerEvent::CommandRejected { command, .. } if command == "save_replay"
    ));
}

#[test]
fn execute_command_emits_events_after_releasing_controller_and_hotkey_locks() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    struct LockCheckingSink {
        state: Arc<shell::ShellState>,
        controller_unlocked: AtomicBool,
        hotkeys_unlocked: AtomicBool,
        reentrant_status_ok: AtomicBool,
        emitted_events: Mutex<Vec<ControllerEvent>>,
    }

    impl shell::EventSink for LockCheckingSink {
        fn emit(&self, event: &ControllerEvent) {
            let controller_lock_free = self.state.controller.try_lock().is_ok();
            let hotkey_lock_free = self.state.hotkeys.try_lock().is_ok();

            if !controller_lock_free {
                self.controller_unlocked.store(false, Ordering::Release);
            }
            if !hotkey_lock_free {
                self.hotkeys_unlocked.store(false, Ordering::Release);
            }

            if shell::recorder_status(&self.state).is_ok() {
                self.reentrant_status_ok.store(true, Ordering::Release);
            }

            if let Ok(mut events) = self.emitted_events.lock() {
                events.push(event.clone());
            }
        }
    }

    let dir = TempDir::new("shell-lock-check").expect("temp");
    let state = Arc::new(shell::ShellState::with_app_config(
        shell::build_controller(dir.path().to_path_buf()),
        None,
        AppConfig::default(),
        dir.path().join("config.json"),
        true,
    ));

    let sink = LockCheckingSink {
        state: Arc::clone(&state),
        controller_unlocked: AtomicBool::new(true),
        hotkeys_unlocked: AtomicBool::new(true),
        reentrant_status_ok: AtomicBool::new(false),
        emitted_events: Mutex::new(Vec::new()),
    };

    shell::execute_command(&state, &sink, &Command::StartCapture).expect("execute start");

    assert!(
        sink.controller_unlocked.load(Ordering::Acquire),
        "controller mutex was held during event emission"
    );
    assert!(
        sink.hotkeys_unlocked.load(Ordering::Acquire),
        "hotkeys mutex was held during event emission"
    );
    assert!(
        sink.reentrant_status_ok.load(Ordering::Acquire),
        "re-entrant status query failed during event emission"
    );
    let emitted = sink.emitted_events.lock().unwrap();
    assert!(
        !emitted.is_empty(),
        "expected events to be emitted from command"
    );
}

fn wait_until_ready(state: &shell::ShellState, sink: &CollectingSink) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        // Any command pumps pending engine transitions; ping is a no-op.
        shell::execute_command(
            state,
            sink,
            &Command::Ping {
                message: "tick".into(),
            },
        )
        .expect("pump via ping");
        let recorder_state = shell::state(state);
        if recorder_state == "Error" {
            panic!(
                "recorder entered Error before buffering: {:?}",
                sink.snapshot()
            );
        }
        if recorder_state == "Ready" && shell::buffered_packet_count(state) > 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("buffer never became ready");
}

fn wait_until_audio_ready(state: &shell::ShellState, sink: &CollectingSink, target: usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        shell::execute_command(
            state,
            sink,
            &Command::Ping {
                message: "audio-tick".into(),
            },
        )
        .expect("pump audio worker");
        if shell::audio_stream_count(state) >= target {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("audio workers did not produce the expected number of streams");
}

#[test]
fn command_handlers_drive_full_lifecycle_and_emit_events() {
    let dir = TempDir::new("shell-lifecycle").expect("temp");
    let state = shell::ShellState::new(shell::build_controller(dir.path().to_path_buf()), None);
    let sink = CollectingSink::default();

    shell::execute_command(&state, &sink, &Command::StartCapture).expect("start");

    let pre_ready_events = sink.snapshot();
    assert!(pre_ready_events
        .iter()
        .any(|e| matches!(e, ControllerEvent::StatusChanged { state } if state == "Buffering")));

    wait_until_ready(&state, &sink);
    let native_audio_requested = cfg!(windows)
        && (std::env::var("SILK_REAL_AUDIO").as_deref() == Ok("1")
            || std::env::var("SILK_REAL_MIC").as_deref() == Ok("1"));
    if native_audio_requested {
        // Native endpoint startup is asynchronous; the scripted source, when
        // present, may be the only source that has produced a packet yet.
        wait_until_audio_ready(&state, &sink, 1);
    } else {
        wait_until_audio_ready(&state, &sink, 2);
    }

    shell::execute_command(&state, &sink, &Command::SaveReplay).expect("save");

    let save_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < save_deadline {
        if sink
            .snapshot()
            .iter()
            .any(|event| matches!(event, ControllerEvent::ClipSaved { .. }))
        {
            break;
        }
        shell::execute_command(&state, &sink, &Command::Poll).expect("poll save worker");
        std::thread::sleep(Duration::from_millis(5));
    }
    let events = sink.snapshot();
    let statuses: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            ControllerEvent::StatusChanged { state } => Some(state.as_str()),
            _ => None,
        })
        .collect();
    assert!(statuses.contains(&"Buffering"));
    assert_eq!(statuses.last().copied(), Some("Ready"), "{statuses:?}");

    assert!(
        events
            .iter()
            .any(|e| matches!(e, ControllerEvent::SaveQueued { .. })),
        "save queued event missing: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ControllerEvent::ClipSaved { size_bytes, .. } if *size_bytes > 0)),
        "save events: {events:?}"
    );

    // The published file must exist on disk at the reported path.
    let saved_path = events
        .iter()
        .find_map(|e| match e {
            ControllerEvent::ClipSaved { path, .. } => Some(path.clone()),
            _ => None,
        })
        .expect("clip_saved event");
    assert!(std::path::Path::new(&saved_path).exists());
    assert!(saved_path.ends_with(".mp4"));
    let saved_bytes = std::fs::read(&saved_path).expect("read published MP4");
    assert_eq!(&saved_bytes[4..8], b"ftyp");

    shell::execute_command(&state, &sink, &Command::StopCapture).expect("stop");
    assert!(sink
        .snapshot()
        .iter()
        .any(|e| matches!(e, ControllerEvent::StatusChanged { state } if state == "Stopped")));
}

#[test]
fn display_and_window_sources_enumeration() {
    let displays = shell::display_sources();
    assert!(displays.is_ok());

    let windows = shell::window_sources();
    assert!(windows.is_ok());
}

#[test]
fn mp4_config_maps_to_direct_mp4() {
    let mut config = AppConfig::default();
    config.output.container = configuration::ContainerFormat::Mp4;

    let controller_settings = shell::controller_settings_from_config(&config);
    assert_eq!(
        controller_settings.output_options,
        muxer::SaveOptions::default()
    );
}

#[test]
fn default_config_sets_direct_mp4_output() {
    let config = AppConfig::default();
    let controller_settings = shell::controller_settings_from_config(&config);
    assert_eq!(
        controller_settings.output_options,
        muxer::SaveOptions::default()
    );
}

#[test]
fn controller_settings_maps_video_bitrate_none() {
    let mut config = AppConfig::default();
    config.encoding.video_bitrate_kbps = None;

    let controller_settings = shell::controller_settings_from_config(&config);
    assert_eq!(controller_settings.video_encoder_config.bitrate_kbps, None);
    assert_eq!(controller_settings.video_encoder_config.preset_level, 2);
}

#[test]
fn controller_settings_preserves_native_resolution_intent() {
    let config = AppConfig::default();
    let controller_settings = shell::controller_settings_from_config(&config);

    assert!(controller_settings.follow_source_dimensions);
    assert_eq!(controller_settings.video_encoder_config.width, 0);
    assert_eq!(controller_settings.video_encoder_config.height, 0);
}

#[test]
fn controller_settings_keeps_fixed_resolution_fixed() {
    let mut config = AppConfig::default();
    config.capture.output_resolution = configuration::OutputResolution::Res2560x1440;
    let controller_settings = shell::controller_settings_from_config(&config);

    assert!(!controller_settings.follow_source_dimensions);
    assert_eq!(controller_settings.video_encoder_config.width, 2_560);
    assert_eq!(controller_settings.video_encoder_config.height, 1_440);
}

#[test]
fn controller_settings_maps_custom_high_video_bitrate() {
    let mut config = AppConfig::default();
    config.encoding.video_bitrate_kbps = Some(80_000);

    let controller_settings = shell::controller_settings_from_config(&config);
    assert_eq!(
        controller_settings.video_encoder_config.bitrate_kbps,
        Some(80_000)
    );
    assert_eq!(controller_settings.video_encoder_config.preset_level, 2);
}

#[test]
fn controller_settings_preserves_canonical_codecs_and_maps_encoder_selection() {
    for codec in ["h264", "hevc", "av1", "avc", "avc1", "h265", "hvc1", "av01"] {
        let mut config = AppConfig::default();
        config.encoding.video_codec = codec.to_string();
        let controller_settings = shell::controller_settings_from_config(&config);
        assert_eq!(
            controller_settings.video_encoder_config.codec, codec,
            "expected codec '{codec}' to be passed through unchanged"
        );
    }

    for (selection, expected_pref) in [
        (
            configuration::EncoderSelection::Auto,
            encoder_api::EncoderPreference::Auto,
        ),
        (
            configuration::EncoderSelection::Hardware,
            encoder_api::EncoderPreference::HardwareOnly,
        ),
        (
            configuration::EncoderSelection::Software,
            encoder_api::EncoderPreference::SoftwareOnly,
        ),
    ] {
        let mut config = AppConfig::default();
        config.encoding.encoder = selection;
        let controller_settings = shell::controller_settings_from_config(&config);
        assert_eq!(
            controller_settings.encoder_preference, expected_pref,
            "expected encoder selection {selection:?} to map to {expected_pref:?}"
        );
    }

    for (fidelity, expected_mode) in [
        (
            configuration::VideoFidelityMode::Standard,
            configuration::VideoFidelityMode::Standard,
        ),
        (
            configuration::VideoFidelityMode::Clarity,
            configuration::VideoFidelityMode::Clarity,
        ),
        (
            configuration::VideoFidelityMode::Archival444,
            configuration::VideoFidelityMode::Archival444,
        ),
    ] {
        let mut config = AppConfig::default();
        config.encoding.fidelity_mode = fidelity;
        let controller_settings = shell::controller_settings_from_config(&config);
        assert_eq!(controller_settings.fidelity_mode, expected_mode);
    }
}

#[test]
fn recorder_status_serializes_fidelity_mode_truthfully() {
    let dir = TempDir::new("shell-fidelity-status-serde").expect("temp");
    let mut config = AppConfig::default();
    config.encoding.fidelity_mode = configuration::VideoFidelityMode::Clarity;
    let state = shell::ShellState::with_app_config(
        shell::build_controller_from_config(&config),
        None,
        config,
        dir.path().join("config.json"),
        true,
    );

    let status = shell::recorder_status(&state).expect("status");
    let json_val = serde_json::to_value(&status).expect("serialize status");
    assert_eq!(json_val["fidelity"]["requested"], "clarity");
    assert_eq!(json_val["fidelity"]["active"], serde_json::Value::Null);
    assert_eq!(json_val["fidelity"]["issue"], serde_json::Value::Null);
}

#[test]
fn shell_state_applies_and_persists_clip_sound_and_theme_settings() {
    let dir = TempDir::new("shell-clip-sound-theme").expect("temp");
    let config_path = dir.path().join("config.json");
    let initial_settings = AppConfig::default();
    assert!(initial_settings.application.clip_sound_enabled);
    assert_eq!(
        serde_json::to_value(initial_settings.application.theme).unwrap(),
        "classic"
    );

    let state = shell::ShellState::with_app_config(
        shell::build_controller(dir.path().to_path_buf()),
        None,
        initial_settings.clone(),
        config_path.clone(),
        true,
    );

    assert!(state.clip_sound_enabled());

    let mut next_settings = initial_settings.clone();
    next_settings.application.clip_sound_enabled = false;
    next_settings.application.theme = serde_json::from_str("\"ember\"").expect("ember theme");

    shell::apply_app_settings(&state, &next_settings).expect("apply settings");

    assert!(!state.clip_sound_enabled());
    let current = shell::app_settings(&state).expect("app settings");
    assert!(!current.application.clip_sound_enabled);
    assert_eq!(
        serde_json::to_value(current.application.theme).unwrap(),
        "ember"
    );

    // Verify persisted config on disk
    let loaded = configuration::load(&config_path).expect("loaded config");
    assert!(!loaded.settings.application.clip_sound_enabled);
    assert_eq!(
        serde_json::to_value(loaded.settings.application.theme).unwrap(),
        "ember"
    );
}

#[test]
fn shell_state_preserves_runtime_clip_sound_on_persistence_failure() {
    let dir = TempDir::new("shell-persistence-failure").expect("temp");
    // Point config_path to an existing directory so save fails when writing file
    let invalid_config_file = dir.path().join("directory_as_file");
    std::fs::create_dir_all(&invalid_config_file).expect("create dir");

    let initial_settings = AppConfig::default();
    let state = shell::ShellState::with_app_config(
        shell::build_controller(dir.path().to_path_buf()),
        None,
        initial_settings.clone(),
        invalid_config_file,
        true,
    );

    assert!(state.clip_sound_enabled());

    let mut next_settings = initial_settings.clone();
    next_settings.application.clip_sound_enabled = false;
    next_settings.application.theme = serde_json::from_str("\"ember\"").expect("ember theme");

    let result = shell::apply_app_settings(&state, &next_settings);
    assert!(result.is_err(), "expected persistence failure");

    // Runtime clip_sound_enabled and stored settings must not have changed
    assert!(state.clip_sound_enabled());
    assert_eq!(
        shell::app_settings(&state).expect("settings"),
        initial_settings
    );
}
