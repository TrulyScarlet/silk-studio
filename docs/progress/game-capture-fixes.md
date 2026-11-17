# Game capture implementation tracking

Current overnight continuation: ADRs 0079–0081 separate three results: controlled
hardware pending-copy removal after remote retirement; typed terminal producer
loss with caller-boundary preservation; and automatic reconstruction of a manual
owned hardware source after typed loss. The latter saves/decodes three formats
and passes twenty reconstruction epochs, retained-context budgets and stop/proof
negatives. It is not independent DLL-helper or actual-game recovery. New remote
WARP removal/reconstruction is blocked before startup after native faults. The
independent two-hour WGC snapshot is pending final confirmation, not acceptance.

Resumed continuation: local-consumer removal retirement now uses an independent
D3D11 fence and passes same-process/same-budget reconstruction with retained old
pixels on WARP/hardware. Full native gate passes 64 recorder / 23 attachment /
21 device-lifetime tests plus earlier suites. Remote-worker recovery is still
unfinished. Production engine permission-denied/invalid-config recovery now stops
after one rejected restart and preserves its error code; 26 engine tests and
final workspace/frontend gates pass. See pending-removal evidence for exact logs.

Delivery acceptance is tracked in `game-capture-delivery-plan.md`. Follow-up
resize API-failure/odd-width rejection tests pass. Owned WARP/hardware pending
queue and actual capture-copy removal probes now establish bounded **fail-stop**,
not automatic loss recovery; see ADR 0078 and pending-removal evidence. General
production/game/endurance/release gates remain open.

## Current checkpoint — 2026-11-14

Automatic **same-chain owned** resize now passes native WARP/hardware transitions
and software/hardware replay save/decode across three formats. Parent capture
resize controls are disabled on that route; geometry requests exercise the real
renderer API. Retirement-stop and stale-approval negatives pass. Combined tracked
native gate exits zero. See `../evidence/d3d12-automatic-resize-2026-11-14.md`.
In-flight loss, general production integration, real-game/fullscreen acceptance,
endurance and release remain unfinished. Older automatic-resize-pending notes
below describe the prior checkpoint, not this validated limited route.

Conditional late owned D3D12 attachment via an actual later ResizeBuffers1 now
saves/decodes replay on WARP and hardware, without guessing a queue. The target
renders before DLL loading; native checks prove zero new factory creations.
Combined tracked acceptance passes **61 recorder / fourteen attachment tests**
plus earlier owned native suites. Final safe/workspace/frontend, feature Clippy,
strict dependencies, settings-save and whitespace gates pass. Evidence:
`../evidence/d3d12-late-resize-2026-11-14.md`.

No-resize targets, ambiguous queues, unsupported node layouts, general game IPC/
loading, automatic later resize/replacement, actual game/fullscreen acceptance and
in-flight device loss remain open. Both eight-hour attempts lack confirmed final
results; the newer durable record explicitly detects interruption. No endurance
pass, installed release or general game-capture completion is claimed.

## Continuing work — 2026-11-13

Resumed work: conditional late D3D12 attachment now observes a real later
ResizeBuffers1 on an already-rendering owned chain. WARP/hardware native capture
and software/hardware MP4 save/decode pass; absent resize and distinct same-device
queue arrays fail closed without media. Tracked combined native validation passes
61 recorder tests and fourteen attachment tests plus all previous owned suites.
This is not unconditional late game discovery. See ADR 0076.
Evidence: `../evidence/d3d12-late-resize-2026-11-14.md`.

The snapshot eight-hour attempt is **interrupted/unconfirmed** on durable status
inspection: no final timestamp/exit code and no matching runner identity. It did
not establish endurance acceptance. Do not reuse the older pending launch text as
current progress or start another unattended run blindly.

