# Segment S6 Progress Log - Global hotkeys and tray

**Status:** In progress
**Started:** 2026-08-27
**Plan reference:** [implementation-plan.md](../implementation-plan.md) §10
**Design references:** [ADR 0011](../decisions/0011-win32-global-hotkeys.md),
[ADR 0012](../decisions/0012-tauri-tray-notifications.md)

## Task checklist

1. [x] Add platform-neutral hotkey binding validation and virtual-key mapping.
2. [x] Register the default global save chord on a dedicated Win32 thread.
3. [x] Detect registration conflicts and preserve a structured error.
4. [x] Bound hotkey control/event queues and debounce rapid repeats.
5. [x] Route hotkey activations through the existing controller commands.
6. [x] Expose validated optional start/stop bindings, live replacement, and
   versioned hotkey persistence; the editor is intentionally minimal S7
   groundwork.
7. [x] Add tray icon, status menu, and close-to-tray behavior.
8. [x] Add native save/failure notifications; desktop open/reveal actions remain
   pending with the S7 library opener surface.
9. [ ] Validate unfocused-window, game-focus, conflict, and repeat scenarios
   on an interactive Windows desktop.

## Implementation

- `crates/hotkeys/src/lib.rs` normalizes configured chords, maps supported keys
  to Win32 virtual-key codes, and owns a bounded `silk-hotkey-loop` using
  `RegisterHotKey`/`UnregisterHotKey` and `WM_HOTKEY`.
- `apps/desktop/src-tauri/src/shell.rs` registers the configured save chord
  (default `Ctrl+Shift+F10`) and dispatches activations through
  `Controller::execute`.
- `ShellState` drains hotkey events during the existing 100 ms UI poll without
  moving media or save work onto the UI thread.
- `build_hotkeys_for` registers the loaded `configuration::HotkeysSettings`
  instead of unconditionally using defaults.
- `configure_hotkeys` validates the existing `configuration::HotkeysSettings`
  model, synchronously confirms replacement, persists the full versioned
  `AppConfig` through the configuration crate, and rolls back registration if
  persistence fails.
- `get_hotkey_settings` exposes only the loaded hotkey subset to the frontend;
  the minimal editor leaves capture, audio, encoder, and storage settings for
  S7.
- Startup resolves the Tauri application config directory, loads/migrates
  `config.json`, and uses the loaded output/source naming settings for the
  controller.
- `apps/desktop/src-tauri/src/tray.rs` creates the Tauri tray menu, restores the
  main window from a left click, routes tray commands through the controller,
  updates menu enablement from recorder state, and hides the window on close
  unless an explicit quit was requested.
- The official `tauri-plugin-notification` bridge emits native desktop
  notifications for successful and failed saves. The desktop plugin does not
  expose the mobile action-button API, so open/reveal actions remain pending.

## Verification log

```text
cargo test -p hotkeys -- --test-threads 1           -> 5 passed
cargo test -p configuration -- --test-threads 1     -> 11 passed
cargo test -p silk -- --test-threads 1              -> 12 passed
scripts/check.ps1                                   -> all gates passed
```

## Known limitations

- Notification open/reveal actions remain pending; Tauri's notification action
  API is mobile-only, and the desktop opener command surface belongs with S7's
  library actions.
- Basic native save-success and save-failure notifications are implemented;
  notification preference persistence is not wired yet.
- Native registration and behavior while another application or game owns
  focus have not been tested interactively.
- The hotkey editor currently covers only the three chord fields; the full S7
  settings surface and configuration plumbing for capture/audio/encoding remain
  pending.
