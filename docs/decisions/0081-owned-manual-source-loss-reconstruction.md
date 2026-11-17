# ADR 0081: Typed-loss reconstruction in the owned manual hardware source

- Date: 2026-11-14
- Status: Implemented and combined-regression validated for owned manual hardware source

Extend ADR 0080 with a distinct exact-owned manual-source profile,
`remove-and-recreate-on-command`. The existing `remove-on-command` mode remains
terminal. Both are hardware-only, no Present/factory hooks or DLL loading, and
unavailable to production UI/general process selection. An initial exact remote
ACK is required before the reconstruction probe may remove its device.

The owner removes its D3D12 device while a queue signal is pending and sends a
typed loss notice without a prior parent retirement exchange. It pumps its owned
windows but performs no further graphics work. The adapter rejects queued shared
frames, crosses the caller's delivered-frame gap and proves independent D3D11
consumer-tail completion. It drops old imported resources before sending the
exact next-session Retired message. Wrong/missing proofs stop normally and boundedly.

Only after that proof may the producer mark remote exports retired, close original
shared handles and release confirmed-invalid producer/application graphs. The
renderer retains both HWND guards, recreates its graph in the same process with
the **same budget**, and exports a fresh epoch. No COM object crosses processes.
The parent validates target, adapter/QPC and bounded dimensions/handles, publishes
the new format, and sends exact approval. The owner resumes only after approval.
PTS stays on the same QPC timeline; encoder/replay use existing format epochs.

The loss mailbox cannot resize/recreate. Stop at either proof boundary must be
normal/bounded; caller stop while holding an old frame must unblock both sides.
Retained contexts must exhaust, not bypass, six surfaces.

Targeted adapter tests pass two reconstructions with same PID/lifetime/HWND,
monotonic PTS and original retained red pixels. Hardware RecorderEngine saves and
approved-FFmpeg-decodes red 318x178, green 640x360 and blue 800x450 clips with zero
reported encoder/capture readbacks. Invalid/missing proofs, unprimed loss, stop
boundaries and retained-context exhaustion are covered separately.

This is **manual-source owned hardware reconstruction**, not independent DLL
recovery, observation of a third-party replacement graph, WARP remote loss, real
driver/TDR, game/fullscreen, audio or release acceptance. WARP remains rejected
before graphics startup after ADR 0079 faults. A queue signal was pending at
removal; no partial consumer-copy API failure is claimed. No new dependency.