**Early owned D3D12 remote helper is implemented and tested** (ADR 0075): exact
passive child, normal sibling DLL loading, separate worker-side factory/queue
discovery, actual Present forwarding warmup and recorder MP4 save/decode on WARP
and hardware. Six native attachment tests pass three repetitions, including wrong
approvals, stale lifetime, non-passive rejection and target exit. Hardware replay
uses NVIDIA H.264 with zero reported encoder readbacks. The combined tracked run
passed 57 recorder tests and all owned native suites before the sixth attachment
case was added; that case and the latest greeting check passed targeted reruns.
Final formatting/workspace/frontend, strict dependency and settings-save checks
pass. Desktop normal dependency graph still excludes all experimental hooks.
Evidence: `../evidence/d3d12-remote-helper-2026-11-13.md`.

Follow-up: the early remote route also supports explicitly selected legacy
CreateSwapChain. Eight attachment native tests now pass together and both new
remote-legacy replay/decode tests pass (hardware zero reported encoder readbacks).
The preceding 57-recorder tracked gate predates those two additional replay tests;
no unrun 59-test full-gate result is claimed.

Subsequent final tracked validation **does** pass all 59 recorder tests and ten
remote attachment tests, plus all earlier owned native suites. One-minute remote
helper soaks pass on WARP/modern and hardware/legacy with retained pixels intact.
Full result/logs are linked in the remote-helper evidence; this is a newer actual
completed run, not an inference from the earlier targeted tests.

This does **not** implement late/pre-existing-chain discovery in real games,
shipping IPC or game/fullscreen acceptance. The passive fixture explicitly delays
creation until the helper is ready. Those remaining boundaries must not be hidden
behind the new remote-loading result.

Durable test runner now records atomic running/final JSON plus separate streamed
logs and bounded process/output deadlines. Its reader detects abandoned runner
identities without rewriting evidence. Success/failure/timeout/stale-PID and
existing-evidence regressions pass; included in the development gate.
Live-runner and launch-error coverage also passes after fixing PowerShell's
automatic DateTime JSON conversion. An actual seven-test native run finalized
through the runner with exit zero. A preserved snapshot passed the sixty-second
WGC soak (3,597 frames/143 fresh checks), then a new eight-hour attempt was launched
with separate durable result/log files. Its result remains pending; see
`../evidence/tracked-test-runner-2026-11-13.md`. This is not acceptance of the earlier
interrupted run or a substitute for attachment implementation.

ADR 0072 adds owned legacy CreateSwapChain observation alongside ForHwnd. Seven
native bootstrap tests pass, including both legacy WARP/hardware lifecycles;
software/hardware legacy replay saves and approved decoding pass. Hardware uses
NVIDIA H.264 with zero reported encoder readbacks. General D3D12 remote attachment
and pre-existing chains remain unfinished. This is substantive prerequisite work,
not a declaration of full game capture completion.

Scope authorized 2026-11-11; see ADR 0044 and ADR 0045.

## Latest milestone — 2026-11-12

The eight-hour owned-window WGC soak **did not produce a completion result**.
The log stops at thirty minutes; on 2026-11-13 no test/child/Cargo processes remained.
Cause is unconfirmed. Earlier chat progress was stale and has been corrected.
The sixty-second variant passed, but no eight-hour acceptance is established.
Durable exit/result recording is needed before another unattended long run. Evidence:
`../evidence/wgc-endurance-pending-2026-11-12.md`.

ADR 0071 adds actual **quiescent API-driven device removal/recreation** tests on
WARP/hardware: observed 0x887A0005, UINT64_MAX removal fence sentinel, rejection
while the removed singleton is retained, then healthy creation after release.
Same-HWND capture and three saved/decode replay epochs pass after quiescent removal.
This is not in-flight loss/driver-reset recovery. Evidence:
`../evidence/d3d12-quiescent-removal-2026-11-12.md`.

