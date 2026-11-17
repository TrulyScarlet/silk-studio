# ADR 0066: Bounded same-chain live D3D12 resize epochs

- Date: 2026-11-12
- Status: Accepted for default-off owned explicit resize only

Add a distinct d3d12-rearm-fixture feature, source ID and CLI opt-in over ADR 0065.
The first live operation is explicit same-process/same-HWND/same-chain resize,
not device-loss recovery or an arbitrary application's automatic ResizeBuffers
hook. The renderer keeps presenting while capture waits at protocol boundaries.

Use validated fixed 128-byte SK12 Resize/Retire/Retired packets. Request contains
the exact current session and even bounded dimensions (2..1920 × 2..1080). One
in-flight transition plus one queued control request maximum. Epoch increments
checked, retaining target and generation. Replacement hello must preserve target,
generation, adapter and QPC frequency and match requested dimensions/next epoch.
Malformed/unexpected/duplicate/wrong-session control or deadlines are terminal.

Protocol:
1. Worker cancels capture admission and emits Retire(next session).
2. Parent advances delivery epoch, discards old queued events, and crosses a
   next_event retirement barrier **after** the prior delivered-frame resolver gap.
   Only then retire old context/drain parent-owned consumer completion and send
   exact Retired(next session). Resolved encoder leases may outlive retirement.
3. Worker validates outstanding old ACKs against exact published slot/sequence
   snapshots during retirement, but never applies them to a new GPU ring. Release
   old bridge source/list references only after drain. Clear old media while the
   executable Present trampoline remains resident and forwarding.
4. Owner-thread mailbox performs real ResizeBuffers. Worker validates actual
   dimensions and creates a fresh bounded ring/fences/handles, stages it inactive
   on the same selected Present hook, and sends replacement hello.
5. Parent validates/imports, queues FormatChanged with the new GPU context before
   any new frames, and approves the exact session. Worker then resumes admission.
   New rings start sequences afresh; old-session ACKs/frames never release them.

RecorderEngine already refreshes capture GPU context on FormatChanged and rebuilds
its replay epoch. CrossCapture now orders that context replacement at the event
boundary instead of changing it beneath an already-delivered frame. Keep continuous
QPC origin/cadence across resize, with downtime preserved and no catch-up bursts.
The six-local-surface retirement budget remains enforced across all replacements;
retaining old contexts/leases can deny further resize rather than grow memory.
Child uses the same bounded pool budget as it replaces inactive media.

Tests must cover deterministic packet/epoch rejection, native ResizeBuffers,
retained old pixels, no new media before approval, old-ACK rejection, resolver-gap
barrier, bounded retained-context failure and three actual replay/decode epochs on
WARP/hardware. Test fixture colors are red/green/blue per size epoch, not game data.

No capture work is added to UI/disk threads or control I/O to Present. The owner
thread performs only its renderer's explicit resize; media drain/import/reconfigure
stays outside Present. Native GPU drain cannot be made safe by declaring a future
fence complete: uncertain drain remains terminal. No silent retry/monitor fallback.

Device replacement/loss (including removal during drain), ResizeBuffers1 queue
replacement, remote loading, pre-existing chains, automatic third-party resize
interception and protected-game/fullscreen support remain separate and unverified.
No new dependency/version/license; existing windows/MinHook/internal interfaces.

Reference: [Microsoft ResizeBuffers and backbuffer-reference release requirements](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgiswapchain-resizebuffers).
