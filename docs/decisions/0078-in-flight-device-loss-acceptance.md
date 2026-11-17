# ADR 0078: Staged in-flight D3D12 device-loss acceptance

- Date: 2026-11-14
- Status: In progress; recovery not established

Quiescent removal and healthy recreation do not prove loss handling while media
commands are pending. First test an owned DIRECT queue with an unsatisfied local
fence followed by a queued signal, then explicitly remove that owned device.
The pending value must be observed before removal and UINT64_MAX rejected after
removal. Release the wait on any unwinding path; do not cause a TDR or alter
system/driver/security settings.

Next exercise an actual capture copy held behind that queue wait in a separate
owned fixture process. Reject publication after removal and bound cancellation.
Existing uncertain-resource retention is allowed only as a fail-stop fixture
result, never described as automatic recovery. Process isolation bounds retained
removed graphs and avoids poisoning later tests through D3D12 adapter singletons.

Before recovering in-process, prove safe retirement of producer, imported D3D11
work, parent delivery gaps, retained encoder leases and all old device references.
Do not equate a removal fence sentinel with successful completion. Do not free
uncertain live GPU work or retry allocation around the six-surface budget. A
replacement requires an application-created authoritative graph and fresh exact
parent approval; never install/uninstall hooks or guess a replacement queue.

Production handling remains bounded fail-stop until those lifetime preconditions
are implemented and validated. Real TDR, driver reset and arbitrary game recovery
remain distinct acceptance scenarios.

Reference: Microsoft ID3D12Device5::RemoveDevice documentation, retrieved 2026-11-14:
https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12device5-removedevice
It documents fence signaling to UINT64_MAX on device removal. No new dependencies.

## Narrow retirement/recreation implementation

A later implementation adds an independent non-shared D3D11 retirement fence
after each local interop copy. Shared D3D12 fence removal sentinels cannot prove
consumer completion. Confirmed removed producer graphs may be released only when
there was no local consumer work, or that independent local fence proves all
consumer work complete without an uncertain signal. Exported remote rings remain
ineligible. Healthy/uncertain devices retain the existing bounded drain/fail-stop
behavior. Renderer retirement additionally requires no installed Present hook and
confirmed removal of both owned D3D12 graphs; no hook is unloaded or reinstalled.

WARP/hardware probes now pass same-process/same-budget graph recreation after a
provably pending media-copy removal, both with and without a previously published
local consumer. Three fresh red frames arrive and retained independent old local
pixels survive. This is explicit owned-graph reconstruction, not an application's
automatically observed replacement, remote-worker recovery, full replay resumption
or TDR acceptance. Uncertain local/remote consumer recovery remains unfinished.
Ten successive same-process loss/recreation cycles on each WARP/hardware probe
also pass with retained local pixels; no process-exit cleanup between cycles.

Microsoft device-loss guidance describes release/recreation of invalid device
resources, not successful fence completion:
https://learn.microsoft.com/en-us/windows/uwp/gaming/handling-device-lost-scenarios
Its D3D11 discussion is used only as lifetime guidance; D3D12/interop behavior is
separately constrained and tested above. No forced TDR or system change is used.