Further production hardening defers WGC staging allocation until observed content
size, preserves the sixteen-surface budget across actual stop/start, reserves
format-event capacity under full queued epochs, and checks capture-device health.
Typed DeviceLost now uses the engine's existing bounded recovery path; its new
deterministic regression passes. Native WGC retained-surface soak observed ~3,600
frames in sixty seconds with old pixels unchanged. Software/hardware-only WGC
replay also validates prompt engine stop while minimized. A local embedded-assets
development desktop executable builds; it is not a signed/installed release.
Final current-state validation passes 53 native recorder tests (35 WARP/18
hardware), fifteen D3D12 lifetime/recreation/removal native tests, nine production-
WGC native tests, workspace/frontend checks, strict dependencies and settings-save
regression. Local development output is `target/debug/silk.exe`; nothing installed
or auto-launched. Protected-game/FSE acceptance and general remote D3D12 loading
remain open, and no general in-flight driver-reset recovery is claimed.

ADRs 0069–0070 harden the **production non-injected WGC route**: lifetime-pinned
window IDs, queued/delivered/encoder texture ownership, bounded retired pools,
ordered resize resolver changes, terminal queued-media rejection, capture-exclusion
rechecks and Idle for stoppable quiet/minimized sources. Odd RGB window sizes now
convert to even NV12 output; hardware discovery no longer mislabels software MFTs.
Six native owned-window WGC lifecycle tests passed repeated runs, and production
DisplayCapture/RecorderEngine saved/decoded software and hardware-only clips from
336x235 window content. Full gate: 51 native recorder tests (34 WARP/17 hardware),
plus native suites. Workspace/frontend, dependencies and settings-save pass.
Legacy HWND/PID-only saved selections now require refresh and explicit reselection.
Evidence: `../evidence/production-wgc-lifecycle-2026-11-12.md`.

### Earlier milestone snapshots (status at the time)

ADR 0068 now implements the owned healthy device-graph teardown/recreation route:
same PID/lifetime/HWND, release-before-create, independently observed fresh queue/
chain association, bounded handle epochs and recorder context refresh. This follows
ADR 0067 without retaining an old device or asserting pointer inequality. Native
and recorder tests cover fresh pixels, old retained leases, exact approval, faults,
same-size/mixed resize, cancellation, explicit restart and repeated epochs. Actual
device removal/driver reset, remote loading, production/game/fullscreen acceptance
remain unimplemented/unverified. Evidence:
`../evidence/d3d12-healthy-recreation-2026-11-12.md`.

Final full capture/decode gate passes 49 safe library tests, seven safe recorder
tests, eleven device-lifetime/recreation native tests, prior native suites and 49
native recorder tests (33 WARP / 16 hardware). Timed twenty-recreation runs checked
6,624 WARP / 5,515 hardware publications over ~60 seconds each. Ten saved replay
epochs per path decode correctly. Workspace/frontend, feature-isolation Clippy,
strict dependencies and settings-save checks pass. No fixture processes remain.

ADR 0067 corrects the next device-replacement plan: D3D12 is a per-adapter
singleton, so retaining the old device while comparing a supposedly new device
cannot prove replacement. New native WARP/hardware tests confirm singleton
identity, queue-retained lifetime and healthy creation after test-owned references
are released. These are precondition tests only. The experimental kind-6 packet
is validated but remains unsupported by producers; incomplete retarget/discard
helpers were removed. Healthy capture device recreation and removal recovery
remain unimplemented. Next implementation must release the entire old device
graph while retaining only HWND guards and scalar metadata, without treating
recycled COM addresses as lifetime proof.
Evidence: `../evidence/d3d12-device-lifetime-preconditions-2026-11-12.md`.

ADR 0066 now implements controlled **same-chain live resize**, distinct from
device-loss recovery. Exact retirement/drain/replacement-handle/approval epochs
protect old/new GPU ownership. A next_event barrier preserves the already-delivered
frame's resolver gap; new GPU context is installed before FormatChanged/new frames.
WARP/hardware native tests and three saved/decoded epochs (318x178 red, 640x360
green, 800x450 blue) pass, as do stale ACK/retirement rejection and six-surface
budget failure. Resolver-gap test passed three further repeats. Final full native/
hardware/decode gate: 47 safe library tests and 39 native recorder tests (26 WARP/
13 hardware), plus all native suites. Workspace/frontend, strict dependencies and
settings-save checks pass. Device replacement/loss, automatic third-party resize
interception and protected-game acceptance remain unimplemented/unverified.
Evidence: `../evidence/d3d12-live-resize-2026-11-12.md`.

