# ADR 0030: System preferences, player navigation, and theme overrides

## Decision

System owns startup, tray behavior, and optional automatic fullscreen playback.
Themes contains the four existing themes and six optional color overrides: main,
background, text, subtext, capture status, and replay buffer. Overrides live under
`application.colors`; `application.openClipsFullscreen` defaults to false. Serde
defaults preserve existing version-six settings and themes when these additive
fields are absent. Native validation accepts only six-digit hex colors. No new
dependencies are needed.

Colors are saved through the existing settings transaction. Drafts affect only an
in-dialog preview; discard and failed-save paths cannot recolor the application.
Reset removes an override and uses the selected preset again. Background changes
derive related surfaces; main color derives hover/border tones and contrasting
button text. Capture and replay signals are independent. Native taskbar/tray icons
continue using the selected preset identity, not arbitrary custom colors.

Fullscreen top and bottom controls now reveal only at their own pointer regions,
or via keyboard focus. Center motion only reveals the cursor. Side clip arrows
retain their existing behavior. Esc closes the player and exits fullscreen; F
switches fullscreen without closing. Space/K toggle playback; arrows seek five
seconds; J/L seek ten seconds; digits seek by percentage; comma/period pause and
step using the MP4 video track's frame rate; Shift-comma/period adjust speed.
Frame stepping is approximate on variable-frame-rate video. Editing inputs retain
their normal keys. Shortcuts are listed in the review panel.

Bookmarks retain preset colors and add a native color picker. Existing bookmarks
can also be recolored. Colors persist with each bookmark's timestamp, title, and
notes using the existing per-clip storage.

## Capture attribution

Recorder status resolves the same DXGI display ID as capture and examines visible,
uncloaked, non-minimized windows in Z order. It identifies the top application
covering that display's center and reports whether its bounds cover the monitor
(eight-pixel frame tolerance). It never reuses stale save-time attribution.
The Target tooltip explicitly states that Silk records the entire display, not
an isolated process. Small floating windows, occlusion, and unusual fullscreen
implementations limit what window geometry can identify; this is a useful label,
not proof of exclusive game capture. No hooks or injection are involved.

## Validation

Configuration tests cover round-trip persistence, old documents, invalid colors,
and all existing settings checks. TypeScript checking, ESLint, Rust formatting,
Clippy, and production builds are run. The browser regression
`scripts/test-system-player-theme.js` exercises save/discard isolation, individual
color signals, System separation, automatic fullscreen, center-hidden controls,
pause/seek/step/speed shortcuts, custom and edited bookmark colors, and Esc cleanup.
The earlier fullscreen and settings regressions are updated for the new labels
and Escape behavior. Native fullscreen and display-label checks use the installed
Windows app; browser automation cannot establish hardware capture correctness.

Results: all 101 configuration/desktop tests passed, as did both browser
regressions, type checking, ESLint, Clippy, formatting, Vite and NSIS builds.
The installed build loaded the existing configuration with its audio routing
unchanged. Native UI inspection confirmed System's three toggles, all six Themes
color controls, and a live `Brave · Windowed` target while that application was
visible. The System panel was visually inspected. Browser screenshot requests
timed out twice; a Themes desktop screenshot was occluded by another application,
so that panel was checked through browser assertions and native accessibility
controls rather than claiming a complete native screenshot review.
