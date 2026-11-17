# ADR 0069: Production window lifetime identity and WGC publication leases

- Date: 2026-11-12
- Status: Accepted; non-injected production WGC only

## Findings

The owned-hook route's lease tests did not establish equivalent safety for the
production WGC backend. WGC previously copied into cyclic preallocated textures
and cloned their COM interfaces for encoder leases, which kept resources alive
but did not prevent those same resources being overwritten. Resize could replace
resolver state before an already-delivered frame was resolved. Its full event
queue could also lose source-close notifications, and its pull loop could wait
indefinitely on a minimized/static source. Production IDs lacked process creation
time and could match recycled PID/HWND values.

## Decision

- Reuse the independent LeasePool abstraction already used by DXGI. Every queued
  and delivered WGC frame retains the exact Arc<GpuTextureSlot> returned to an
  encoder resolver. Acquisition only reuses slots with no other owners. Full pools
  drop incoming frames; never overwrite a retained publication.
- Eight active staging slots and an eighteen-event channel (sixteen possible
  live publications plus two reserved format boundaries). A weak-reference surface
  budget limits active plus retired allocations to sixteen across resize and
  restart. Failed allocations do not consume the budget. Retained leases can deny
  another resize; report that failure rather than grow or invalidate live pixels.
- Allocate the initial staging pool only after observing the first frame's
  ContentSize. Item.Size can differ from it; allocating an immediately discarded
  startup pool would unnecessarily exhaust the cross-session retirement budget.
  Publish observed format before staging any image. A quiet initial source may
  yield Idle until its first frame, with no invented initial media.
- Carry a new pool with FormatChanged. Callback rendering may prepare it, but
  next_event installs its resolver at the delivery boundary, after the prior
  delivered-frame resolve gap. Old queued frames precede that boundary. Existing
  resolved encoder leases stay valid after retirement/stop.
- Store terminal failure outside the lossy frame channel and close admission
  atomically on item close. Recheck terminal state after dequeue; discard queued
  media and invalidate resolvers on termination. Setup uses a cleanup guard so
  errors unregister handlers and break pool/handler reference cycles.
- Add VideoCaptureEvent::Idle. WGC returns it after an empty 50-ms receive tick;
  the engine services control without encoding fake media or treating silence as
  device/source loss. No timestamp is invented. Other implementations may continue
  to block according to their existing bounded contracts.
- Check the capture device's GetDeviceRemovedReason alongside target validation.
  Report typed DeviceLost rather than delivering queued surfaces from a removed
  device. The engine handles that typed error through its existing bounded
  SourceLost restart/reconfigure path; it must not accidentally treat the API's
  typed loss as an unrelated fatal worker error. Deterministic coverage verifies
  this dispatch; no actual WGC driver reset is claimed.
- Canonical IDs are `pid:PID:created:FILETIME:hwnd:HWND`. Validate process creation
  time, PID and HWND before setup, before StartCapture and around delivery. Recheck
  display affinity before delivery too. Target exit or a new exclusion discards
  queued media; no monitor fallback. This is not protected-process injection.
- Legacy HWND/PID-only IDs are no longer advertised or silently refreshed. They
  fail with a reselect instruction. Saved settings remain unchanged until the user
  refreshes and explicitly selects the running window. UI labels stale selections
  accordingly. Device/global monitor IDs are unchanged.

Tests include pure bounded-pool/failed-allocation checks, engine Idle handling,
and native WGC against an exact owned child: retained pixels, all-eight-slot
pressure, minimize/restore, format boundary/retired budget, exit with queued media,
stale lifetime, and an owned application enabling capture exclusion. A development-
only WindowAction sets exclusion only on the fixture's own HWND. No real game's
capture policy is changed. Native WGC startup may return transparent content;
pixel tests wait for observed fixture pixels instead of assuming first-frame color.

No external packages/versions/licenses added. capture-hook-prototype adds an
internal capture-windows **dev-dependency only** for these production-route tests.
The prototype is still not a production dependency of the desktop application.

Limits: window handles have no separately verified generation token within one
process lifetime; WGC's item lifetime/Closed event provides the OS capture binding.
Validation is not an atomic OS transaction against target state changes after the
last check. Surface counts are bounded, not GPU-memory/driver-lifetime profiling.
Minute/hour-scale game acceptance and protected-game support do not follow.

References:
- [GetProcessTimes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)
- [GetWindowThreadProcessId](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowthreadprocessid)
- [GraphicsCaptureItem.Closed](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureitem.closed)
