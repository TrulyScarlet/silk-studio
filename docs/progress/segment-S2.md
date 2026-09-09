# Segment S2 Progress Log — Native display capture

**Status:** In progress
**Started:** 2026-08-26
**Plan reference:** [implementation-plan.md](../implementation-plan.md) §6
**Design reference:** [ADR 0005](../decisions/0005-wgc-d3d11-display-capture.md)

## Task checklist

1. [x] DXGI adapter/output enumeration with primary-display ordering and
   `VideoSourceInfo` mapping.
2. [x] WGC free-threaded display session over a BGRA D3D11 device.
3. [x] Bounded callback event queue and eight-entry GPU staging registry.
4. [x] Shared millisecond timestamps through `MediaTimeline`; 30/60 FPS
   delivery shaping.
5. [x] Cursor policy defaults on and is configurable before start.
6. [x] Resolution-change notification and capture-item close notification.
7. [x] Explicit stop cleanup, event-handler removal, and staged-texture release.
8. [ ] Verify resolution change and monitor removal on hardware.
9. [ ] Complete one-hour resource-growth soak and record process counters.
10. [x] Define the S3 encoder-side GPU-handle resolver/context (ADR 0006).

## Implementation

- `crates/capture-windows/src/enumerate.rs` enumerates attached DXGI outputs.
- `crates/capture-windows/src/device.rs` creates hardware D3D11 first and WARP
  second.
- `crates/capture-windows/src/session.rs` owns the WGC session thread, event
  pump, timestamp conversion, bounded GPU registry, FPS shaping, and cleanup.
- `crates/encoder-api/src/lib.rs` defines the type-erased
  `GpuFrameContext`/resolver/lease bridge; WGC exposes a session-scoped context
  without putting D3D11 types in the shared interface.
- `apps/desktop/src-tauri/src/shell.rs` uses native WGC capture for the
  persisted-configuration application path; deterministic shell tests retain
  mock capture and mock encode behavior.
- `crates/capture-windows/examples/capture_probe.rs` is the repeatable manual
  soak driver.

## Verification log

```text
cargo check --workspace --all-targets                         -> clean
cargo test -p capture-windows --all-targets                    -> 1 passed, 5 ignored
cargo test -p capture-windows --release -- --ignored --test-threads 1 -> 5 passed
cargo run -p capture-windows --release --example capture_probe \
  -- --seconds 10 --fps 60                                      -> 268 frames,
                                                               0 drops,
                                                               0 timestamp violations
cargo run -p capture-windows --release --example capture_probe \
  -- --seconds 15 --fps 30                                      -> 293 frames,
                                                                0 drops,
                                                                0 timestamp violations
cargo test -p encoder-api                                      -> 3 passed (resolver/lease tests)
cargo test -p capture-windows --release -- --ignored \
  --test-threads 1                                             -> 6 passed, including staged GPU
                                                                resource resolution
scripts/check.ps1                                               -> All gates passed
```

The ignored native run was executed on the interactive Windows development
machine. It proves actual WGC capture and 30 FPS shaping there; it is not a
claim for other GPUs, monitor topologies, or Windows builds.

## Known limitations

- The source id uses the DXGI display name and can change after monitor
  replugging.
- The S3 Media Foundation backend resolves GPU handles to preserve the
  lifetime contract, but it currently rejects WGC's BGRA texture because the
  in-device BGRA-to-NV12 conversion path is not implemented.
- Resolution-change and display-removal paths are implemented but not yet
  manually exercised.
- No one-hour GPU-resource-growth measurement has been completed.
- The environment-controlled shell smoke path still permits capture and
  encoding to be selected independently; production configuration uses native
  WGC capture and Media Foundation encoding together on Windows.