ADR 0065 adds independent in-process owned capture-worker bootstrap. The renderer
directly creates/presents and shares only PID/creation-time/HWND metadata; worker
creates its own factory, claims the exact observed HWND, validates queue/device
and initializes capture. Native tests verify at least twelve renderer Presents
before capture initialization and twelve after cancellation. WARP/hardware replay,
five bootstrap native tests (including stale identity/approval), 500-frame soaks
and queued-media rejection pass. Full gate now passes 44 safe library tests and
35 native recorder tests (23 WARP/12 hardware), plus existing native suites.
Workspace/frontend, strict dependency and settings-save gates pass. Live rearm is
still unimplemented; it needs epoch/handle replacement and ordered GPU-context
transition at FormatChanged. No remote D3D12 loader or game support claimed.
See `../evidence/d3d12-worker-bootstrap-2026-11-12.md`.

ADR 0064 implements actual owned CreateSwapChainForHwnd interception as a distinct
default-off source. The renderer no longer calls the cooperative creation wrapper
on this route; successful detour observations supply queue/chain proof for native
ownership. Overflow/contention make observation terminal without changing DXGI's
successful result. WARP/hardware factory/Present capture, replay/decode, faults,
window-state transitions and 500-frame short soaks pass. Final full gate: 42 safe
library tests, six safe recorder tests, two native owner tests, 14 native hook tests,
three helper tests, other native suites and 32 native recorder tests. Workspace,
settings-save and strict dependency gates pass. Independent bootstrap/loading and
general-game support remain unimplemented; no release/commit. Evidence:
`../evidence/d3d12-factory-interception-2026-11-12.md`.

ADR 0063 now implements native ownership at cooperative owned factory creation:
canonical COM identities, DIRECT/device/window validation, eight retained slots
and retained HWND guards. Rendering/GPU capture resolves the explicitly selected
association. Native WARP and hardware tests pass, including a second same-device
DIRECT queue that cannot replace the selected queue. Full native/hardware/decode
gate passed again with 41 safe library tests plus two explicit native owner tests
and all 30 native recorder tests. Factory interception/bootstrap remains future
work. See `../evidence/d3d12-native-queue-owner-2026-11-12.md`.

ADR 0062 adds the metadata contract for future owned queue observation:
eight fixed associations, explicit token selection, conflict/capacity rejection,
generation-safe retirement and terminal selected-source loss. Eight new deterministic
tests pass (41 feature-enabled library tests total). This is preparation only:
no native factory interception or replacement of the supplied fixture queue yet.
Workspace/frontend gates, all-feature Clippy, settings-save regression and strict
dependency verification passed again. No dependencies were added.

ADRs 0057–0061 add owned-child D3D12 shared-fence transport, recorder integration,
actual resident Present detouring, window-state tests, short soaks, rational cadence
and OS capture-exclusion handling. Full native/hardware/decode and workspace gates
passed. VALORANT's selected window explicitly excludes capture: zero frames,
PermissionDenied, no monitor fallback or injection. See
`../evidence/d3d12-owned-hook-and-valorant-2026-11-12.md` for limits/results.

## Implemented first slice

- Preserve source aspect within fixed output bounds for isolated sources; native
  dimensions follow WGC content rather than the monitor.
- Snapshot source selection before deferred settings updates; browser regression
  covers repeated saves and retry after a failure without reopening settings.
- Route selected game/window targets exclusively to WGC window capture; prevent
  silent monitor fallback. Label this as window-based, not hook-based capture.
- Bind enumerated targets to PID + HWND and include non-foreground/minimized/
  cloaked application windows. Reselection after process restart may be required.

