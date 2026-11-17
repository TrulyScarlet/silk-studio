# Owned quiescent D3D12 removal/recreation evidence

Date: 2026-11-12. ADR 0071. Default-off owned fixture only.

## Actually observed

On WARP and NVIDIA GeForce RTX 2080 Ti, documented ID3D12Device5::RemoveDevice
changed GetDeviceRemovedReason from healthy to **0x887A0005**. Native unused-fence
tests observed **UINT64_MAX**, and D3D12CreateDevice failed while the removed
singleton was still retained. Releasing every old fence/device interface allowed
healthy creation on the same adapter. No GPU commands were submitted in that test.

Owned same-HWND lifecycle tests performed two quiescent removals/recreations per
path. Twenty-four fresh green/blue publications per path had increasing QPC and
exact new session/size/adapter identity. Retained old red local pixels survived.
New media waited for exact approval and original Present continued after stop.

Recorder tests saved and approved FFmpeg decoded all frames across three epochs:
- Hardware: 318×178 red **840 ms**, 640×360 green **848 ms**, 800×450 blue **843 ms**;
  12/25/24 checked color samples, zero reported encoder readbacks.
- WARP/software: 318×178 red **840 ms**, 640×360 green **852 ms**, 800×450 blue
  **847 ms**; 25/26/22 checked color samples, explicit software encoding.

These targeted runs passed both native removal-contract tests, both recreated
lifecycle tests and both replay/decode tests. All-feature Clippy passed.
The subsequent complete native/hardware/decode gate passed all fifteen D3D12
lifetime/recreation/removal tests and all 53 native recorder tests (35 WARP / 18
hardware), plus prior suites. Workspace/frontend, strict dependency and settings-
save checks passed in the same validation invocation.

## Safety and limits

The parent delivery/retirement barrier, old media drain and owner renderer drain
all complete BEFORE removal. Healthy teardown releases the old GPU graph; a bare
old interface is held only for RemoveDevice/reason validation, then released before
new creation. This avoids interpreting removal's UINT64_MAX fences as completed
work and does not loosen existing uncertain-drain resource retention.

This establishes **quiescent API-driven removal/recreation**, not automatic recovery
from in-flight device loss, driver reset, TDR or protected-game behavior. No driver
reset was induced, security setting changed or unrelated device/process targeted.
No release/signing/install/push/PR performed; work remains uncommitted.
