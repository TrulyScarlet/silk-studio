# ADR 0055: Owned D3D12 swap-chain capture boundary

- Date: 2026-11-12
- Status: Accepted for default-off owned-window testing only

Extend ADR 0054 with actual HWND flip-discard swap chains, not an arbitrary-game
hook. Create two owned windows/chains on the capture device's known DIRECT queue.
Only chain zero is the selected source. Use GetCurrentBackBufferIndex, render
PRESENT -> RENDER_TARGET -> PRESENT, then selected capture submits
PRESENT -> COPY_SOURCE -> PRESENT on that same queue before calling Present.
Capture code does not clear/repaint the source. Chain one remains unrelated blue.

Renderer allocators/lists are fixed per backbuffer and reused only after a separate
render fence completes. A busy renderer skips a submission without a CPU fence
wait. Capture saturation/cancellation cannot prevent either original Present path;
capture drops while renderer progress continues. OS/driver Present may still block.
Signal the renderer fence after capture submission and Present so resize/teardown
retires the whole queue iteration, not just the preceding clear command list.

Keep source COM leases with capture slots until safely drained. Window/control
operations are owner-thread-only; the fixture is !Send/!Sync. Replay creates it in
a dedicated window/render worker with a bounded three-frame channel, not in UI.
Capture and render timestamps remain paired with each admitted sequence.
The CPU frame carries its actual copy-submission Instant; the replay worker uses
the deterministic checked timeline conversion already covered by unit tests.

Explicit resize cancels capture, drains both queues' work (one shared queue, two
fences), releases all backbuffer leases/lists, calls ResizeBuffers and rebuilds
render slots. Capture remains terminal after resize; no silent format rearm or
monitor fallback. Fresh capture needs a fresh fixture. CPU frames already delivered
remain valid. On uncertain completion retain resources/window objects and prohibit
new core fixtures until process exit instead of reclaiming under GPU work.

No hook, queue discovery, fullscreen transition, GPU-only/shared-fence transport,
cross-process helper, production UI route, game attachment or anti-cheat claim.
No new dependency; existing windows 0.62.2 bindings (MIT OR Apache-2.0).

References:
- [Microsoft: D3D12 swap chains, queues, buffer lifetime and fullscreen semantics](https://learn.microsoft.com/en-us/windows/win32/direct3d12/swap-chains)
- [Microsoft: ResizeBuffers reference release requirements](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgiswapchain-resizebuffers)
