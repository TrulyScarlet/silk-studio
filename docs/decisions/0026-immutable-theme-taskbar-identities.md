# ADR 0026: Immutable theme identities for Explorer's taskbar image cache

## Evidence

After ADR 0025, the user reproduced Default/Studio showing the Legacy yellow
icon. We confirmed the saved setting was Studio, WM_GETICON returned monochrome
pixels, and the installed shortcut pointed to Studio-v4.ico, while a focused
taskbar screenshot was yellow. Removing/re-adding a taskbar button and targeted
notifications for a mutable ICO did not reliably invalidate Explorer's image.

## Decision

Give the main window one of four deterministic AppUserModelIDs:
`<bundle identifier>.theme.{studio,classic,ember,vamp}`. Each identity always
refers to the same immutable ICO pixels. Publish relaunch command, display name,
and icon resource before assigning the ID. Windows can reuse its cached image
for that group without reusing another theme's colors. Repeated updates of the
same theme are no-ops before disk or shortcut work.

Remove the timer, pending-refresh state, and manual ITaskbarList DeleteTab/AddTab
calls. Do not hide the window, restart Explorer, delete caches, or generate an
unbounded sequence of identities. Keep the bundle/process identifier and config,
single-instance lock, storage, and notification identity unchanged. Only the
window and matching launcher/pin shortcut identities vary with the theme.

For shortcuts targeting this exact executable, save both IconLocation and
AppUserModelID together and notify the Shell. Other builds' shortcuts are not
modified. Shortcut COM initialization is balanced on failure as well as success.

The legacy frontend icon command retains its IPC signature, but reads the
currently saved theme from native ShellState. A delayed frontend effect cannot
override a newer save with an old theme. Startup and configure_settings also
apply the saved theme directly.

## Validation

- Unit tests cycle 800 theme selections and assert that only four identities
  exist and that no identity ever maps to different pixels. Existing ICO,
  metadata ordering, and shortcut-target tests remain.
- `pwsh -File scripts/test-taskbar-themes.ps1` builds an isolated native harness,
  performs 24 selections with rapid intermediate bursts, inspects Explorer's
  taskbar through UI Automation and MSAA, and classifies actual taskbar pixels. It checks
  button count as well as color. It does not launch capture or change the
  installed app's settings or shortcuts. Evidence goes to
  `target/taskbar-theme-test/`.
- The harness has its own application identity and display name so another
  running Silk instance cannot be mistaken for the test window.
- On this Windows 10 build 19045 host, the native stress run passed all 24
  transitions, including rapid bursts and revisits. The installed NSIS build
  then passed 12 saved-theme transitions through its real Settings UI. Every
  taskbar pixel classification matched the saved theme, and each check found
  one visible app button. The user-reported Legacy-to-Studio transition was
  exercised repeatedly. Installed-run evidence is in
  `target/installed-repeat-results.json` and its adjacent focused screenshots.
  Studio was restored and capture left stopped after verification.
- Formatting, Clippy, and all 55 desktop unit tests pass. The installer build
  and silent upgrade completed successfully. The former timed workaround's
  limited successful checks are explicitly superseded in ADR 0025.

## Limits

Verified platform coverage must be reported with test results; Windows 11 and
other DPI/taskbar layouts are not implied by a Windows 10 test. The pixel test
requires an interactive Explorer taskbar and currently uses its English UIA
labels and Windows PowerShell's desktop .NET accessibility support. It intentionally fails on an unsupported layout instead of silently
passing.

Window grouping and jump-list history are now per theme. Normal matching .lnk
pins are updated, but Windows-managed pins not exposed as accessible .lnk files
may retain a separate launcher. Updating such pin databases is outside scope.
No new dependencies or capture/hardware verification are introduced.

## References

- https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-id
- https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchiconresource
- https://learn.microsoft.com/en-us/windows/win32/shell/appids
