# Native owned D3D12 queue association — cooperative creation only

Date: 2026-11-12. ADR 0063. No game attachment or production route changes.

`d3d12_queue_owner.rs` performs successful owned CreateSwapChainForHwnd calls,
validates DIRECT queue type, canonical device identity from queue/chain and actual
HWND/current-process liveness. Eight fixed entries retain queue/device/canonical
IUnknown references and Rc<Window> guards. Resolution accepts only the selected
canonical chain; retirement closes metadata before releasing native references.
The guard keeps owned windows alive while their associations exist.

`d3d12_present_fixture.rs` records both creations, explicitly selects one and
resolves rendering/GPU capture queue/device from that association. Cancellation
retires selection, while renderer/bridge retain their own GPU lifetime contracts.

## Tests actually run

Native tests on WARP and NVIDIA GeForce RTX 2080 Ti pass: invalid HWND, COMPUTE
queue rejection, failed native creation with no entry, explicit selection, COM
interface alias normalization, rejection of unrelated chains using a second
same-device DIRECT queue, eight-slot bounds, stale-token retirement, retained
HWND lifetime and terminal selection retirement. No GPU submission in these tests.

Final `scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode` passed:
41 safe feature-enabled library tests, both explicit native owner tests, six safe
recorder tests, three helper tests, all existing native suites and 30 native
recorder tests (20 WARP/10 hardware). Includes real ResizeBuffers, cancellation,
retained outputs, window states, producer failure and two 500-frame short hook
soaks. Initial full run exposed readback-only dead-code warnings; resolution was
also integrated into that renderer and the final gate passed without warnings.
Feature-only D3D12 and all-feature/all-target Clippy pass with warnings denied.

Final replay observations:
- Hooked hardware: 837 ms, 20 NVIDIA H.264 GPU inputs, zero reported capture or
  encoder CPU readbacks, 19 decoded red samples.
- Hooked WARP: 837 ms, zero capture readbacks, 38 software-encoder readbacks,
  25 decoded red samples.
- Manual-boundary hardware: 833 ms, 13 GPU inputs, zero reported readbacks,
  12 decoded red samples.

Full workspace check passed (fmt/Clippy/tests/frontend typecheck/lint/build).
Strict dependency verification passed. Settings-save regression timed out once
waiting for initial page readiness during parallel validation; standalone retry
passed. Cause not established; no regression-test timeout was weakened.

## Limits

This is a cooperative factory wrapper, not native factory interception, generic
queue discovery or independent bootstrap. No pre-existing arbitrary chain/queue
pairs are accepted. Native tests do not validate multi-node adapters, all DXGI
creation variants, real games, exclusive fullscreen or anti-cheat compatibility.
Readback counters do not prove zero-copy/vendor-internal behavior; short soaks do
not establish performance/endurance. No dependency was added (existing windows
0.62.2, MIT OR Apache-2.0). Changes remain uncommitted; no release produced.
