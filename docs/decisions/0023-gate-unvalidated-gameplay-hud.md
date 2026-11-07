# ADR 0023 - Gate unvalidated gameplay HUD in production

**Status:** Accepted
**Date:** 2026-11-04

## Context

The controlled Valorant HUD benchmark in
`docs/evidence/native-hud-valorant-2026-08-31.md` failed the presentation
gate: the shown native HUD switched the game from hardware-independent flip to
composed presentation, including while no cue was being drawn. Disabling
capture exclusion alone did not restore that mode. Detaching the visual while
idle helped only while idle; active cues still latched composition. The
`attached-hidden` diagnostic candidate has not been measured.

The overlay setting defaults to off, but saved user configuration can enable
the unsafe production HWND at startup or after changing settings. The settings
UI also suggested disabling the HUD guaranteed zero FPS impact, which has not
been established for the recorder or Windows notifications.

## Decision

Keep the production in-game visual-feedback gate closed irrespective of a
previously saved overlay or Windows notification preference. Neither app
initialization nor settings reconfiguration creates a native HUD window.
Event dispatch cannot produce HUD cues or Windows save-event toasts, and the
test-cue command explains the suspension. Do not erase saved preferences;
the settings UI shows both visual controls as unavailable while retaining the
audible chime and in-app/tray feedback. The standalone diagnostic benchmark
still constructs `NativeHud` directly, so its controlled experiments remain
possible without exposing an unvalidated gameplay HUD to users.

Reopen the production gate only after a paired real-game baseline, idle,
visible active-cue, and recovery run establishes the presentation mode,
frame-time p99, cue visibility, focus behavior and capture behavior across the
intended hardware/game matrix. A passing unit test or idle-only run is not
release evidence. Windows toasts need their own presentation test before
re-enablement. This decision avoids the *known HUD-induced stutter*, not
all possible recording, encoding, audio or system-level FPS impact.

## Verification

The desktop HUD runtime tests cover startup with a saved enabled preference,
settings reconfiguration and cue dispatch while the gate is closed. The
native HUD's separate diagnostic tests and benchmark remain available.
