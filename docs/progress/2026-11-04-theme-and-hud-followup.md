# Theme and HUD follow-up

## Desktop themes

The four existing theme settings now affect window ground, panels, controls,
selection accents and the replay signal rather than changing only the signal.
The default Silk Studio theme remains neutral. Theme cards render miniature
examples using the same theme tokens as the app; capture configuration is not
changed by switching themes.

Validation: desktop TypeScript typecheck, ESLint, Vite production build, source
design lint (0 failures; one existing clock-padding warning), and app inspector
at 1440, 768 and 390px (0 failures/warnings) passed. A browser interaction check
selected all four themes and confirmed that the live window and each preview
used the same ground and accent values. The settings screen was inspected at
1440 and 390px. In the web preview, Tauri event registration emits errors
because the native bridge is absent; native settings persistence was not tested.

## Native confirmation HUD

No production HUD behavior has been changed or enabled. The benchmark harness
SelfTest passes, including its diagnostic `attached-hidden` argument path.
`docs/evidence/native-hud-valorant-2026-08-31.md` remains the source of truth:
turning capture exclusion off alone did not restore hardware-independent flip;
idle detached-root did, but active cues still latched the tested game into
composed presentation. The `attached-hidden` candidate has not been measured.

Next gate requires a stable foreground Valorant session, an elevated PresentMon
run and the standalone diagnostic HUD benchmark. Run paired no-HUD baseline,
idle, active cue, and no-HUD recovery conditions in one renderer session, with
capture exclusion held constant. Then repeat with `attached-hidden` as the only
changed variable, confirm that cues are actually visible, and analyze recovery
between cues and p99 pacing as well as presentation mode. Do not enable the
production HUD or claim an FPS fix before the measured gate passes. At this
session the game and PresentMon were not running; live evidence is unavailable.

Later follow-up: ADR 0023 closes the production native-HUD and Windows-toast
paths even when saved preferences enabled them. The chime remains available.
Intel PresentMon Console 2.6.0 (signed), the diagnostic benchmark binary, and
the capture/analyzer script SelfTests are prepared on this machine. There is
still no paired in-game frame-pacing measurement for this configuration.
