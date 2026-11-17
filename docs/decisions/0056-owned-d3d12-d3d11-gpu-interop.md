# ADR 0056: Owned same-adapter D3D12/D3D11 GPU publication

- Date: 2026-11-12
- Status: Accepted for default-off owned-fixture testing only

Use explicit `d3d12-gpu-fixture` without enabling MinHook/injection. Create a D3D11
BGRA device on the D3D12 device's exact adapter LUID; hardware additionally requests
video support. WARP uses explicit WARP creation without hardware video flags and
still must match LUID. Require device5/context4
shared-fence interfaces. Unsupported interop is a terminal error, not CPU fallback.

Allocate three BGRA8 D3D12 shared DEFAULT-heap textures, COMMON at interchange,
and two shared fences opened by D3D11. Unnamed noninherited NT handles are RAII
closed after opening within this process; this is NOT cross-process IPC/security.
Capture copies the selected PRESENT backbuffer to a shared COPY_DEST texture,
restores source PRESENT and destination COMMON, then signals produced(sequence).
No Map/readback buffer is used by this route. Original rendering remains bounded
and independent of full capture/consumer pools.

Outside submission, poll producer completion, queue D3D11 Wait(produced), copy to
one of three existing LocalGpuBridge output surfaces, Signal(consumed), Flush.
Do not recycle a shared source until consumed completion, regardless of CPU handoff.
If local pool is full, discard only a producer-complete source before any consumer
copy is submitted. Immutable publication handles/resolved leases independently
protect the local output pool from queued-event and encoder retention races.

Keep source backbuffer COM references until drained. Shutdown/resize closes all
publication admission, invalidates unresolved output handles, drains both fences,
and resets source-bearing command lists before ResizeBuffers. Resolved output
leases retain their D3D11 device/texture independently. On uncertain execution or
signal completion retain GPU objects and core reservation until process exit;
never retry allocation indefinitely. Capture/encode/render never wait for UI/disk.
Fence polling is nonblocking in the frame path; OS/driver calls are not hard-bounded.

Use the established output-pool restart budget and global unique handles. Native
tests may deliberately read back pixels as evidence; recorder GPU runs must report
zero capture CPU readbacks and require hardware encoding without software fallback.
WARP/software encoder tests must explicitly label encoder readbacks.

No game attachment, queue discovery, hook, cross-process protocol, fullscreen,
multi-adapter support or anti-cheat/account-safety claim. Existing windows 0.62.2
(MIT OR Apache-2.0) bindings only; no new package/license.

References:
- [Microsoft: shared heaps](https://learn.microsoft.com/en-us/windows/win32/direct3d12/shared-heaps)
- [Microsoft: D3D11/D3D12 shared-fence interop](https://learn.microsoft.com/en-us/windows/win32/api/d3d11_4/nf-d3d11_4-id3d11device5-opensharedfence)
