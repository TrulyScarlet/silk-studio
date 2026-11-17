# GPU-frame hook recorder bridge: native evidence

Date: 2026-11-11. Design: ADR 0049. Default-off development fixture only.

## Environment and reproduction

- Windows x64 10.0.19045/build 19045; NVIDIA GeForce RTX 2080 Ti,
  Windows-reported driver 32.0.16.1742, as in the preceding fixture evidence.
- Original controlled D3D11 fixture: BGRA8 640x480 (4:3), 30 FPS target, debug build.
- WARP/Microsoft Basic Render Driver also exercised separately.

```powershell
.\scripts\test-game-capture-prototype.ps1 -Native -Hardware -Decode
```

The existing script filters include the new GPU bridge tests; decoder execution
still requires the exact approved FFmpeg SHA-256, checked before use.

## Observed behavior

- Seventeen portable prototype tests passed. New pool tests cover queued/current
  publications, the next_event-to-resolve gap, distinct producer/consumer threads,
  retained encoder leases, weak-reference resurrection, full pools, dropped events,
  stale handles, contention, and invalidation. Two safe adapter tests also passed.
- All seven explicitly enabled native adapter/replay tests passed, along with
  the previous in-process and cross-process hook fixtures.
- GPU-mode capture delivered Gpu frame payloads with a session-scoped resolver.
  Retaining all three leases caused additional GPU copy work to be dropped, not
  additional texture allocation. Capture readback counters stayed zero.
- Retained pixels stayed unchanged under pressure and remained readable after
  stop. New resolves and session-resource access through the retired context
  failed. Holding two old contexts prevented a third pool from being allocated;
  releasing an old context restored startup within the six-surface budget.
- Actual GPU-mode hardware replay used `NVIDIA H.264 Encoder MFT`. The observed
  representative run reported 12 gpu_surface_inputs and 0 cpu_readback_inputs.
  Its MP4 was 640x480, about 864 ms, and every frame decoded with approved FFmpeg;
  the decoded primary-color pixel was visible rather than blank/black.
- WARP GPU delivery with `H264 Encoder MFT` software encoding reported 0 native
  GPU-surface inputs and 29 encoder readbacks in a representative run. It also
  saved/decoded correctly, but is explicitly NOT GPU-only.
- CPU bridge comparison replays continued passing with both software/hardware
  encoders. These inputs are CPU already, so GPU-input counters correctly stay zero.
- WARP rejected D3D11_CREATE_DEVICE_VIDEO_SUPPORT with 0x887A0004. Hardware device
  creation uses that flag; the explicit WARP compatibility device does not.

## Telemetry correctness

EncoderMetrics adds defaulted gpu_surface_inputs/cpu_readback_inputs fields so
older serialized metrics remain readable. Media Foundation records its actual
GPU-surface versus readback branch. The fallback wrapper accumulates per-attempt
deltas before retiring a failing backend; active-backend-only snapshots would
otherwise erase evidence of earlier readbacks. Both backward compatibility and
failed-attempt/non-double-counting behavior have unit tests.

## Final validation

- `scripts/check.ps1`: passed workspace formatting, Clippy, tests, frontend
  typecheck/lint/build. Unrelated hardware tests remained ignored.
- `scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode`: passed
  all portable/safe/native prototype gates and approved-decoder checks.
- `cargo test -p encoder-api`: 19 passed, including new telemetry contracts.
- Settings source-save browser regression, strict dependency verification,
  and `git diff --check`: passed. No additional dependency package was introduced.

## Limits

- This removes application CPU readback only on the tested hardware GPU path.
  It does not prove that every vendor/MFT uses GPU surfaces, nor describe WARP
  or a software encoder as GPU-only. GPU copies still occur; this is not zero-copy.
- No FPS/latency, resource-soak, or guaranteed nonblocking-driver benchmark was
  performed. The fixture's rendering remains consumer-commanded rather than an
  independently running game. Counters are route evidence, not performance proof.
- No game injection/attachment, concurrent Present lifecycle, cross-process
  resize/rearm, device-loss recovery, game audio sync, exclusive fullscreen,
  Alt-Tab/minimize, Valorant/anti-cheat, AMD/Intel/multi-GPU, other graphics APIs,
  HDR, signing, antivirus acceptance, or production packaging was verified.
- New code remains default-off and absent from the desktop dependency tree.
  No installer/update was published; only fresh generated test media was decoded.