## Implemented second slice — development only

- Original x64 D3D11 test renderer and real Present function detour under the
  default-off `experimental-hooks` feature in `capture-hook-prototype`.
- Explicit self-hook command-line opt-in; no game discovery or injection.
- PID + process-creation-time + HWND identity validation, session/epoch metadata,
  bounded three-surface GPU leases, stale/weak lease protection, and bounded
  retirement across epochs. Another swap chain cannot become the capture target.
- Native tests passed on WARP and NVIDIA GeForce RTX 2080 Ti: actual pixel
  readback, target isolation, full-ring drops without interrupting Present,
  resize invalidation, disconnect, retained old frames, and detach/reattach.
- Pinned permissively licensed MinHook dependency; notices retained. The desktop
  application's dependency tree does not include the prototype or MinHook.
- Evidence: `docs/evidence/d3d11-hook-prototype-2026-11-11.md`.

## Implemented third slice — cooperative cross-process fixture

- Fixed-size 128-byte wire format with explicit version, message type, bounded
  fields, and zero reserved bytes. Strict ACK/session/epoch ownership ledger.
- Separate consumer launches the controlled renderer over private standard pipes,
  verifies PID/creation time/HWND/QPC frequency/adapter LUID, duplicates unnamed NT
  texture handles, and validates actual shared GPU resource descriptors.
- Three shared export slots with keyed GPU ownership. Full slots drop export
  work; matching ACKs permit reuse only after consumer GPU ownership is released.
- Native tests passed on WARP and RTX 2080 Ti: cross-process pixels, full-ring
  backpressure, retained frames, exact ACK reuse, malformed identity/descriptor
  rejection, stale and premature ACKs, real keyed-mutex timeout rejection,
  graceful shutdown/crash, and EOF/truncated/malformed/stalled pipe cleanup.
- Ten deterministic prototype tests now cover the lease/session/wire contracts.
- Export and pipe I/O remain outside the Present callback. This is still a
  cooperative test harness, not an injected helper or production threading model.
- Design/evidence: ADR 0047 and
  `docs/evidence/cross-process-hook-transport-2026-11-11.md`.

## Implemented fourth slice — recorder-interface validation

- `ControlledCapture` implements the existing VideoCapture contract for only
  `fixture:d3d11`; it has a dedicated capture worker and a bounded three-event
  queue. No UI or production source routing imports this adapter.
- Shared peer lifecycle code is reused by the transport fixture and adapter.
  Startup waits for handshake validation; stop/failure clean up the exact owned
  helper and reader; invalid display/game IDs never fall back to a monitor.
- Checked QPC-to-millisecond conversion has deterministic drift/backward/overflow
  tests. Source dimensions stay 640x480 (4:3), not the monitor dimensions.
- Actual replay saves passed through hook -> cross-process shared GPU surface ->
  immutable CPU BGRA worker bridge -> Media Foundation -> replay buffer -> MP4.
  This is explicitly a CPU-readback development bridge, not zero-copy production.
- WARP/software-H.264 and RTX 2080 Ti/hardware-H.264 saves were validated by the
  MP4 reader and actually decoded with the hash-verified approved FFmpeg binary.
- Native tests cover queue overflow, repeated start/stop, terminal producer loss,
  and real replay saves; safe startup-failure/invalid-source tests need no desktop.
- Design/evidence: ADR 0048 and
  `docs/evidence/controlled-hook-replay-2026-11-11.md`.

## Implemented fifth slice — GPU-frame recorder bridge

- Explicit GPU bridge copies imported frames into three protected local BGRA8
  textures, releases/ACKs the cross-process slot, and supplies GPU handles through
  the existing GpuFrameContext contract. No capture-worker CPU readback in this mode.
- Publication tokens cover queued/currently delivered frames; encoder Arc leases
  and weak references prevent premature reuse. Retired resolvers reject handles
  while existing leases remain readable. Per-capture restart budgeting permits
  at most six local surfaces, including retained old contexts/frames.
