# ADR 0033 — Restore the HUD with a retained surface and adjustable chime

**Status:** Accepted at user request; gameplay performance validation pending
**Date:** 2026-11-08

## Context

The user requested restoration of visual feedback, less clip-time stutter, and
an adjustable chime for accessibility. ADR 0023 suspended the HUD after the
previous swapchain-backed implementation caused a persistent composed-presentation
mode in Valorant. Those historical measurements remain valid for that implementation;
they are not measurements of this replacement.

## Decision

Supersede the native-HUD production suspension in ADR 0023. Respect the existing
saved enable switch (including an explicit off preference); keep the default off.
Restore the HUD controls and test action. Windows notification toasts remain gated
separately. No graphics hooks or process injection are introduced.

Replace the HUD's flip-model DXGI swapchain with an IDCompositionSurface. Each
cue updates that surface once through BeginDraw/EndDraw, honoring the returned
pixel offset and DPI. Release the D2D target and end the surface update on errors.
DirectComposition still animates opacity/translation without application frame
ticks. Prepare the cue before showing the nonactivating window, and hide the HWND
after exit. Warm the drawing/text path once at startup while hidden. Keep graphics
resources on the existing worker, with a bounded command
queue and nonblocking controller dispatch. Coalesce pending cues within each bounded
drain. Only saved/failed/rejected outcomes open the production HUD: SaveQueued no
longer overlaps a visual transition with the start of export. The diagnostic API
continues to support queued cues.

Guard hold and exit timers with monotonic deadlines. KillTimer does not remove
already-posted timer messages; an obsolete expiry must not hide a replacement cue.
Re-arm early timer delivery for the remaining deadline, rather than waiting
another complete periodic hold interval.

Add application.clipSoundVolume, an integer percentage from 0 to 300. Missing
values load as 100, preserving the original level. Zero mutes; the independent
enabled switch remains. Scale the original embedded 16-bit PCM samples on one
audio worker with one pending queue slot. PlaySound runs synchronously only on
that worker, preserving buffer lifetime. Dispatch uses try_send and drops excess
cues rather than delaying recording. No system, game, or recorded-track volumes
change. The embedded asset has sufficient headroom for 300% (covered by tests).
The Test chime button uses the draft volume without persisting it; Save Settings
commits it. Playback-device failures remain nonfatal, and queue acceptance is not
proof of audible output.

## Dependencies and references

No new dependency or asset. Existing windows 0.62, windows-sys 0.59 (MIT OR
Apache-2.0) and the original embedded PCM WAV are reused; the dependency ledger
continues to apply.

- [Microsoft: IDCompositionSurface::BeginDraw](https://learn.microsoft.com/en-us/windows/win32/api/dcomp/nf-dcomp-idcompositionsurface-begindraw)
- [Microsoft: PlaySound and buffer lifetime](https://learn.microsoft.com/en-us/previous-versions/dd743680(v=vs.85))

## Validation and limits

Deterministic tests cover volume bounds/defaults/serialization, persistence failure,
exact PCM gain and headroom, mute, event selection, queue behavior, and stale timer
deadlines. A browser fixture covers draft preview, mute, save, reopen and discard.
The opt-in Windows native test exercises both HUD sizes, surface updates, hidden
idle HWNDs and preservation of foreground focus on this desktop.

Removing a swapchain eliminates a presentation path; it does not guarantee
independent flip, lower p99 frame time, or no stutter. A visible top-level window
can still cause DWM composition. No paired in-game PresentMon measurements were
made for this change. Retest baseline, idle, active cues, and recovery in the same
game session, including focus, actual visibility and capture exclusion. Exclusive
fullscreen visibility, mixed-DPI/multi-monitor behavior, capture exclusion across
drivers, audio-device audibility and hearing accessibility remain unverified.

Validation on 2026-11-08:

- `cargo fmt --all --check`: passed.
- `cargo clippy -p silk -p native-hud -p configuration --all-targets -- -D warnings`: passed.
- Configuration, native-HUD, replay-buffer and media-clock test suites: passed.
- `cargo test -p silk --bin silk -- --skip sharing::tests`: 56 passed.
  The complete desktop suite also ran; an unrelated, concurrently added
  `sharing::tests::cache_keeps_recent_drops_and_enforces_limits` failed with
  Windows Access Denied when setting a file timestamp. HUD/chime tests passed.
- `cargo test -p native-hud retained_surface_shows_updates_hides_and_preserves_focus -- --ignored`:
  passed on the actual Windows desktop after correcting early timer delivery.
- Frontend `npm run typecheck`, `npm run lint`, `npm run build`: passed.
- `scripts/test-hud-chime.js`: passed in isolated headless Chrome at 1440x900
  and 960x640 with mocked IPC (not an audible playback test). Screenshots inspected.
  Minimum-size document width matched the viewport, with no horizontal overflow.
- Settings source design lint: zero failures; one existing timestamp-formatting warning.
