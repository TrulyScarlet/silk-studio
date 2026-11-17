# ADR 0065: Independent in-process owned D3D12 capture worker bootstrap

- Date: 2026-11-12
- Status: Accepted for default-off exact owned-child testing only

Separate capture initialization from the renderer, using a new bootstrap feature,
CLI opt-in and source ID. Worker independently creates a DXGI factory and installs
ADR 0064's observer before renderer creation. Renderer directly calls DXGI and
does not adopt observations or initialize capture media. It sends only exact
PID/process-creation-time/HWND metadata on a bounded one-item channel: no device,
queue, chain, resource, descriptor or graphics pointer handoff. Warp selection
is explicit fixture configuration, not inferred hardware support.

Worker claims the unique pending observation for that HWND, validates current
process lifetime/window ownership, DIRECT queue, actual chain HWND and canonical
device identities. Retain selected COM objects and ADR 0062's explicit selection
contract. Renderer owns its window guards until worker joins, including errors;
this is an owned lifecycle guarantee, not arbitrary HWND-generation discovery.
Retain existing eight pending offers; one selected association is adopted here.
Do not guess a chain if observation is missing, ambiguous, closed or lost.

Renderer must report at least twelve completed Present iterations before worker
creates capture media/installs its Present detour. Rendering is not paused to
install or cancel capture. Worker creates GPU bridge, exports a fresh session,
installs existing resident Present detour and runs bounded transport independently.
No pipe/control/disk work goes in Present. Target liveness and worker/factory
failure gates remain terminal. Stale source metadata or parent approval fails
without media delivery or monitor fallback. Close worker media/observation and
join before destroying owned windows. Executable forwarding remains resident.

Window changes stay on owner thread via a two-item mailbox and one-item replies,
with deadlines. A stopped capture must allow twelve further renderer Presents.
The render-only fixture currently retains its unused baseline offscreen/readback
allocations; do not call this minimal memory or zero-copy. CPU readback metrics
count actual submitted work, not unused allocations or test-only pixel checks.

This is independent in-process bootstrap at owned child startup, not a remote
loader, after-the-fact discovery of pre-existing chains or game attachment.
Hook installation still happens before application factory calls, using the
worker's newly constructed factory. Real fullscreen/anti-cheat support remains
unverified. All dependencies already inventoried; no new packages/licenses.

## Live rearm is deliberately not enabled by this ADR

Resize/device-loss rearm additionally needs a bounded epoch/handle replacement
handshake, rejection of queued old-epoch media/ACKs, source-resource release before
ResizeBuffers, drain/device-loss outcomes and a recorder GPU-context transition
ordered before new-epoch frames. RecorderEngine already refreshes the capture's
context during FormatChanged; CrossCapture must expose the new context at that
exact boundary and reject old queued events. It currently exposes one startup context and
does not yet implement these transitions. Renderer recreation or starting a fresh
process is not live capture rearm. Implement/test that as a subsequent milestone;
current loss/unsupported format remains terminal rather than unsafe auto-recovery.
