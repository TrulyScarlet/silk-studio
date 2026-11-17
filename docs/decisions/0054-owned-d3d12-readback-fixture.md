# ADR 0054: Owned D3D12 fence/readback fixture

- Date: 2026-11-12
- Status: Accepted for default-off owned-renderer testing only

The existing encoder GPU bridge accepts D3D11 surfaces, not D3D12 resources. Start
with an explicitly CPU-readback D3D12 fixture instead of asserting keyed-mutex
interop, queue discovery or game compatibility without implementation/evidence.

Use a private DIRECT queue and three fixed slots. Each owns a BGRA8 render target,
RTV heap, command allocator/list and READBACK buffer. The fixture alone owns the
resource states: RENDER_TARGET -> COPY_SOURCE -> RENDER_TARGET on the same queue.
Query GetCopyableFootprints; remove row padding only after fence completion.
Poll completion, never wait for a fence in submit or map before completion. A full
ring drops admission. Do not reset allocators or reuse CPU publication slots until
GPU completion AND consumer release. Immutable Arc publications have weak-lease
tracking so a retained consumer cannot trigger unbounded buffer replacement.

Fence values are monotonic/nonzero and never wrap; UINT64_MAX completion means
device removal, not success. Recording/signaling errors close admission. Teardown
may wait only on the controller/worker, with a deadline. If completion is uncertain,
retain all GPU objects rather than reclaim resources still referenced by commands.
This fallback is bounded per fixture, not permission for repeated failed restarts.
An atomic reservation permits only one live fixture per process; uncertain GPU
completion retains that reservation permanently, rejecting further creation.

Replay uses the existing recorder video worker, bounded capture slots and software
Media Foundation encoding. The test adapter is pull-paced, not an independent
game renderer; it never blocks the UI on per-frame media processing.
This is deliberately NOT GPU-only, a Present detour, swap-chain capture, fullscreen
capture, cross-process sharing, production integration or protected-game targeting.
Resize/recreation testing uses sequential fresh owned fixtures, not resident rearm.
Future D3D12 hooks require separate queue/resource-state ownership and shared-fence
transport design; never copy this owned-state assumption into arbitrary games.

No new dependency: existing windows 0.62.2 (MIT OR Apache-2.0) D3D12 bindings.

References:
- [Microsoft: readback buffers and fence requirement](https://learn.microsoft.com/en-us/windows/win32/direct3d12/readback-data-using-heaps)
- [Microsoft: resource barriers](https://learn.microsoft.com/en-us/windows/win32/direct3d12/using-resource-barriers-to-synchronize-resource-states-in-direct3d-12)
- [Microsoft: fence completion](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12fence-getcompletedvalue)
