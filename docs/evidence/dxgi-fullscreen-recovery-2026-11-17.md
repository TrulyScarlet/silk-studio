# DXGI fullscreen recovery — 2026-11-17

## Final validation and installation

After the null-focus regression fix, `scripts/check.ps1` passed all formatting,
Clippy, workspace tests, frontend typecheck/lint/build and dependency gates.
Built the NSIS candidate and installed/reopened
`e861c20-dirty-dxgi-fullscreen-recovery`. The installed executable contains that
build marker; saved configuration was preserved byte-for-byte, and no saved clip
files were modified. Current unsaved replay was discarded for the update.
Installer SHA-256:
`581738367BB7957C7B246BF29188C99890256296E2071AA8521723B858B2839F`.
Configuration backup:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-fullscreen-recovery-update-20261117-014840\config.json`.
This is a locally installed candidate, not a published release.

## Changes and deterministic checks

Guarded capture now handles temporary duplication interruption locally (ADR 0089).
`cargo test -p capture-windows --lib` passed: 25 passed, four native tests ignored.
New tests cover temporary/device-loss HRESULT classification and throttled retry
timing across three interruptions. The explicit native
`selected_game_duplication_recreation_keeps_gpu_and_snapshot` passed against the
running selected VALORANT window: three recreations, same device/shared context,
same frozen texture, and an existing lease still on the original GPU device.

## Live automatic-game replay

Evidence directory:
`C:\Users\Home\AppData\Local\Temp\opencode\valorant-guarded-shield-20261117-014254`.

`automatic_game_focus_replay` saved/decoded 25.055 seconds at 2560x1440 with
one format event, 771 GPU encoder inputs and zero encoder CPU readbacks.
`capture.jsonl` records actual duplication interruptions at 01:43:14.078 and
01:43:17.284, with same-device restorations at 01:43:14.347 and 01:43:17.517.
These were real Windows duplication interruptions, not synthetic device faults.
Replay duration remained intact despite both interruptions.

The foreground log verifies one complete game -> owned test window -> game cycle.
FFmpeg found a frozen interval from 3.588s to 10.855s, then changing frames after
return. A contact sheet shows actual VALORANT lobby pixels before, during and
after this interval. No >=0.1s black interval was detected. The second away
attempt was denied by Windows (foreground HWND 0); the strict two-cycle script
therefore FAILED rather than marking repeated focus-cycle acceptance as passed.
The remaining replay stays frozen, as expected without another verified return.

An earlier attempt in `valorant-guarded-shield-20261117-014050` likewise retained
25.055 seconds with one format event, and a 6.766-second freeze followed by live
frames; its second away request was also denied. It predates test diagnostics.

## Limits

The first full validation run exposed a pre-existing null-HWND focus predicate:
when Windows temporarily reports no foreground window, a null target could
compare equal. Focus checks now reject null targets, null foreground and zero
PIDs explicitly; the deterministic regression was extended.

The running game was not reconfigured. Its exact fullscreen setting was not
independently read/changed. Real duplication loss/recreation and one verified
focus-return cycle were tested, not two complete repeated cycles or all games.
Audio and genuine device removal were not tested. Export probe JSON deliberately
does not claim pixel/fullscreen acceptance; separate visual/frame evidence above
is narrower. No game/Vanguard/security settings, saved clips, or capture-affinity
flags were changed.
