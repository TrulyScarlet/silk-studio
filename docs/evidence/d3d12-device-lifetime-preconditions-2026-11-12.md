# D3D12 device-lifetime preconditions

Date: 2026-11-12. ADR 0067. This is NOT capture device-replacement acceptance.

The proposed strategy of retaining one old device for a canonical COM comparison
was rejected against Microsoft's D3D12CreateDevice contract: devices are process-
local singletons per adapter. Holding an old device (including through a child
queue) makes a subsequent same-adapter creation return that device. The incomplete
factory-offer discard and Present-retarget helpers were removed instead of hiding
their unused-code warnings. Existing bootstrap and same-chain resize stay intact.

## Tests actually run

`d3d12_device_replace_native.rs`, with `d3d12-device-replace-fixture`, passed on
WARP and hardware adapter 0 (NVIDIA GeForce RTX 2080 Ti). Each test:

1. Creates two devices on the same adapter and confirms canonical IUnknown equality.
2. Creates an unused DIRECT queue, drops both explicit devices and their canonical
   interfaces, then obtains the device from the retained queue. A new creation is
   canonically equal to that queue-retained device.
3. Releases the queue and every test-owned device/interface, calls creation again,
   checks healthy GetDeviceRemovedReason and the same adapter LUID. No GPU commands
   are submitted in this test. No address-inequality assertion is made.

These observations do not independently prove runtime-internal device destruction
or replacement. They establish why a retained-device identity check is invalid.

The reserved SK12 kind-6 ReplaceDevice packet roundtrips and rejects truncation,
reserved bytes, invalid dimensions and zero session identity fields. It differs
from Resize kind 3. No producer operation/source/CLI mode accepts replacement.

## Validation

- `cargo clippy -p capture-hook-prototype --all-features --all-targets -- -D warnings`: passed.
- Bootstrap-only Clippy: passed.
- All-feature library suite: 48 passed, two native owner tests ignored here and
  exercised separately by the complete gate.
- `scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode`: passed,
  including both new singleton tests, existing native suites and all 39 native
  recorder tests (26 WARP, 13 hardware), with approved FFmpeg decoding.
- `scripts/check.ps1`: passed formatting, workspace Clippy/tests and frontend
  typecheck/lint/build.
- Strict dependency verification and standalone settings-save regression: passed.
- `git diff --check`: passed with existing CRLF conversion warnings only.
- No owned D3D11/D3D12 fixture processes remained. About 2.58 GB disk space free.

No dependencies added. No protected-game capture, injection, driver reset or actual
device-removal test performed. All changes remain uncommitted; no push/PR/release.

## Next implementation boundary

Preserve owner-thread HWND guards but release the complete old producer device
graph before creation. Reuse the parent retirement/delivery barrier and bounded
epoch contract. Validate recreated queue/device/chain associations and fresh pixels
without using recycled COM addresses as device-lifetime proof. Any uncertain drain
must remain terminal. Healthy controlled teardown/recreation and actual removal
recovery must be reported separately.
