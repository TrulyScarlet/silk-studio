# DXGI freeze continuity candidate — 2026-11-17

The selected VALORANT DXGI replay probe now asserts exactly one format event and
at least 24 seconds of retained history during its 25-second focus test. Export
passed: 25.033s, 2560x1438, 768 GPU encoder inputs, zero encoder readbacks, one
format event. Evidence directory:
`C:\Users\Home\AppData\Local\Temp\opencode\valorant-guarded-shield-20261117-003428`.

Foreground trace confirmed the game initially. The owned-window activation and
game-return calls were denied by Windows; by the final trace another process was
foreground. Freeze detection found a frozen interval from 10.433s to clip end.
Black detection found only a startup interval 0–0.267s. Thus full replay duration,
no format-driven restart and freezing on focus loss have evidence. Return/resume
and repeated away/return cycles do **not** have verified acceptance from this run.
Audio synchronization was not tested. Do not claim full game-only privacy.

The helper no longer relies on potentially blocking COM activation, writes focus
traces as they occur, and performs both black and freeze analysis. Its source is
`scripts/test-selected-guarded-focus.ps1`.

`scripts/check.ps1` passed all standard formatting/lint/test/frontend gates.
Built and installed the local candidate `e861c20-dirty-dxgi-freeze`, preserving
the configuration byte-for-byte. Saved clips were not modified. Capture was stopped
before replacement; the unsaved replay buffer was discarded for this update.

Installer: `target/release/bundle/nsis/Silk_0.1.4_x64-setup.exe`, SHA-256:
`621128EC27C772BDABD6A0AB3DEE99AD99BDF4091408CC0C470AF6B5D9F5BCD4`.
Config backup:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-dxgi-update-20261117-004033\config.json`.
This is an uncommitted local test candidate, not a public release. Production
routing is DXGI-only per ADR 0087; WGC is not used as a fallback.
