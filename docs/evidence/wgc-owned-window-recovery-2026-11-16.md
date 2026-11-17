# WGC selected-window recovery evidence — 2026-11-16

## Scope

Non-injected WGC capture, with experimental hook development paused (ADR 0084).
No game or anti-cheat settings changed. No third-party process injected.

## Findings and fixes

1. Forward compositor timestamp gaps previously reset the timeline to zero, then
   clamped timestamps against the old last frame. This could stall FPS shaping
   until the new timeline caught up and lose elapsed video time. WGC now accepts
   forward gaps without changing the origin; backward discontinuities still fail.
   Deterministic media-clock tests cover repeated long gaps and backward resets.
2. A separate-process owned-window test reproduced terminal surface-budget
   exhaustion during minimize/restore followed immediately by resize. Allocation
   now returns temporary unavailability separately from allocation errors. The
   frame callback closes the current frame and retries later without marking the
   new format accepted. The sixteen-surface budget is unchanged. Actual allocation
   or WGC recreation errors remain terminal.

## Native verification

Commands executed successfully on this Windows machine:

```powershell
cargo test -p capture-windows --test native_display -- --ignored --test-threads=1 --nocapture
cargo test -p capture-windows owned_capture_exclusion_and_self_target_fail_closed -- --ignored --nocapture
cargo test -p capture-windows --lib selected_wgc_window_resumes_after_minimize_and_resize_and_closes_safely -- --ignored --nocapture
```

- All seven native display tests passed: monotonic frames, start/stop behavior,
  enumeration, 30 FPS shaping, GPU resolution, and NV12 conversion.
- Owned HWND exclusion returns `PermissionDenied`. Selecting the recorder's own
  process remains rejected before affinity checks; this is not evidence for the
  excluded external-window production route.
- The lifecycle regression creates a separate child process and selects its exact
  PID/creation-time/HWND identity through `game:`. Backend stays `wgc-window`.
- A black topmost, non-activating owned window overlaps the target. After draining
  queued frames, the captured center pixel remains the target's nonblack baseline.
- Minimize, restore, immediate resize, fresh resized frames, monotonic resume
  timestamps and target closure all pass. The completed overlap/lifecycle test
  passed once, then five consecutive additional runs.
- Pixel inspection uses diagnostic CPU readback of the owned window only. This
  does not change production GPU-surface delivery.

## Limits

This is a static Win32 fixture, not real gameplay or an exported replay. A center
pixel check is narrow evidence, not proof of all-frame/all-pixel privacy. HDR,
exclusive fullscreen, real driver loss, multi-GPU transitions, cross-monitor
movement and protected games were not verified in this run. The long-idle gap fix
is deterministically tested, not a sleep/resume hardware claim. When retained GPU
leases consume the budget, resize waits and drops input rather than growing memory;
progress requires the consumer eventually releasing old leases.
