# VALORANT supported-capture investigation — 2026-11-16

## Scope

Investigate non-injected, documented alternatives after ADR 0085. Do not change
game/Vanguard settings, remove window exclusions, download restricted SDKs, or
resume the experimental hook. No OBS implementation/source was inspected.

## Additional live checks

EnumWindows/GetWindowThreadProcessId/GetClassName/GetWindowDisplayAffinity audit:

- Selected process: 18912, creation time 134393228496281874.
- Visible window: HWND 1770878, class `VALORANTUnrealWindow`, title `VALORANT`.
- Reported bounds: (0,0)–(2560,1439), not minimized.
- GetWindowDisplayAffinity succeeded and returned 17 (`0x11`).
- Other top-level windows in that process were hidden helper/IME windows, also
  reporting 17. No alternate visible, nonexcluded game render window was found.

This rules out accidentally selecting Riot Client instead of the game's visible
HWND. The previously decoded monitor crop still showed underlying desktop/Riot
Client rather than the visible game: selecting the correct HWND does not prove
that Desktop Duplication includes its pixels.

Important precision: Silk's WGC probe fails at the capture-affinity preflight.
It is not evidence that a native CreateForWindow call returned access denied.
We deliberately do not remove that check to attempt capture of an excluded target.

## Documented alternatives reviewed

### WGC picker / alternate WGC entry point

Microsoft describes the picker as a consent/selection mechanism yielding a
GraphicsCaptureItem, consumed by the same frame-pool/session APIs. No documented
exemption from window display affinity was found. A picker is useful product UX,
but not established as a solution for this excluded window. Not tested as an
exclusion workaround.

Reference: https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture

### NVIDIA public NvFBC Capture SDK

NVIDIA states Windows support is deprecated; the last supported Windows 10
version is 1803, build 17134. The current SDK 9.0 download is Linux. NVIDIA points
Windows developers toward native capture APIs. This is not a supported modern
Windows backend for Silk; no SDK was downloaded, imported, or patched.

Reference: https://developer.nvidia.com/capture-sdk

### Windows AppRecordingManager

Microsoft documents recording the **calling app's own** content, including
historical-buffer saving. It is not documented as a general cross-process game
capture API. Do not claim that calling it from Silk captures VALORANT.

Reference: https://learn.microsoft.com/en-us/uwp/api/windows.media.apprecording.apprecordingmanager

### NVIDIA Highlights

NVIDIA describes integration by game/engine developers. It is not evidence of an
arbitrary external-app API for Silk to capture another game's render frames.
No integration or license agreement accepted.

Reference: https://developer.nvidia.com/highlights

### Official external recorder benchmark

Machine inventory: RTX 2080 Ti, driver 32.0.16.2124; NVIDIA App and its ShadowPlay
helper installed; Xbox Gaming Overlay package 7.326.8061.0 installed.

Tried the documented NVIDIA Alt+Z shortcut before and after launching NVIDIA App.
No NVIDIA Overlay/helper process appeared in the process check. NVIDIA App's home
UI opened and listed VALORANT. No recording was started, and no vendor-recorded
gameplay was verified. Native UI Automation exposed no descendant controls in
this run. This is a benchmark setup blocker, not proof that NVIDIA recording
cannot work. No overlay settings, game optimizations, driver or game settings were
changed; no existing recording was toggled blindly.

NVIDIA App documentation: https://www.nvidia.com/en-us/software/nvidia-app

A successful vendor recording would only establish that vendor's recorder works
in this session—not that Silk has access to its private capture API or that its
internals are hook-free. Explicit clip import could preserve some Silk library/
editing value, but would not replace Silk's own replay capture and is not
implemented or approved as the product solution here.

## Follow-up official NVIDIA benchmark

The overlay was successfully enabled through NVIDIA App's normal **Enable and
open overlay** control. Its initial UI showed `Record: Alt+F9 — Start`, confirming
no existing recording was interrupted. Microphone was temporarily turned off
through its normal shortcut before testing.

