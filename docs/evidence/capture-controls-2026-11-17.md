# Simplified capture controls — 2026-11-17

## Scope

UI-only refinement in `apps/desktop/src/SettingsModal.tsx`, `App.tsx` and
`App.css`; existing capture/encoding/replay implementations unchanged. Three
tasks (Game, Screen, Window) replace five technical source choices. Game has
automatic/manual selection. Existing saved types and identities remain compatible
(ADR 0090). No dependencies added.

Kept Silk's existing theme/type/layout rather than dealing a new visual direction
for this small component change. The screen answers "What should Silk record?"
with the task and selection controls first. The manual picker precedes help;
overlap/non-isolation remains visible, with extra limitations in a disclosure.

## Validation

- `scripts/test-settings-save-gate.mjs` passed with an isolated Edge profile/mock
  IPC: exact three labels, Screen/Window saves, failed-save retry, automatic and
  manual game saves, discard, disclosure opening, and preserving legacy
  `game-guarded` plus exact selected source on save.
- Real Edge screenshots reviewed for automatic/manual at 1440, 768 and 390px;
  Screen/Window at 1440px. First visual pass found a truncated automatic label
  and help pushing the manual picker down; second pass shortened the label/copy,
  moved the picker first and put optional details behind a disclosure. Controls
  inherit existing responsive stacking. Native app is Windows-first, not a new
  mobile product; narrow views retain the existing multirow settings navigation.
- Screenshot evidence:
  `C:\Users\Home\AppData\Local\Temp\opencode\silk-capture-controls-verified-20261117-021136`.
- Design source lint: zero failures, four warnings. Warnings refer to existing
  clock zero-padding (legitimate time formatting) and unrelated existing shadows;
  no restyle of unrelated app components was attempted. This is not a claim of a
  clean full-app visual-inspector audit.
- First full gate run failed the existing timing-sensitive
  `duplicate_start_is_rejected_while_running` integration test. No capture/engine
  code was changed to mask it; full rerun of `scripts/check.ps1` passed formatting,
  Clippy, workspace tests, frontend typecheck/lint/build. Browser regression passed
  again after adding disclosure and Screen/Window screenshot checks.

## Installation

Built/installed/reopened local candidate `e861c20-dirty-capture-controls`.
Installer SHA-256:
`EC95E85DCDA0A51A45CD20B5BCAD8677E4F79D27312EC224BF28C2BD58A94CB1`.
Saved configuration remained byte-for-byte identical; saved clips were not
modified. Configuration backup:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-capture-controls-update-20261117-021413\config.json`.
Current unsaved history was discarded for installation. Installed executable
build marker checked; process responding. Native UI Automation opened Capture
settings and verified Record, Game selection, Frame rate and Output resolution.
An initial UI lookup raced WebView startup; retry after loaded UI succeeded.

## Remaining limitations

No new game-footage acceptance test was performed for this UI-only change; the
previous native fullscreen recovery evidence still applies. DXGI overlap,
restricted-application, actual-resolution/device-loss and recognition-list limits
are unchanged. No claim that all games are recognized or Window is isolated.
