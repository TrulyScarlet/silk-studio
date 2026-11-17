# Production WGC lifecycle and odd-window replay evidence

Date: 2026-11-12. ADRs 0069–0070. Exact owned test windows only.

## Production changes

WGC now uses real queued/delivered/resolved texture leases rather than cycling
through textures retained by an encoder. Eight active staging slots and sixteen
active/retired surfaces maximum apply across resize/restart. FormatChanged installs
the new resolver pool at next_event, not in a callback beneath a delivered frame.
Item close and backend failure invalidate queued media out-of-band. Setup failure
cleanup unregisters handlers. An empty receive returns Idle after a 50-ms tick,
allowing the video worker to service stop without fabricating frames.

New production window IDs include PID, process creation FILETIME and HWND. Stale
PID/HWND-only settings require explicit reselection; they are not silently upgraded.
Lifetime and capture exclusion are rechecked at setup and delivery. No desktop
fallback or protected-game loading was added. The UI labels stale saved selections.

Odd-sized BGRA input is now distinguished from even NV12 output. Hardware MFT
enumeration no longer includes software transform classes under hardware labels.

## Native WGC tests actually run

`wgc_owned_native.rs` launches the exact owned bootstrap renderer and captures its
selected HWND using the production WGC implementation, even while an unrelated
owned blue window covers it. It does not capture a display or a real game.

Six tests passed, then passed three additional consecutive repetitions, then passed
in the complete native gate:

- Retained red frame stays red while fresh restored-window frames turn magenta.
  Target exit rejects already-queued media and retires context; retained pixels
  remain valid after stop.
- Retain all eight red publications; new content cannot overwrite any of them.
  Idle returns promptly under saturation; releasing one lease permits new magenta.
- Minimize yields Idle rather than monitor fallback/source-loss; restore produces
  newly observed magenta content.
- Borderless resize preserves the old delivered-frame resolver gap. New format
  retires old handles while resolved pixels stay red. A third allocation with a
  retained old surface fails the sixteen-surface budget rather than growing.
- Canonical lifetime ID is enumerated. Legacy HWND-only/PID-only IDs and wrong
  creation time fail before any backend/context starts.
- Fixture sets WDA_EXCLUDEFROMCAPTURE on its own HWND after WGC queued frames.
  Next delivery returns PermissionDenied, discards queued media and retires context.
  No game's exclusion settings were changed.

WGC can initially emit transparent startup content. The first saturation test
incorrectly assumed the first eight frames were red and failed on a transparent
sample. The test now waits for eight observed red publications; production code
does not replace or manufacture startup pixels. Final native gates pass.

## Actual replay and hardware observations

The test uses production DisplayCapture with a `game:` lifetime-pinned source and
asserts backend `wgc-window`, through RecorderEngine and MP4 saving. A test-only
wrapper observes actual FormatChanged dimensions; it does not implement capture.

Observed WGC content: **336×235 BGRA**. Encoded output rounds to **336×234 NV12**.
The saved clip contains selected red-window content, not the unrelated covering
window. Approved FFmpeg decodes every saved frame and checks the primary color.

Before fixes, GPU conversion rejected 336×235 and a HardwareOnly run fell through
to software because enumeration had incorrectly labeled a software transform as
hardware. That failed run is not hardware acceptance evidence.

Final complete-gate results after both fixes:
- WARP-rendered owned window / explicit software encoder: **833-ms MP4**, 25
  decoded color samples, H264 Encoder MFT, 292 reported encoder readbacks.
- Hardware-rendered owned window / hardware-only encoder: **866-ms MP4**, 26
  decoded color samples, NVIDIA H.264 Encoder MFT, 279 reported GPU-surface inputs,
  **zero reported encoder readbacks**.

These are application counters, not FPS, zero-copy or vendor-internal proofs. WGC
itself uses the system-selected capture adapter; a WARP-rendered source does not
mean WGC's capture device is WARP. Native hardware conversion also passed actual
64×64 RGB -> 64×64 NV12 and odd 65×63 RGB -> 64×62 NV12 white-pixel checks.

## Validation

- `scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode`: passed all
  existing native suites, six new production-WGC native tests, native hardware
  conversion checks and **51 native recorder tests (34 WARP / 17 hardware)**.
