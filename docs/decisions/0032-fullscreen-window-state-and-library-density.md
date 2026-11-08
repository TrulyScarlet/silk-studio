# Fullscreen window state and library density

## Problem

On the installed Windows app, entering fullscreen from a maximized window left
the maximized state set. The outer window measured 2560 × 1440 while its client
area was 2560 × 1400, matching the taskbar work area. Fitting a 16:9 clip into that
shorter client area produced side bars; activating another monitor also exposed
a clipped native caption. The frontend already occupied its entire client area,
so a CSS crop or stretch would have hidden the symptom and lost image content.

The previous exit handler also called `SetWindowPos` with the primary work area
and system border metrics, even when the window belonged to another monitor or
was not maximized before fullscreen.

## Decision

Serialize fullscreen transitions on the Tauri window thread and complete their
bounded IPC reply after the transition. Remember whether the window was
maximized, unmaximize before entering borderless fullscreen, and preserve the
selected monitor if restoring the normal window would move it. On exit, let
Tauri restore normal placement and reapply maximization only when appropriate.
Repeated exit requests are no-ops. Remove the primary-monitor sizing workaround.
Keep `object-fit: contain`; matching-aspect clips fill the monitor without
cropping, while different-aspect footage remains fully visible.

Group collection and game filters in one row. Wide windows share a row with the
library title and toolbar; narrower windows wrap those groups. Folder, storage,
and deletion metadata use one compact line. Remove the generic subtitle and
redundant indexed-count status; action confirmations and errors remain visible.

## Validation

- Reproduced the original native client/work-area mismatch. Manually clearing
  maximization before entering fullscreen restored 2560 × 1440 client bounds
  and eliminated the caption/side bars when clicking the other monitor.
- `scripts/test-fullscreen-focus.ps1` checks native outer/client/monitor bounds,
  styles, and focus loss/regain through a temporary test window on the other
  display. It saves a screenshot without refocusing Silk.
- `scripts/test-library-compact.js` checks actual iframe viewports at widths
  960, 1280, 1440, 1920, and 2560: filter alignment, no document overflow, no
  overlap, and compact header height. The wide header measures 90 CSS pixels.
- Existing hotkeys/Favorites and fullscreen browser regressions pass.
- TypeScript, ESLint, Rust formatting, Clippy, and 55 shell tests pass. The existing
  capture recovery test requiring explicit hardware configuration remains ignored.
- Built and installed the NSIS release. Native focus-loss/regain checks pass on
  the 2560 × 1440 monitor and the 1920 × 1080 monitor at offset (2560, 87).
  Screenshots confirm the entire matching-aspect clip fills both displays with
  no clipped caption. Maximized placement restores exactly; normal placement on
  the second monitor and theater mode restore, and closing the player does not
  move or resize the window. The library layout was inspected at native maximized
  and normal window sizes. Preview snapshots timed out twice, so native
  screenshots supplied the visual inspection.

## References and limits

The installed Tao 0.35.3 implementation was inspected to understand its maximized
non-client sizing. No dependency source was copied or modified. Microsoft's
[non-client area documentation](https://learn.microsoft.com/en-us/windows/win32/gdi/nonclient-area)
and [SetWindowPos contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos)
describe the frame/client distinction and frame recalculation behavior.

No dependencies or capture/encoding changes. Monitor disconnection during an
active transition is not covered by the two-display regression.
