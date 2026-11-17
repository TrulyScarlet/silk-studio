# ADR 0049: Bounded local GPU recorder bridge

- Date: 2026-11-11
- Status: Accepted for the default-off controlled hook adapter

## Ownership design

Keep imported keyed-mutex textures on the transport worker. Preallocate three
ordinary BGRA8 textures on that same protected D3D11 device. Copy an imported
frame to a reserved local slot, then release/ACK the imported slot. Deliver a GPU
handle through VideoCapture with the existing GpuFrameContext resolver. No CPU
readback occurs in GPU-bridge mode on the capture worker or Present callback.

A publication token owns each local reservation. The bounded event queue owns
tokens for unread frames; the capture object retains the currently delivered
token until the next pull. A resolver returns an Arc lease, so releasing the
publication does not permit reuse while an encoder still holds the surface.
Reject stale handles and invalidate the resolver on stop/failure. Do not let a
small timing gap between next_event and resolve permit a queued overwrite.
Weak references also prevent reuse. Queue-full/drop/error paths release tokens.

Reserve slots through a short try-lock on the producer; drop capture work if the
pool or lock is busy. No extra texture allocation under pressure. Maintain a
per-capture pool budget that counts old contexts/leases across restarts; reject
startup when another three-slot pool would exceed six local surfaces. Existing
leases may remain alive after shutdown, but their handles must not resolve again.

## Validation and explicit limits

Add portable publication/lease/race/stale-handle tests and native retained-pixel,
restart, pool-exhaustion, and replay/decode tests. Keep the CPU bridge as an
explicit test comparison mode. Record capture readbacks and encoder GPU-surface
versus CPU-readback input counters: software MFT fallback is not evidence of a
GPU-only encoder path. Hardware replay must demonstrate GPU-surface inputs and
zero readbacks before describing that tested path as GPU-only.

GPU copy/convert commands use the same multithread-protected immediate context;
their GPU ordering, not CPU waits, protects source reads. This is not a zero-copy
pipeline: GPU copies are intentional to decouple cross-process ownership.
No game attachment, hook-thread concurrency, cross-process resize/rearm, UI
selection, anti-cheat acceptance, or production packaging is added here.