- Native tests passed for retained pixels under pressure, stale handles, pool-full
  drops, stop/restart, quota rejection, and quota recovery after releasing contexts.
- Actual RTX 2080 Ti/NVIDIA H.264 Encoder MFT replay passed with GPU-surface inputs
  and zero encoder readbacks; the saved MP4 decoded using approved FFmpeg.
- WARP GPU delivery plus software H.264 encoding also passed, with explicit
  encoder CPU-readback counters. This compatibility route is not GPU-only.
- Added additive/backward-compatible encoder input-path metrics, including failed
  fallback attempts so earlier readbacks cannot be hidden by backend changes.
- Design/evidence: ADR 0049 and `docs/evidence/gpu-hook-replay-2026-11-11.md`.

## Implemented sixth slice — independent renderer lifecycle safety

- Added a portable admission/in-flight gate and an explicitly opt-in fixture
  probe feature. Capture cancellation does not disable/remove the Present hook.
- Two independent renderer threads exercise the same patched Present entry,
  full capture queues, unrelated rendering during an in-flight callback hold,
  cancellation, resize invalidation/new epochs, simulated loss notification,
  own-device recreation, retained pixels, and cooperatively quiescent detach.
- Lifecycle runs passed on WARP and the local RTX 2080 Ti; included in the native
  prototype test script. Both probe-enabled and probe-disabled lint gates pass.
- See ADR 0050 and `docs/evidence/independent-hook-lifecycle-2026-11-11.md`.
- This is not arbitrary-game shutdown or concurrent selected-chain support;
  attachment and removal still rely on fixture-owned cooperative quiescence.

## Implemented seventh slice — cooperative DLL helper

- Added default-off `crates/capture-hook-helper` and a versioned packet/C ABI.
  The owned fixture loads its canonical sibling DLL only after parent approval
  through private child pipes; no remote injection or arbitrary target selection.
- Controller and DLL independently verify selected PID/creation time/HWND. The
  DLL enforces render-owner affinity, increasing epochs, and at most three
  successful attachments per process to bound retained shared-ring allocation.
- Native WARP/RTX 2080 Ti tests cover independent cross-process pixels, full-ring
  progress, Stop with continued rendering, resize/device rearm, foreign-thread
  rejection, stale-epoch rejection, input-pipe EOF cleanup, and target exit.
- DLL remains loaded until process exit. This is not arbitrary-game unload safety.
- WARP crash testing exposed incomplete pixels when process death interrupts
  queued GPU work. Only frames completed before the crash have retention evidence;
  a future production path must discard dead-source pending frames.
- Design/evidence: ADR 0051 and `docs/evidence/cooperative-helper-2026-11-11.md`.

## Implemented eighth slice — DLL helper to replay recorder

- ControlledCapture now has an explicit cooperative-DLL mode, using the existing
  VideoCapture/GPU bridge/recorder/MF/replay/MP4 interfaces. Frame delivery is not
  driven by recorder Render requests; skipped input frames are still released/ACKed.
- Added one-entry rearm control, delivery-epoch rejection, GPU context replacement
  on FormatChanged, and retained-child-handle liveness checks. Observed target
  death rejects pending events and invalidates the resolver, without desktop fallback.
- Actual DLL-route clips saved/decoded on WARP/software and RTX 2080 Ti/hardware.
  Native replay tests cover 640x480 red, 800x600 green resize, and 960x540 blue
  device recreation, with replay-epoch changes and every decoded frame color checked.
- The hardware DLL route reported GPU-surface inputs and zero capture/encoder CPU
  readbacks. WARP explicitly reports encoder readbacks; it is not GPU-only.
- Bounded queue/restart and dead-source pending-media rejection tests passed;
  target exit through RecorderEngine becomes terminal Error rather than reattachment.
- Design/evidence: ADR 0052 and `docs/evidence/dll-helper-replay-2026-11-11.md`.
  Interrupted GPU work at process death is still not crash-atomic pixel publication.

