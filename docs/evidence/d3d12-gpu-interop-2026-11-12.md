# Owned D3D12/D3D11 GPU interop evidence

Date: 2026-11-12. Windows x64; DXGI hardware adapter 0 and WARP. ADR 0056.
Default-off `d3d12-gpu-fixture`; no production source route or external target.

## Implemented ownership boundary

The owned windowed fixture captures its selected current D3D12 backbuffer into
three fixed shared DEFAULT-heap BGRA8 textures. Copies restore source PRESENT and
destination COMMON. A shared produced fence signals completion. A D3D11 device
on the exact same adapter LUID opens these textures plus produced/consumed fences.
Handles are unnamed, noninherited and RAII-closed after local opening. They are
never sent to another process; this is API/device interop, NOT cross-process IPC.

Outside rendering, the bridge polls producer completion. It reserves a local
output publication before scheduling the GPU Wait/copy; full local pools schedule
no consumer work and may discard only a producer-complete slot. Successful D3D11
copies signal consumed and Flush the protected immediate context before publication.
A producer slot is reusable only after that consumer fence completes. No CPU
fence wait, Map, software pixel conversion or readback occurs in this bridge.

Three shared inputs and three local encoder outputs bound the new bridge. Existing
LocalGpuBridge handles/publications protect queued events and resolved encoder
leases independently of the shared-input fence. The adapter retains a delivered
publication through the next_event -> resolve gap. Output restart budget remains
six surfaces per shared controller budget. The fixture's reused offscreen core
still allocates its fixed unused targets/readback buffers in GPU mode; they are
not mapped by this path, but this is not a minimal-allocation/zero-copy design.

Resize/cancel retires unresolved publication handles, drains producer and consumer,
and resets source-bearing lists before releasing backbuffer references. Resolved
D3D11 leases retain their independent texture/device after stop. Resize still
terminates capture rather than rearming it. Uncertain completion retains native
resources and the core's creation reservation; forced-timeout/device-loss behavior
was not exercised, and OS/driver calls and worker joins have no hard time bound.

## Observed native and adapter results

- New deterministic dual-fence tests cover producer-complete versus consumed,
  full rings, stale sequence/invalid releases, ordered completion, discard only
  before consumer submission, closure, device-removed sentinel and overflow.
- Native WARP/hardware interop passed actual selected red GPU pixels while the
  unrelated blue owned window covered it. Pixel verification deliberately used
  test-only D3D11 readback, not the recorder GPU path.
- Holding three output publications/resolved leases kept their pixels unchanged.
  Both original Present paths completed at least 20 more calls while the output
  pool stayed full and completed input frames were dropped. No local replacement
  textures were allocated to evade retained leases.
- Actual ResizeBuffers to 640x360 retired GPU capture and stale unresolved handles;
  both renderers continued for at least 12 further calls. A resolved GPU texture
  remained readable after fixture destruction.
- Two retained contexts exhausted the six-local-surface budget; a third creation
  was rejected. Releasing the second context restored creation within that budget.
  Fresh fixture/in-flight teardown passed without bypassing it.
- The WARP recorder-adapter regression retained three encoder leases, checked the
  previous publication became stale, observed selected/unrelated Present progress,
  and stopped cleanly. After stop, unresolved handles/session context were invalid
  while the resolved test lease still contained red pixels. Readback there is
  explicit test evidence, separate from the replay run below.

## Actual replay/decode

Pipeline: owned D3D12 swap chain -> three shared textures/two fences -> local D3D11
GPU publication -> Media Foundation -> replay buffer -> direct MP4. No pixel readback
assertions are called in the GPU replay worker. HardwareOnly prevents silently
substituting software encoding. Existing CPU/offscreen cases remain separate.

Final full gate:

- Hardware: NVIDIA H.264 Encoder MFT, 318x178, 852 ms, 16 encoder GPU-surface inputs,
  zero reported capture CPU readbacks and zero encoder CPU-readback inputs.
  Approved FFmpeg decoded every saved frame and checked 15 expected red-color
  samples (one downscaled sample per saved frame).
- WARP: H264 Encoder MFT software encoding, 318x178, 866 ms, zero capture readbacks,
  33 encoder readbacks and zero encoder GPU-surface inputs. Every saved frame
  decoded and 20 red samples passed. This route is explicitly NOT GPU-only.

MP4 validation checks source dimensions, plausible 700–2000 ms duration, leading
keyframe, at least ten nonempty samples and monotonic sample timestamps. Decode
requires the already-approved exact FFmpeg SHA-256, never an arbitrary PATH binary.
GPU-only here means no application capture/encoder CPU readback on the measured
hardware route, not zero-copy, zero allocations or verified vendor internals.

## Initial failure and correction

The first native WARP attempt failed with DXGI_ERROR_UNSUPPORTED (0x887A0004) when
using hardware-style explicit-adapter/video-support D3D11 creation. WARP now uses
the explicit WARP driver configuration without hardware video flags, followed by
the same strict adapter-LUID equality check. Hardware still requests video support.
There is no runtime switch to another adapter or silent CPU capture fallback.
Both native modes and both replay modes passed after that correction.

## Reproduction and validation

```powershell
.\scripts\test-game-capture-prototype.ps1 -Native -Hardware -Decode
```

Passed: 28 feature-enabled library tests, five safe recorder tests, one safe
dimensions test, three helper tests, existing D3D11 native suites, six D3D12 native
cases, and 24 native recorder cases (16 WARP/eight hardware), including all configured
approved decode checks. Existing deterministic timestamp/replay tests also pass.
Workspace formatting/Clippy/tests, frontend typecheck/lint/build, settings-save
browser regression and strict dependency verification passed. No new package;
existing windows 0.62.2 bindings and local bridge only.

## Not established / next milestone

No cross-process D3D12 protocol, process-death crash atomicity, hook/queue discovery,
arbitrary game, fullscreen/Alt-Tab/minimize, anti-cheat or account-safety acceptance,
HDR/other formats, multi-adapter/vendor matrix, physical GPU loss, forced fence
delay/timeout, concurrent producer callers, long-duration performance/soak tests,
signing/antivirus distribution or production rollout. All game-family statuses
remain unverified. No running game/settings were modified or installer released.

Next: controlled cross-process shared-texture/fence ownership and failure tests,
before general D3D12 helper/hook integration. No commit, push or PR was created.
