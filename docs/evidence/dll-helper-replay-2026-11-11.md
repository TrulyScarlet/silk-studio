# Cooperative DLL helper to replay recorder evidence

Date: 2026-11-11. Windows x64/D3D11, original controlled application only.
Design: ADR 0052. Earlier cooperative DLL evidence is ADR 0051's milestone;
this is the first actual DLL-route RecorderEngine/MP4 gate.

## Reproduce

```powershell
.\scripts\test-game-capture-prototype.ps1 -Native -Hardware -Decode
```

The existing `_warp`/`_hardware` native filters include the seven new DLL recorder
tests. Only the exact SHA-256-approved FFmpeg executable is accepted. No downloads,
installed-app modification, real game attachment or production release.

## Actual native results

- `CooperativeDll` mode validates the owned target offer before DLL approval,
  imports the helper's three shared surfaces, and copies into bounded local GPU
  slots. No Render request drives this route; source rendering is independent.
- Both Microsoft Basic Render Driver/WARP with software H.264 MFT and RTX 2080 Ti
  with NVIDIA H.264 Encoder MFT saved and decoded a 640x480 DLL-route MP4.
- A representative basic hardware DLL replay reported 11 GPU-surface inputs and
  zero encoder readbacks. Capture counters also showed zero CPU readbacks.
  WARP's corresponding run reported 29 encoder readbacks, explicitly not GPU-only.
- Both driver scenarios saved three separate fresh-epoch MP4s: 640x480 red (4:3),
  800x600 green after resize (4:3), and 960x540 blue after own-device recreation
  (16:9). MP4 dimensions, keyframe start, sample count, monotonic sample times,
  and duration were checked. The replay buffer epoch increased on each rearm.
- Approved FFmpeg decoded ALL frames of each clip. Bounded 1x1 RGB sampling also
  checked EVERY decoded frame against that epoch's expected color, not only the
  first frame. The representative WARP clips checked 15/15/15 samples; hardware
  checked 10/21/20. No previous epoch's color appeared in those clips.
- Hardware rearm snapshots reported cumulative GPU-surface inputs 213/438/659
  and zero encoder readbacks throughout; capture readbacks also stayed zero.
  These counters include ongoing capture while test-side decoder/hash validation
  runs, not just samples in the saved clips, and are NOT performance measurements.
- The full-queue rearm/restart test rejected queued old-epoch events before the
  new FormatChanged and frame, then repeated successfully with a fresh child.
- Killing the exact owned helper after three copied inputs, while events were
  queued and unread, closed delivery, discarded queued media and invalidated its
  GPU context. Repeated next_event calls failed instead of returning pending frames.
- Killing the helper during RecorderEngine capture became terminal Error with a
  producer-exit warning. No new source epoch/automatic reattachment was started.

## Implementation boundaries

DeliveryGate closes terminally and rejects retired epochs. The rearm mailbox has
one entry and rejects invalid/odd/out-of-bounds dimensions before enqueueing. A
rearm retires the old resolver, drains old headers through Stop/Bye, revalidates
the same process/window/generation and expected next epoch, then publishes a
replacement GPU context with FormatChanged. RecorderEngine's existing format
path reconfigures encoding and resets replay; no production recorder change was
needed. QPC origin remains continuous across these fixture transitions.

Local surfaces retain the existing six-surface restart budget. Retained old
contexts/leases can intentionally prevent rearm; failure is terminal, not an
unbounded allocation. The DLL still permits only three successful attachments.
The fixture's bounded lifetime was extended from 30 to 60 seconds to accommodate
three replay saves and approved-tool verification, not to claim a soak test.
Stopping this adapter cleans up/terminates only its exact owned fixture child;
this is not a game-stop policy or arbitrary-app shutdown implementation.

## Validation

- Native prototype gate: 23 feature-enabled library tests, two safe adapter tests,
  one helper ABI test, existing direct/cross-process/lifecycle/helper fixtures,
  and all 14 opt-in recorder tests (10 WARP, four hardware) passed.
- New deterministic tests cover terminal/epoch delivery gating and the one-entry
  rearm mailbox; existing timestamp/replay-buffer tests remain passing.
- `scripts/check.ps1`: workspace format/Clippy/tests, frontend typecheck/lint/build
  passed. Default-feature prototype tests total 22 (mailbox test is native-feature-only).
- Settings-save browser regression and strict dependency verification passed.
  No additional third-party dependency was introduced.

## Untested assumptions and remaining risks

This rejects work after OBSERVED process death; it does not make check-to-copy,
check-to-deliver or subsequent encoder use atomic against concurrent process death.
ADR 0051's partial-WARP-pixels experiment remains relevant. Outstanding driver
operations may stall or be interrupted; no hard GPU-call/join deadline or
crash-atomic frame publication is established. Already encoded/saved valid media
is not erased on target death; pending capture frames are discarded.

No real games, remote injection, arbitrary-game teardown/DLL unload, exclusive
fullscreen/Alt-Tab/minimize, physical GPU removal, other vendors/multi-GPU,
anti-cheat/Valorant, frame-rate/latency/soak benchmark, game-audio synchronization,
signing/antivirus or production distribution was verified. Software fallback is
not GPU-only. Experimental code remains absent from the desktop dependency tree.
