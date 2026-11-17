# ADR 0077: Observed automatic resize recovery for owned late capture

- Date: 2026-11-14
- Status: Same-chain owned recovery implemented and validated; broader recovery incomplete

## Scope and plan

Extend the exact owned late ResizeBuffers1 route, not arbitrary game injection.
The capture worker must observe the application's resize rather than receive a
parent capture-resize command. Fixture-only geometry messages may request that
the owned renderer exercise its normal API path; they carry no COM pointers.

1. Close media admission before the selected application's actual ResizeBuffers1.
   Drain/release captured back-buffer references using the existing bounded GPU
   release path before forwarding the original API. Never wait for parent/UI/disk
   work in that callback. Preserve its arguments/HRESULT; failed or uncertain
   teardown closes capture rather than claiming recovery.
2. Validate the successful resulting queue/buffer observation and publish one
   bounded offer. Restrict the first implementation to the same retained chain,
   device and single DIRECT queue. Overlap, ambiguity, unsupported dimensions or
   failed API result are terminal. No silent retarget/reinstall/unload.
3. The worker asynchronously announces observed dimensions, crosses the existing
   exact parent retirement/delivered-frame boundary, builds fresh media, and waits
   for exact new-epoch approval. Rendering keeps forwarding while capture pauses.
4. Admission requires both the Present gate and a stable automatic-resize phase;
   cancellation/failure must not race a rearm into publishing stale-sized frames.
5. Add deterministic phase/epoch tests, native retained-pixel/identity/stop cases,
   and saved/decode replay across observed epochs on WARP and hardware. Then run
   formatting, feature Clippy, workspace/frontend and combined native gates.

This is same-chain resize recovery. Device graph replacement, unexpected in-flight
loss/TDR, general IPC/loading, multi-node/rotating queues, real games/fullscreen,
packaging and long endurance acceptance remain separate. Existing RemoveDevice
probes only establish quiescent removal. No new dependencies or licenses planned.

## Implemented evidence

Feature-gated `--auto-resize-owned` requires the exact passive late route. A
selected-HWND WM_APP fixture message carries bounded scalar geometry through one
atomic mailbox; the renderer invokes its real ResizeBuffers1. The parent adapter
accepts producer-originated Resize only in automatic mode and crosses its existing
delivery/retirement barrier. Parent capture resize commands are unavailable.

WARP and hardware native tests pass three observed resize epochs, retained red
pixels, fresh green pixels/dimensions and stop without killing the renderer. WARP
negative cases pass stop during retirement and stale approval rejection. Hardware
and software recorder tests each save/decode three clips across 318x178, 640x360
and 800x450. The tracked combined native gate passes with exit zero; see
`../evidence/d3d12-automatic-resize-2026-11-14.md`.

Subsequent targeted WARP tests pass stop during fresh-Hello approval and a second
actual resize during held retirement (terminal failure without new media).
Later targeted WARP cases pass actual API rejection and a successful unsupported
odd-width resize, each terminal without new media and with renderer survival.
Concurrent API stress and abrupt in-flight removal recovery still need additional
coverage. These results do not establish general game/fullscreen support.