## Implemented ninth slice — owned-process resident attachment (2026-11-12)

- Added separate default-off resident-hooks/resident-helper features. A controller
  normally loads the DLL only into the retained passive fixture child it launched,
  validating native AMD64, executable path, PID/creation time/HWND, and module offsets.
  No arbitrary PID/game attachment is exposed; no security bypass or elevation.
- The capture-blind renderer does not load the helper, pass a swap-chain pointer,
  handle capture commands, or pause for capture. The worker discovers selected-HWND
  Present calls, initializes textures outside Present, and captures without TLS
  render-owner state. MinHook installation still uses its own thread management.
- Cancellation leaves the trampoline/forwarder and pinned modules resident. Native
  WARP/hardware tests passed with full rings, unrelated-chain isolation and continued
  original Presents after cancellation and worker exit, without cooperative pause.
- ResidentDll recorder replay also passed on both drivers and decoded with approved
  FFmpeg. The RTX 2080 Ti route reported 14 GPU-surface inputs and zero encoder or
  capture readbacks in the representative final run. WARP reports encoder readbacks.
- One remote attempt per owned child bounds uncertain-load allocations. Documented
  module-snapshot startup races use bounded ERROR_BAD_LENGTH-only retries.
- Design/evidence: ADR 0053 and `docs/evidence/resident-attachment-2026-11-12.md`.
  Real-game targeting/IPC and game compatibility are NOT established by this stage.

## Implemented tenth slice — owned offscreen D3D12 readback (2026-11-12)

- Added standalone default-off `d3d12-fixture` (no MinHook/injection dependency)
  with one private DIRECT queue, three fixed BGRA8 targets/readback buffers,
  per-slot command allocators/lists and a monotonically signaled fence.
- Resource states belong solely to this renderer. Submission never waits for
  fence completion; polling maps only completed buffers. Full slots drop capture
  admission, including when completed CPU frames remain leased to consumers.
- Deterministic ownership tests cover completion versus release, stale release,
  order, closure and fence overflow/device-removed sentinel rejection. Native
  WARP/hardware tests cover red/green/blue/white pixels, padded rows, full rings,
  retained frames, cancellation, in-flight teardown and sequential size/device
  recreation. This is NOT live swap-chain resize or a Present hook.
- A test-only recorder adapter saves/decodes actual D3D12-origin frames through
  CPU readback and software H.264. Final clips were 846 ms WARP / 842 ms hardware,
  318x178, with 25 decoded color samples checked per clip and 39 capture readbacks
  reported per run. No D3D12 GPU-only path is claimed.
- ADR 0054 and `docs/evidence/d3d12-readback-2026-11-12.md` record design, failures,
  validation and remaining work. No production game path was changed.

## Implemented eleventh slice — owned D3D12 swap-chain capture (2026-11-12)

- Added owner-thread-only `D3d12PresentFixture` with two real windowed flip-discard
  swap chains on the capture device's explicit DIRECT queue. Render selected red
  and unrelated blue; copy only the selected current backbuffer before Present,
  restoring PRESENT state. Capture no longer relies solely on offscreen targets.
- Separate fixed per-backbuffer renderer allocators/fences let both Present paths
  progress when all three capture leases are held and after capture cancellation.
  No per-submission CPU fence wait, channel blocking or UI/disk/encoding work.
- Native WARP/hardware checks include selected HWND covered by the unrelated owned
  blue HWND, exact retained red pixels, full-ring drops, cancellation, real
  ResizeBuffers, fresh instance/in-flight teardown and zero remaining fixture HWNDs.
  Resize invalidates capture permanently; render continues, no implicit rearm.
- Added a dedicated owned-window/render worker and bounded three-frame CPU adapter
  for RecorderEngine. Swap-chain replay saves and approved-FFmpeg decode passed on
  WARP/hardware. Still software H.264 and explicit CPU readback, not GPU-only.