- `scripts/check.ps1`: passed fmt, workspace Clippy/tests, frontend typecheck,
  lint/build. Capture-windows: 15 unit tests; recorder-engine: 24 unit tests;
  gpu-windows: ten safe tests; encoder-media-foundation: 34 safe tests.
- Strict dependency verification and standalone settings-save regression passed.
- Only targeted generated prototype artifacts were cleaned after Cargo dry-run
  identified 1.9 GiB; no source/clips deleted and disk guards remained enabled.

The owned D3D12 recreation route also passed again: twenty-recreation timed runs
checked 6,648 WARP and 5,678 hardware publications over about sixty seconds each.
This does not turn healthy recreation into actual device-removal recovery.

## Limits

### Subsequent restart and timed checks

An additional cross-session budget test found Item.Size (350×242 here) differed
from first ContentSize (336×235). Allocating both pools made a second session fail
while eight old frames were retained. Staging allocation now waits for observed
ContentSize; the second session fits sixteen total surfaces, a third session fails
while sixteen are retained, and a fresh session succeeds after releasing them.
The redundant initial format/pool is no longer published.

The eight-test native WGC suite subsequently passed, including the restart test
and a 60-second retained-surface soak: **3,603 frames, 144 fresh magenta checks**,
with original red pixels still valid after stop. The resize test was corrected to
wait for observed yellow content because WGC can also emit transparent transitional
frames immediately after resize; a failed first-frame assumption is not reported
as a successful run. Updated software/hardware replay tests both passed saving/
decoding 833-ms clips and stopping the engine while the target was minimized in
under two seconds. Hardware remained NVIDIA H.264 with zero encoder readbacks.

### Outstanding acceptance scope

Further bounded-pressure work reserves two format slots in an eighteen-event
channel (sixteen possible live publications plus two format boundaries). A new
native test pauses consumption through initial pool saturation and resize, then
observes both ordered formats and fresh yellow pixels without losing the format
event to a full frame queue. The eight non-soak native tests pass together; the
unchanged sixty-second soak also passed in the full gate.

Capture device health is now checked before setup/delivery, and typed DeviceLost
enters the engine's bounded source-recovery path. A deterministic regression
observes encoder reconfiguration and subsequent encoded frames, not a fatal
CAPTURE_DEVICE_LOST worker event. Recorder-engine now has 25 unit tests. This is
dispatch validation, not evidence of a real WGC driver-reset recovery test.

Final full gate INCLUDING health/format-capacity changes passed **53 native
recorder tests (35 WARP/18 hardware), fifteen D3D12 lifetime/recreation/removal
native tests, nine WGC native tests** and all prior suites. Final WGC soak observed
3,602 frames in sixty seconds and 144 fresh color checks. Workspace/frontend,
strict dependency verification and settings-save regression passed again;
recorder-engine has 25 unit tests. No unexecuted hardware reset is implied.

### Local development executable

`cargo build -p silk --features tauri/custom-protocol` passed with embedded frontend
assets. Output: `target/debug/silk.exe`, 25,097,728 bytes. SHA-256:
`FF8F59C61C440E5EF0710D7FE4E11DEF0E41FC8231E51DCCE4BF688A80C10FD1`.
Authenticode reports NotSigned. This is a local development executable, not a
release/installer. It was not auto-launched or installed, so no user's persisted
capture selection was activated. Normal desktop dependencies were checked to
exclude capture-hook-prototype, capture-hook-helper and MinHook.

After validation, Cargo dry-run identified a further 1.1 GiB of generated prototype
artifacts; only that package's generated build cache was cleaned. The desktop
executable and source/clips were preserved. About 2.25 GB disk space remained and
no owned fixture processes remained. Final formatting and tracked/new-file
whitespace checks passed with CRLF recognized. An initial check disabled normal
line-ending handling and falsely flagged existing CRLF lines; no unrelated files
were rewritten. No source rollback, push or release was performed.

Real games, exclusive fullscreen, protected-game hooks, cross-vendor hardware,
driver-reset recovery and shipping/signing remain unverified/unimplemented. The
six native lifecycle tests use one Windows/GPU setup and owned windows. Target
checks are not an atomic transaction against future OS state changes. Retired
surface budgets can reject rapid resize while callers retain old frames. Source
silence produces Idle, not invented repeating media. All work remains uncommitted;
no installer, push, release or PR produced.
