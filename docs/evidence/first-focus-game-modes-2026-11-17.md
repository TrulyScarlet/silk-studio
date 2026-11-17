# Automatic/manual first-focus game capture — 2026-11-17

After testing, `scripts/check.ps1` passed and the local candidate was built and
installed as `e861c20-dirty-auto-first-focus`. Configuration was preserved
byte-for-byte and saved clips were not modified. Installer SHA-256:
`12B8080C96D90E061EDB30A4676450CBBD15C2E4CD53EB7AB3A79F2B7A50756C`.
Config backup:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-first-focus-update-20261117-012948\config.json`.
Not published; existing saved mode was not automatically changed to auto mode.

Native hardware tests in `native_selected_game_replay.rs` exercised both choices.
Each required four seconds of zero buffered video packets and zero buffer duration
while the target game was unfocused, then restored/focused VALORANT, recorded
25 seconds and decoded the saved MP4 with approved FFmpeg.

## Automatic mode

`automatic_game_waits_for_first_focus` passed. Source `game:auto`, one format
event, 2560x1440, 25.054s saved, 774 encoder GPU inputs, zero encoder readbacks.
First-frame inspection showed actual VALORANT lobby pixels. Black detection at
0.1s minimum found no black interval. Evidence:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-auto-first-focus-20261117-012109`.

## Manual mode

`manual_game_waits_for_first_focus` passed after fixing the test setup to wait for
asynchronous foreground activation before destroying its owned waiting window.
Source exact PID/lifetime/HWND, one format event, 2560x1438, 25.044s saved,
777 encoder GPU inputs, zero encoder readbacks. Black detection at 0.1s minimum
found no black interval. First frame showed actual VALORANT store pixels, but
also the owned setup window during its close transition and desktop/taskbar edges.
This is evidence of DXGI's overlap/transition limitation, not isolated window
capture or a privacy acceptance pass. Evidence:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-manual-first-focus-20261117-012505`.

Prior manual attempts stopped correctly when the game was already foreground or
when Windows had not yet completed the focus request. Those failures were setup
failures, not passed product acceptance.

## Limits

Video-only startup tests; audio synchronization, repeated focus cycles, live game
exit/restart, switching between multiple recognized games, UI startup/tray behavior,
and all other recognized titles remain unverified. Exact executable recognition
does not establish that every matching process is trustworthy. Pixel-inspection
findings are separate from machine probe JSON, which conservatively marks game
pixel acceptance false. No WGC, injection or capture-restriction modification used.