The official desktop recording prompt was accepted for this consented benchmark.
NVIDIA then displayed:

> VALORANT is preventing desktop capture, close the app or disable desktop
> capture and try again.

No newly written recording file was found under NVIDIA's configured output root
`C:\Users\Home\Videos\NVIDIA` during the attempted capture. Screenshot evidence:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-nvidia-benchmark-end.png`.

Desktop capture was then disabled through Settings → Video capture. A game-only
attempt confirmed foreground HWND 1770878, but did not produce a file. Invoking
record from the desktop overlay returned the desktop-capture consent prompt,
not a successfully attached game recording. This does **not** prove NVIDIA's
game-only backend cannot capture VALORANT.

NVIDIA App's Features settings explicitly state that enabling its overlay takes
effect **the next time a game is launched**. VALORANT was already running when
the overlay was enabled; no game restart was performed, since the current game
state/match could not be safely inspected through the excluded window.

At the end, temporary settings were restored: NVIDIA overlay disabled (confirmed
`IsShadowPlayEnabled=0`), desktop capture disabled (`DwmEnabled=0`), microphone
restored (`MicMode=2`, original UI was Always on). No driver installation, game
optimization, capture-affinity change or Vanguard change was made. NVIDIA App was
closed normally after testing; no recording is claimed. The temporary UI accessibility launch option
was process-local, not a persistent vendor configuration change.

The Silk replay probe now labels successful decode as **export verified only**
and writes `game_pixels_verified=false` / `game_capture_acceptance_passed=false`
until separate pixel inspection establishes the actual game image. This prevents
repeating the erroneous inference that a valid encoded file proves gameplay.

## Updated conclusion

### Installed-app update (2026-11-16, 23:35 local)

Stopped capture through the running app's accessible Stop control, then ended the
old PID 19804 for replacement. NSIS silent installation exited 0. Relaunched
installed Silk as PID 5952 from `C:\Users\Home\AppData\Local\Silk\silk.exe`.
Both installed/build executables contain `e861c20-dirty-capture-safety` and have
equal length. Binary comparison found only three differing bytes in Tauri's
bundle-type marker (`NSS` installed versus `UNK` build output), not media code.
Configuration SHA-256 before/after installation is identical:
`4B3B6DE3DD2E6C48350318C3205A04F525D6F2D5F62D864708FC04B9315A00E6`.
Backup: `C:\Users\Home\AppData\Local\Temp\opencode\silk-update-20261116-233549\config.json`.
Saved clips were not modified. The unsaved capture buffer was stopped for update.
VALORANT was no longer running at this update check, so no game-only benchmark
was performed in this step. This update is not a VALORANT compatibility claim.

Validation after the probe-label correction: `scripts/check.ps1` passed formatting,
Clippy, workspace tests, TypeScript, ESLint and frontend build. Interactive game
acceptance remains unsupported; a passing workspace gate does not change this.

Rebuilt local test installer now includes ADR 0085's guarded affinity rejection:
`target/release/bundle/nsis/Silk_0.1.4_x64-setup.exe`, SHA-256
`9E5C68C0073D34F5C66F85086D919D3E6E645FF1D6DF8885671BB4945B8106ED`.
Build identity: `e861c20-dirty-capture-safety`. A local updater signature was
generated; publisher signing is not claimed. Not installed or published. This
supersedes the earlier test installer hashes, not the current installed app.

No new supported, hook-free VALORANT backend was verified. Do not generalize this
session's affinity restriction to all FPS games or all anti-cheat software.

Next meaningful benchmark is enabling/using the official recorder through its
normal UI with visible state confirmation, then examining actual gameplay frames.
If it works, establish vendor-supported integration availability rather than
reverse engineering it. Riot/NVIDIA developer support is another route to clarify
supported capture interoperability. No tickets were submitted on the user's behalf.

Microsoft affinity reference:
https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
