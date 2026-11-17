# ADR 0071: Owned quiescent D3D12 device-removal probes

- Date: 2026-11-12
- Status: Accepted for default-off owned fixture tests

Extend the device-recreation test route with `--fault=replace-remove-quiescent`.
This uses documented ID3D12Device5::RemoveDevice on an **owned**, already drained
device. It is not a driver reset, TDR, protected-process action, or simulation of
removal while work is in flight.

Follow ADR 0068's exact parent retirement barrier and release old worker media,
queue/chain/device and pending factory references first. Owner thread verifies its
renderer and capture work drained, retains the two HWND guards plus a bare
ID3D12Device5 interface, and drops the entire healthy renderer/device graph while
its teardown still has valid completed fences. Only then call RemoveDevice,
require GetDeviceRemovedReason to fail, and release that last device interface
before new device creation. Do not compare old and new COM addresses. Existing
fresh-association, format/adapter/epoch and approval validation remains mandatory.

Native tests additionally create an unused fence, call RemoveDevice, require the
UINT64_MAX completion sentinel, and verify creation fails while the removed
singleton is retained. After releasing fence and device interfaces, creation must
return a healthy device. This is done without any submitted GPU work. UINT64_MAX
is loss, not proof that a future GPU operation completed; existing uncertain-drain
paths remain terminal and are not weakened to enable this probe.

Run WARP/hardware native lifecycle tests and actual three-epoch saved/decode replay
tests. In-flight removal, unexpected application/device failure during drain,
adapter-wide reset and automatic third-party recovery remain separate work.
RemoveDevice requires supported runtime/ID3D12Device5; unsupported casts fail rather
than emulating success. Microsoft's current API page lists Windows build 20348.

No new dependency/version/license or production hook route. Existing windows
0.62.2 (MIT OR Apache-2.0) provides the API. The fixture's opt-in and replacement
feature are required; this cannot target another application's graphics device.

References:
- [RemoveDevice and UINT64_MAX fence behavior](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12device5-removedevice)
- [D3D12CreateDevice removed-singleton behavior](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-d3d12createdevice)
