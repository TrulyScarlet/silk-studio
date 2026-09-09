# ADR 0005 — WGC/D3D11 display capture

**Status:** Accepted for S2 implementation
**Date:** 2026-08-26
**Segment:** S2

## Context

The S2 video backend must capture a selected Windows display without process
injection, produce GPU-backed `VideoFrame` values, remain bounded when the
consumer is slow, and preserve a shared monotonic timestamp timeline. The
backend must also release platform resources deterministically on stop.

## Decision

1. Use the Windows Graphics Capture API with a free-threaded
   `Direct3D11CaptureFramePool`. DXGI enumerates attached outputs and supplies
   the monitor handle used to create the capture item. The source identifier is
   the DXGI device name (for example, `\\.\DISPLAY1`); this is stable for a
   fixed topology but may be renumbered after replugging.
2. Create a BGRA-capable D3D11 device, preferring the hardware driver and
   falling back to WARP. Enable `ID3D11Multithread` protection before any
   callback issues D3D11 context operations.
3. Initialize the Windows Runtime as multithreaded on `silk-wgc-session`, then
   keep session lifecycle and cleanup on that thread. The free-threaded
   callback uses cloned pool/device/context references under explicit
   multithread-safety wrappers. The public backend crosses the worker boundary
   only with channels, atomics, counters, and opaque `GpuFrameHandle` values.
   The callback copies each WGC surface to a private default-usage texture
   before publishing the handle.
4. Bound the callback-to-consumer queue at 64 events and the staged texture
   registry at 8 entries. If the event queue is full, drop the newest frame and
   increment the drop counter. If the registry is full, evict the oldest GPU
   texture. Neither path blocks the compositor callback or grows GPU memory.
5. Treat WGC `TimeSpan.Duration` as 100-nanosecond ticks, convert it through
   `MediaTimeline` at a 10 MHz source time base, and emit millisecond
   `VideoFrame` timestamps. A 60-second jump or backward timestamp is reported
   as a backend failure rather than silently corrupting replay ordering.
6. Use a 31 ms minimum delivery interval for a 30 FPS request and no
   decimation for 60 FPS. The WGC cursor policy defaults to enabled and can be
   changed through `WgcDisplayCapture::set_cursor_capture` before start.
7. On stop, mark callback delivery inactive, signal the session thread, join
   it, unregister event handlers, close the session/frame pool, and clear the
   staged texture registry. Final counters remain available for diagnostics.

## Consequences

- The S2 backend is independent of OBS and uses only Microsoft platform APIs
  plus project interfaces.
- GPU memory is bounded independently of capture duration and consumer speed.
- `FramePayload::Gpu` is intentionally opaque. The encoder-side
  resolver/context contract is defined separately in ADR 0006; no CPU readback
  is introduced in S2.
- Display names are a practical MVP identity, not EDID identity. EDID-based
  persistence can be added if replug testing requires it.
- WGC display capture is supported from the project floor of Windows 10 1903
  (build 18362); the API capability probe still controls startup behavior.

## Verification

- On the interactive Windows development machine, the release native tests
  passed: monotonic primary-display capture, idempotent stop/double-start
  rejection, 30 FPS shaping, and prompt post-stop behavior.
- The 10-second release probe captured 268 frames at 60 FPS with zero queue
  drops, zero resize events, and zero timestamp-order violations.
- One-hour soak, monitor removal, resolution changes, and GPU process-counter
  measurements remain unverified and are S2 exit work.
