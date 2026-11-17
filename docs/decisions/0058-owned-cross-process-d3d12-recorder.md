# ADR 0058: Owned cross-process D3D12 recorder adapter

- Date: 2026-11-12
- Status: Accepted for default-off controlled testing only

Expose ADR 0057's exact launched child through a separate CrossCapture fixture
source ID, not the production desktop/game factory. Accept only the known fixture
executable and 30 FPS target. Start a worker with three queued GPU publications,
private bounded asynchronous command writes and liveness/error/delivery gating.
Provide its D3D11 GPU context before encoder configuration. Hold each delivered
publication through the next_event -> resolver gap, as in prior controlled adapters.

Check retained child liveness before/after receiving/copying/queueing and again
before handing queued frames to the engine. Terminal observation discards queued
events, invalidates unresolved handles and never recovers to another source.
Resolved GPU leases may outlive stop. Observation is not process-death atomicity.
Retain a parent-owned local retirement fence for consumer GPU teardown, since the
producer's shared fence can become unavailable when that process exits.

Only mark GPU execution uncertain when actual consumer work is submitted; output
pool retirement before admission is not uncertain GPU execution. Otherwise an
ordinary stop can unnecessarily retain resources and exhaust restart budgeting.
Preserve the existing six-local-output-surface budget across controller restarts.
Fixture cleanup may terminate only the exact child launched by that worker.

Native tests cover source rejection, startup failure/retry, pending media after
producer exit, retained encoder leases, bounded restart, WARP/software decode and
hardware-only replay/decode with explicit capture/encoder CPU readback counters.
No UI media processing, injection, game integration or release is introduced.
