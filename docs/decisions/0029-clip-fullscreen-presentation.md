# ADR 0029: Fullscreen clip presentation independent of review layout

## Problem

Native fullscreen enlarged the complete review dialog. The VOD panel, audio mixer,
and theater grid still occupied space, and theater viewport height caps could
clip the video. The custom titlebar lived above the dialog's stacking context.

## Decision

Keep fullscreen and theater preference in a persistent presentation component.
Render its root through a body portal so it can cover the custom titlebar. In
fullscreen, suspend theater layout rules and give the video stage the entire
viewport, using `object-fit: contain`. Review and editing panels are hidden.
Playback controls overlay the video and fade after inactivity, remaining available
on hover or keyboard focus. Exiting fullscreen restores the theater preference.

Previous/next arrows appear when the pointer reaches the corresponding screen
edge or the button receives keyboard focus. They follow the visible library's
sort/filter order, skip missing clips, and stop at the first/last clip. Navigation
keeps presentation mounted and replaces clip-specific content by path. Playback,
trim, and marker state initialize for the selected clip; switching pauses and
releases the previous media and ignores its outstanding metadata responses.
New clips open paused. Arrow keys retain their existing five-second seek behavior.

Native builds continue to use the existing window fullscreen command; browser
previews use the DOM Fullscreen API. F toggles fullscreen; Esc exits it, including
while a volume input is focused. No native capture/encoding changes or new
dependencies are introduced.

## Validation

`npm run typecheck`, `npm run lint`, Vite production build, and NSIS build pass.
`scripts/test-clip-fullscreen.js` is a browser regression check for Vite's explicit
preview mode. It provides fake fullscreen IPC and a locally served video, and
checks viewport bounds, containment, hidden panels, repeated F while paused,
theater enter/exit, five-clip navigation, media disposal, initial time, boundary
arrows, keyboard visibility, idle controls, and close cleanup. It never calls the
installed recorder's IPC. CSS transitions are disabled only within the test so
background-tab animation throttling cannot affect timing assertions.

The native installed app was also exercised with a saved Valorant clip: entering
fullscreen from theater produced a 2560x1440 window with the full video and no
review sidebar. Native previous/next navigation, edge-hover visibility, return to
theater, and entering fullscreen from standard view also passed. Inspection found
one remaining trim toggle; it was removed and covered by the browser regression.
Browser preview screenshot capture timed out twice; native window
screenshots were used for visual inspection instead. Other monitor aspect ratios
and touch-only navigation still need dedicated hardware checks.
