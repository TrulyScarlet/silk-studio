# ADR 0012 - Tauri tray and desktop notifications

**Status:** Accepted for S6
**Date:** 2026-08-27
**Segment:** S6

## Context

Silk must remain controllable while its main window is hidden or unfocused and
must confirm save outcomes without putting media work in the UI. The desktop
shell already uses Tauri, so the tray and notification surfaces should stay in
that boundary rather than introduce another platform integration layer.

## Decision

1. Use Tauri's built-in `tray-icon` feature for one tray icon with Open, Start,
   Stop, Save Replay, and Quit commands.
2. Rebuild the small tray menu when `StatusChanged` arrives so the status label
   and command enablement reflect `Stopped`, `Buffering`, `Ready`, or `Error`.
3. Restore and focus the main window on a left-click. A normal window close is
   intercepted and hides the window when minimize-to-tray is enabled; the Quit
   menu marks an explicit exit and stops the controller before exiting.
4. Use the official MIT/Apache-2.0 `tauri-plugin-notification` crate for native
   save-success and save-failure notifications. Notifications include the
   published path or stable failure code and message.
5. Keep notification open/reveal actions out of this slice. Tauri's desktop
   notification API does not expose the mobile action-button interface; those
   actions will use the S7 library/opener command surface instead.

## Consequences

- Tray commands share the same controller command path as UI buttons.
- The tray bridge performs no capture, encoding, muxing, or disk work itself.
- Notification behavior is native for installed desktop builds; development
  builds have the platform limitations documented by the plugin.
- Icon color/state variants, notification preference persistence, and Open/
  Reveal actions remain follow-up work. Hotkey settings persistence is handled
  by the versioned configuration path introduced as S6/S7 groundwork.

## Verification

- Tray menu state transitions and close policy have deterministic unit tests.
- Native tray, close-to-tray, and notification behavior still require an
  interactive Windows desktop run.
