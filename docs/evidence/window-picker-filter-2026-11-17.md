# Windows helper filtering in source pickers — 2026-11-17

## Scope and decision

`crates/capture-windows/src/enumerate.rs` omits exact executable stems
TextInputHost, InputApp, ShellExperienceHost, StartMenuExperienceHost, SearchHost,
SearchApp and LockApp from enumerated window sources, case-insensitively. This
filters input/search/lock/shell helper clutter from both manual Game and Window
pickers (which share enumeration). Existing title/tool-window rules remain.

Do not filter localized application titles, generic Host suffixes, explorer or
ApplicationFrameHost: real applications can use these hosts. Unknown games and
ordinary applications remain selectable. This is a small usability filter, not
an identity/security boundary or a comprehensive games-only classifier. No
processes are stopped, no OS/game settings changed, no dependencies added, and
capture/encoding/replay behavior is unchanged. Existing selected source IDs are
not rewritten; an old saved helper identity may still be displayed by the UI's
stale-selection fallback until the user selects another target.

## Validation

- Two deterministic tests passed: helper names/case variants excluded; known and
  unknown games, real apps, generic hosts, missing metadata and near-name matches
  retained.
- Opt-in native enumeration test passed with
  `SILK_PICKER_EXPECT_APP=VALORANT-Win64-Shipping`: seven application windows,
  no filtered helpers, valid source identity syntax, VALORANT retained.
- `scripts/check.ps1` passed formatting, Clippy, workspace tests and frontend
  typecheck/lint/build. No game-footage test was repeated for this list-only
  change. Broader Windows versions/locales were not tested.

## Installation

Built and installed/reopened `e861c20-dirty-picker-filter`. Installed executable
build marker verified; saved configuration preserved byte-for-byte and saved
clips untouched. Unsaved replay discarded for update.

Installer SHA-256:
`4AFDBCAAFAA74095174D343DEA67265A0435094AD2CB7A9F43FB303CA65A3476`.
Configuration backup:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-picker-filter-update-20261117-022824\config.json`.
