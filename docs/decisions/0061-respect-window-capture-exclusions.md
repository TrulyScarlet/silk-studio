# ADR 0061: Respect window capture exclusion and report the native stage

- Date: 2026-11-12
- Status: Accepted

The user explicitly authorized testing the running VALORANT window. Use only the
existing non-injected WGC route; experimental hooks remain owned-fixture-only.
The valid selected top-level HWND was not minimized and reported display affinity
0x11 (WDA_EXCLUDEFROMCAPTURE). CreateForWindow returned E_INVALIDARG before frames.
This is observed application/OS capture exclusion, not proof of which component
set it, vendor endorsement, account safety or a general statement about all builds.

Read GetWindowDisplayAffinity before WGC startup and again before item creation.
If Windows reports WDA_MONITOR or WDA_EXCLUDEFROMCAPTURE, return PermissionDenied
without creating a capture session or selecting a monitor. Do not modify affinity,
change game/security settings, inject, or try to circumvent the exclusion.
If the read-only query fails, leave the ordinary WGC API to decide support; that
is not authorization for an alternate/bypass route. Keep source PID/HWND validation.

Add setup-stage labels retaining the original HRESULT so unsupported-window failures
are diagnosable. An explicit ignored probe requires PID/HWND/process-creation-time,
verifies the VALORANT image with query-limited process rights, and writes only under
the approved temporary artifact directory. Its blocked-result validation is not a
capture-success test. No screenshot/video was captured from the excluded window.

References:
- [Microsoft display affinity values and behavior](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)
- [Microsoft read-only affinity query](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowdisplayaffinity)
