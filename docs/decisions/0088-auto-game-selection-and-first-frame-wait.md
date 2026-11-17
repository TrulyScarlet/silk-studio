# ADR 0088: Recognized-game discovery and first-frame waiting

Date: 2026-11-17

Expose two DXGI game choices: auto-detect recognized foreground games, or select
an exact running process/window. New configurations default to automatic mode;
existing saved settings are not overwritten. Recognition initially uses a small,
explicit executable-stem list, not titles, full-screen heuristics or arbitrary
foreground windows. Unknown games require manual selection. This is a usability
classifier, not a security identity/anti-spoofing boundary.

The automatic router polls at most twice per second while unattached. It selects
a fresh PID/creation-time/HWND identity only when a recognized game is foreground.
It retains that target through Alt-Tab, rather than adopting browsers or switching
games mid-replay. Source loss uses engine recovery, then fresh discovery. Affinity
exclusions still fail closed; WGC is never a fallback.

Before attachment the router emits Idle, not pixels. Its synthetic enumeration
entry supplies only an initial encoder configuration placeholder; the actual
source format is delivered by the selected capture session. No desktop source is
chosen to fill the gap. Background selection/encoding remain outside the UI.

Guarded capture likewise emits no initial black frames. Until the first focused
game image it stays Idle and does not create the media timestamp origin. After
the first image it emits the existing frozen snapshot while unfocused. Idle
receiver timeouts are not end-of-stream, so waiting cannot spuriously restart
the replay. Genuine resize/device loss retains the existing epoch limitations.

Initial recognition: VALORANT, CS2, Apex Legends, Fortnite, Overwatch, Rocket
League, Terraria and Helldivers 2. Names are not a compatibility claim; each game
still requires native saved-clip verification. Broad catalog discovery, persistent
user-added recognition rules, and multiple concurrently running game preferences
are not included in this narrow initial implementation.

Validation: standard `scripts/check.ps1` passed. Deterministic routing regressions
cover recognition rejecting desktop/browser/launcher names, and an unattached
automatic session returning Idle with no monitor/backend/GPU context. Native
first-focus startup, live discovery, and game-switch acceptance remain pending;
these changes have not been packaged/installed yet.