- Design/evidence: ADR 0055 and `docs/evidence/d3d12-presentation-2026-11-12.md`.
  No D3D12 detour, protected game, fullscreen or production source route was added.

## Implemented twelfth slice — owned D3D12/D3D11 GPU interop (2026-11-12)

- Added standalone default-off `d3d12-gpu-fixture`, three shared BGRA8 textures and
  two shared fences. Same-adapter D3D11 imports use exact LUID validation and
  protected immediate context; the route does not Map/read back capture pixels.
- Producer slots cannot be recycled until consumer-copy fence completion. Local
  encoder output has its own three-surface publication pool, retained lease/stale
  handle checks and six-local-surface restart budget. Full pools drop work without
  stopping either owned Present path.
- Native interop tests passed on WARP/hardware for covered selected-source pixels,
  retained publications/resolved leases, full output pool, resize retirement,
  stale handles, bounded restart and in-flight teardown. Test-only pixel readbacks
  are separate from measured recorder runs.
- Actual D3D12 swap-chain -> shared-fence GPU interop -> NVIDIA H.264 Encoder MFT
  -> replay MP4 passed with 16 encoder GPU inputs, zero reported capture/encoder
  CPU readbacks and approved decode in the final hardware run (852 ms, 318x178).
  WARP interop also decoded but software encoding reported 33 encoder readbacks.
- Added deterministic dual-fence ledger tests and adapter retained-lease regression.
  All native/prototype, workspace/frontend, settings-save and dependency gates pass.
  ADR 0056 and `docs/evidence/d3d12-gpu-interop-2026-11-12.md` record evidence/limits.
  This is in-process interop, NOT cross-process D3D12 IPC, hooks or game support.

## Remaining work — not implemented

The five-title research and staged acceptance plan is in
[`fps-capture-compatibility.md`](fps-capture-compatibility.md). All protected-game
runtime results remain unverified; WGC and hook evidence must stay separate.

- A game-attached x64 D3D11 capture backend and production capture-interface
  integration. The current hook operates only in the controlled test renderer.
- Production target approval and helper startup/shutdown beyond the owned-child
  private-pipe fixture protocols. The externally loaded route still deliberately
  accepts only passive owned children, not arbitrary games or production UI selection.
- Production game-source integration, bounded cross-process ring retirement on
  resize/device loss, and synchronization with independently rendering games.
  Fixed-ring CPU and GPU recorder adapters are verified only with the fixture.
- Signing/packaging review for any future helper/DLL. MinHook is reviewed for the
  development prototype only; no OBS implementation material is permitted.
- Native acceptance tests: selected game obscured by another app, Alt-Tab,
  minimize, game restart, different aspect ratios, and changing displays.
- Exclusive-fullscreen, multi-GPU, driver, antivirus, and anti-cheat compatibility
  evidence, including Valorant. No anti-cheat bypass or account-safety guarantees.
- A real-game test requires an explicitly selected non-protected D3D11 target;
  no installed/running game was selected or attached during this work.
- D3D12 Present detouring/queue discovery, cross-process shared-fence transport and
  production integration remain unimplemented. Owned windowed swap-chain capture
  and in-process D3D12/D3D11 GPU interop now pass, but do not establish general-game
  attachment or fullscreen compatibility.
  Vulkan/OpenGL support is also unimplemented.

The source-selection and aspect-ratio work is not a substitute for the hook
milestone. Do not publish it as verified fullscreen game capture.

## Validation of the first slice

- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test --workspace`: passed; hardware/interactive tests remained ignored.
- `npm run typecheck`, `npm run lint`, `npm run build` in `apps/desktop`: passed.
- `node scripts/test-settings-save-gate.mjs`: passed using isolated Edge storage
  and mocked IPC with two monitors and two process-bound application windows.
- `git diff --check`: passed (repository line-ending normalization warnings only).

No live game, Alt-Tab, exclusive-fullscreen, or antivirus acceptance test was run.
No new installer was published.
