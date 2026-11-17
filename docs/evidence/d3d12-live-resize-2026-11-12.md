# Owned same-chain live D3D12 resize epochs

Date: 2026-11-12. ADR 0066. Default-off `d3d12-rearm-fixture` only.

## Complete implemented route

Source `owned-d3d12-live-resize` launches the exact owned bootstrap child with
explicit `--rearm`. CrossControl exposes even bounded dimensions through a one-item
request mailbox: one in-flight transition plus at most one queued request. Three
new SK12 fixed-size messages carry exact-session Resize/Retire/Retired metadata.
`resize_epoch.rs` rejects unsolicited/overlapping/wrong-format/wrong-identity
replacement offers and checked epoch overflow. Native handle values may be reused
only under a new approved epoch; they are not lifetime identity by themselves.

Worker cancels capture admission before Retire. Parent advances delivery epoch,
rejects old queued events, and waits for next_event to cross its already-delivered
frame/resolver boundary. Only then retire old context, drain actual parent-owned
consumer GPU completion, and send exact retirement proof. Resolved encoder leases
can remain alive. Worker releases drained old source/list/media references before
owner-thread real ResizeBuffers. Fresh bounded media is staged on the same resident
Present hook and new handles offered. Parent imports, enqueues new GPU context with
FormatChanged ahead of new frames, and approves the exact next session. Only then
worker resumes admission. No unhook/reinstall/unload and no monitor fallback.

Outstanding ACKs during retirement must match old published slot/sequence snapshots
exactly; they never release a new ring. After rearm, old-session ACKs are terminal.
Adapter/QPC/target/generation remain fixed; actual dimensions must match the request.
Source QPC origin/cadence continues across epochs. Six-local-surface budgeting is
shared across replacements; retaining old contexts may deny further resize.

## Tests actually run

WARP and NVIDIA GeForce RTX 2080 Ti native lifecycle tests passed real same-process,
same-chain ResizeBuffers: 318×178 -> 640×360 -> 800×450, changing red/green/blue
pixels. Old red local pixels remain correct after first replacement. Old context
handles retire; fresh frames carry the new session/dimensions. No new frames are
published before exact replacement approval. WARP negative tests reject old ACK
after new approval and incorrect retirement epoch before resize. Invalid dimensions
are rejected before sending.

Recorder tests passed:
- Delivered frame still resolves after epoch rejection begins, until next_event
  crosses the retirement barrier. Old queued events are actually discarded.
  Retained old pixels stay correct; new context resolves green/blue frames and
  timestamps continue forward. The barrier test passed three additional repeats.
- Retaining two old/current contexts makes the next replacement fail explicitly
  with the six-surface budget error; no new unbounded pool or monitor fallback.
- RecorderEngine saves and approved FFmpeg decodes every video frame in all three
  format/replay epochs, with correct dimensions and expected primary colors.

Final full-gate saved clip observations:

| Driver/encoder | Dimensions | Duration | Decoded color samples |
| --- | --- | ---: | ---: |
| WARP/software | 318×178 | 840 ms | 25 red |
| WARP/software | 640×360 | 863 ms | 26 green |
| WARP/software | 800×450 | 861 ms | 22 blue |
| Hardware-only NVIDIA H.264 | 318×178 | 834 ms | 13 red |
| Hardware-only NVIDIA H.264 | 640×360 | 834 ms | 23 green |
| Hardware-only NVIDIA H.264 | 800×450 | 841 ms | 24 blue |

Capture readback counters remain zero for these GPU routes; hardware encoder
readback counter is zero. WARP uses the explicit software encoder route, not a
GPU-only encoding claim. Pixel checks are test-only reads; zero-copy or vendor
internal behavior is not established.

Final `scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode` passed:
47 safe feature-enabled library tests (two native owner tests run separately), six
safe recorder tests, three helper tests, two native owner tests, 14 hook tests,
five bootstrap native tests, four live-resize native tests, other native suites
and 39 native recorder tests (26 WARP/13 hardware). Deterministic tests cover packet
roundtrip/truncation/reserved bytes/bounds/epoch overflow, exact replacement and
ACK session/slot/sequence/single-use semantics.

Workspace check passed fmt/Clippy/tests/frontend typecheck/lint/build. Strict
dependency verification, standalone settings-save regression, all-feature Clippy
and bootstrap-only Clippy passed. No dependencies added. No owned fixture processes
remained. About 1.0 GB disk space remained; existing disk-space guards were retained.

During test development, two harness issues were corrected: the pixel helper had
treated GpuFrameLease itself as the downcast resource, and the decoder's primary-
color check rejected the initial magenta test pattern. The helper now uses the
lease API; the third fixture epoch is blue. Final gates pass; those failed runs
are not presented as successful validation.

## Limits and next milestone

Only an explicit owned renderer command initiates resize. This does not intercept
an arbitrary application's ResizeBuffers/ResizeBuffers1 or recover a removed or
replaced device/queue/chain. Uncertain drain/device removal stays terminal; no
future fence is assumed complete. Device-loss recovery requires its own contract
and hardware/fault evidence, not relabeling this resize result.

No real game/exclusive-fullscreen/FSO/independent-flip/anti-cheat/account-safety
acceptance follows. No protected game was hooked and VALORANT exclusion remains
respected. All work remains uncommitted; no installer/release/push/PR produced.
