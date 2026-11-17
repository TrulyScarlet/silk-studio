# Conditional late D3D12 attachment evidence

ADR 0076. Original implementation, exact opted-in owned fixtures only.

## Actually implemented and tested

The target creates its HWNDs/chains and completes at least twelve actual selected
Presents **before** publishing identity to the parent. The parent then normally
loads the helper DLL. The helper independently creates invisible STATIC-class
probe windows/chains to obtain ResizeBuffers1's method entry; it drops that probe
graph before discovery. The probe never supplies the target queue or media.

A later real ResizeBuffers1 on the same target chain supplies explicit per-buffer
DIRECT queues. The observer forwards the API first and preserves its HRESULT. It
validates the exact pinned window/process lifetime, bounded buffer count, matching
canonical device and identical canonical queue for every buffer, with a supported
single-node layout. Missing or ambiguous evidence never falls back to a guessed
queue, renderer COM handoff or desktop capture. Discovery has a five-second bound.

Native WARP and hardware cases verify pre-load rendering, **zero new intercepted
factory creations**, twenty selected red GPU publications, monotonic QPC, retained
pixels through helper stop/process exit, and continued target execution after
helper stop. This proves discovery did not secretly rely on another chain creation.

Negative cases exercise:
- No subsequent ResizeBuffers1: terminal worker exit status one, no GPU Hello or
  media admitted, target remains alive.
- Actual successful ResizeBuffers1 with distinct same-device DIRECT queues:
  terminal discovery failure, no media, target remains alive. This negative fixture
  pauses rendering rather than submitting through an unimplemented rotating-queue
  application graph.
- Four deterministic policy tests cover identical queues, order-independent
  ambiguity, zero/preserved or excessive counts, missing/mismatched arrays, wrong
  devices, non-DIRECT/null queues and unsupported node layouts.

The first WARP replay attempt caught a test-fixture assumption: twelve calls to
step did not always mean twelve completed Presents. Startup now waits for twelve
actual selected Presents with a three-second deadline before publishing identity.
The failed attempt is not acceptance evidence; both final replay paths pass.

## Replay and combined validation

Final late-path MP4s were saved through RecorderEngine and decoded by the exact
approved FFmpeg build:
- Hardware: **841 ms**, fourteen decoded primary-color samples, NVIDIA H.264 MFT,
  fifteen GPU inputs and **zero reported encoder readbacks**.
- WARP/software: **833 ms**, twenty-five decoded samples, H264 Encoder MFT,
  thirty-eight explicit encoder readbacks.
- Both report zero capture readbacks; native pixel reads are test-only.

Tracked all-feature native validation completed with actual exit **zero**, status
**passed**, in about seventeen minutes: **61 recorder tests, fourteen attachment
tests**, two native queue-owner, seven bootstrap, four cross-process, fifteen
device lifetime/recreation/removal, two GPU interop, sixteen hook, four basic D3D12,
four resize and nine production-WGC tests. This includes the prior early/legacy
remote routes, minute soaks, healthy recreation and quiescent removal regressions.

Record and separate streamed logs:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-late-resize-native-5ba8717dd93b4d78bf1d3a619cd50f16`.

All-feature prototype/helper Clippy passed before the combined run. Final safe
unit, workspace/frontend and dependency checks are separate from native acceptance.
They subsequently passed: fifty-six safe prototype library tests, seven safe
recorder integration regressions, four helper tests, workspace formatting/Clippy/
tests, frontend typecheck/lint/build, strict dependency verification and the
settings-save gate. All-feature and remote-only/bootstrap-only/late-helper-only
feature Clippy passed. Tracked-runner regressions and new-file whitespace checks
also passed. Combined final workspace log:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-late-final-checks-5eaff0396e5143eb9ae87b2bd5618063.log`.

The normal desktop graph was rechecked to exclude prototype/helper/MinHook. After
tests, `cargo build -p silk --features tauri/custom-protocol` rebuilt the local
embedded-assets development executable: `target/debug/silk.exe`, 25,097,728 bytes,
SHA-256 `8DA4E6CD58BD626DEC2749F097BFA2C7DEC1B873043488EE0F180F7FA5BFCC32`.
Authenticode reports NotSigned. It was not auto-launched or installed and does not
include the experimental hook backend. Earlier executable hashes are historical
build evidence, not a claim about this current output.

## Limits and operational evidence

This is **conditional late discovery after a future successful explicit queue-
bearing ResizeBuffers1 call**, not unconditional discovery of any static running
game. BufferCount zero, distinct queues, multi-node layouts, late legacy mode,
automatic later resize/replacement, arbitrary-process IPC/loading, game/fullscreen
acceptance and in-flight loss recovery remain unsupported/unverified. The owned
fixture retains its target HWND guard; no general HWND-generation credential is
claimed. No actual third-party/protected process was targeted, no capture exclusion
was changed and no security/driver setting was modified.

Disk exhaustion interrupted an initial compilation, not the final native gate.
Cargo dry-run identified 3.8 GiB of generated Windows-binding artifacts plus the
targeted prototype cache. Only those generated package artifacts were cleaned and
required configurations rebuilt; source, clips, desktop executable, result records
and endurance snapshot were preserved. Disk/save guards were not weakened.

The later eight-hour snapshot attempt's durable reader now reports
`interrupted_unconfirmed`: no final timestamp/exit code and no matching runner
identity. Its cause remains unconfirmed. Neither overnight attempt established an
eight-hour pass; no further unattended run was launched.

No release/install/signing, commit/push or PR. Experimental code remains outside
the normal desktop dependency graph; this is not a shipping game-capture claim.
