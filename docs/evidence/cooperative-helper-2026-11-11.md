# Cooperative capture helper evidence

Date: 2026-11-11. Windows x64, D3D11; original owned test application only.
Design and limits: ADR 0051. No installer/update, injection, or game attachment.

The later DLL-to-recorder integration is recorded separately in
`dll-helper-replay-2026-11-11.md`; this document describes the original helper-only gate.

## Reproduce

```powershell
.\scripts\test-game-capture-prototype.ps1 -Native -Hardware -Decode
```

The script builds `silk_capture_hook_helper.dll` using the explicit
`experimental-helper` feature, runs ABI tests/lint, and launches the
`--helper-test` scenario with WARP and hardware. The fixture launches only its
own executable, retains its child process handle, and validates the offered
PID/creation time/HWND before approving DLL load. The helper independently
validates the selected chain/window. No process enumeration/injection is used.

## Observed results

- Actual sibling-DLL loading, export lookup, and D3D11 detouring inside the owned
  child passed with Microsoft Basic Render Driver (WARP) and RTX 2080 Ti.
- Wrong ABI version and creation time were rejected before valid attachment.
  Nonowner-thread Stop calls and stopped-epoch reuse were also rejected.
- Independent rendering continued with all three shared surfaces held by the
  controller; no frame requests drive the renderer. Queues/transport are bounded,
  pipe IO is on separate threads, and renderer publication uses try_send.
- Cross-process red 640x480 pixels, resized green 800x600 pixels, and new-device
  blue 960x540 pixels matched. Each explicit rearm used a new approved epoch.
  Retained earlier resources preserved their pixels across the transitions.
- Stop detached capture without stopping the application: a later progress report
  showed additional Presents. The DLL stays loaded until process exit.
- Closing the controller's input pipe during active capture detached the helper;
  the fixture then presented 12 more frames before its controlled test exit.
- Terminating the owned target closed its pipe with no other-source fallback.
  A frame explicitly completed by readback before termination retained its pixels.
- Successful attachment is capped at three epochs for the DLL lifetime (at most
  nine exported textures cumulatively). A fourth attach is rejected; this is a
  conservative fixture quota, not automatic production resource retirement.
- Five consecutive WARP helper scenarios passed after adjusting the crash test
  to distinguish completed resources from interrupted GPU work. The final native
  gate also passed on both drivers.

## Important failed experiment

The initial crash scenario acquired a frame/header and immediately killed the
WARP producer before completing a readback. Some pixels were red and the remaining
rows were zero. This reproduced intermittently. Do not claim header delivery or
shared-mutex acquisition proves post-crash pixel completion. The corrected test
establishes completion before termination; it does NOT solve validity of in-flight
frames at process death. Production consumption must detect a dead source and
discard pending work; stronger publication/recovery is still outstanding.

## Validation and boundaries

The native prototype script passed: 20 portable prototype tests, two safe adapter
tests, one helper ABI boundary test, all existing native fixtures, both new helper
driver scenarios, and all seven opt-in recorder tests. Existing hardware GPU
replay reported 15 GPU-surface inputs and zero readbacks in the representative
final run; approved FFmpeg decoded its MP4. Those recorder/decode tests
still use the earlier fixture adapter, NOT the new DLL route.

Workspace format/Clippy/tests, frontend typecheck/lint/build, browser settings-save
regression, and strict dependency verification passed. No new external package
was resolved; helper dependency purpose/licenses are in THIRD_PARTY_LICENSES.md.

Unverified: real games, remote injection, arbitrary-thread/callback teardown,
DLL unload, fullscreen/Alt-Tab/minimize, physical GPU removal, anti-cheat/Valorant,
other vendors/multi-GPU, long-running performance, adversarial local DLL replacement,
and signing/antivirus/production distribution. Rust panics are caught at the ABI
boundary but no forced-panic scenario or access-violation recovery was tested.
Driver calls and pipe-writer joins lack hard completion guarantees. Output-pipe
failure handling exists but the native failure test specifically closes input;
it is not a complete recorder-process crash matrix.
