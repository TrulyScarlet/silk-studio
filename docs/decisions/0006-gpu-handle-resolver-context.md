# ADR 0006 - GPU handle resolver/context bridge

**Status:** Accepted for the S2/S3 boundary  
**Date:** 2026-08-26  
**Segment:** S2 preparation for S3

## Context

S2 emits `GpuFrameHandle` values instead of exposing D3D11 objects through the
capture trait. S3 encoders still need a bounded-lifetime view of the native
resource, and the staging ring may evict a handle before a slow encoder reads
it. The bridge must not add CPU readback, a global resource table, or a
dependency from the shared media types onto a Windows API.

## Decision

1. `encoder-api` owns the bridge contract. `GpuFrameResolver` resolves one
   `GpuFrameHandle` into an `Arc<dyn Any + Send + Sync>` resource. The concrete
   resource type remains in the platform backend; an encoder adapter may
   downcast only to a resource type it explicitly supports.
2. `GpuFrameContext` wraps one session resolver and exposes resolution from a
   `VideoFrame`. A CPU payload is rejected before the resolver is called.
3. `GpuFrameLease` owns the resolved `Arc`, so the native resource remains
   alive after the capture staging ring evicts the handle. The encoder holds a
   lease only for the encode operation and releases it before the next frame
   unless its backend needs a longer documented lifetime.
4. `VideoEncoder::set_gpu_frame_context` installs or clears the context. It is
   called before the first encode and after the encoder drains. Software
   encoders may ignore it; GPU encoders reject an absent or incompatible
   context before accepting frames.
5. `capture-windows` supplies a session-scoped context. Its resolver clones a
   `GpuTextureSlot` from the eight-entry staging registry. Unknown or evicted
   handles return `GpuFrameResolveError::StaleHandle`; the S3 worker counts and
   drops those frames rather than blocking capture.
6. The WGC D3D11 wrapper remains responsible for its multithread-safety
   guarantee. No raw D3D11 context or platform object is added to
   `encoder-api`, and no CPU readback is introduced.

## Consequences

- Capture and encoder interfaces remain independently testable; only the
  type-erased bridge is shared.
- A slow encoder can observe a stale handle because the capture registry is
  deliberately bounded. This is a controlled drop, not a memory-growth path.
- The S3 Media Foundation adapter downcasts `GpuTextureSlot` and verifies that
  its device/context belongs to the same capture session before conversion.
- Deterministic shell helpers remain mock-backed, while the configured desktop
  application installs the native capture context through the engine worker.

## Verification

- `encoder-api` unit tests cover CPU-payload rejection, typed lease access, and
  stale-handle propagation.
- The ignored native WGC suite resolves a staged D3D11 texture through the
  context, and the S3 native integration test consumes it through the local
  same-device Media Foundation path.
