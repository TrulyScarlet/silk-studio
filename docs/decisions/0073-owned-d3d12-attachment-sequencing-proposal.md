# ADR 0073: Owned D3D12 attachment sequencing (proposal)

- Date: 2026-11-13
- Status: Proposed; **not implemented or accepted as attachment evidence**

Follow-up: ADR 0075 implements a narrower early-owned-fixture route using explicit
phase ownership of inherited pipes rather than the proposed kernel-handle control
envelope. This proposal is historical design analysis, not evidence that all of its
steps or general third-party attachment have been implemented.

## Constraint found during implementation inspection

The existing remote loader accepts only FixturePeer, whose canonical executable
and passive opt-in are D3D11-specific. The D3D12 Peer launches an integrated
bootstrap worker. Reusing either API by changing a filename would not establish
independent D3D12 attachment: the existing chain might predate factory observation,
and capture_worker currently relies on owner-side present counters/action mailboxes
within the same linked module. A DLL has separate Rust globals. The normal desktop
dependency graph must remain unchanged.

## Proposed smallest complete route

1. Add an exact owned D3D12 passive launch mode. It creates/pumps the two owned
   HWNDs but holds swap-chain creation behind a bounded readiness handshake. This
   is explicitly an **early attachment fixture**, not existing-game discovery.
2. Retain the launched process handle and pin PID/creation time/HWND. A sealed
   owned-target abstraction may share loader mechanics with D3D11, but callers
   must not construct it from arbitrary PIDs, paths, UI selections or foreground
   state. Preserve independent one-attempt reservation per retained child.
3. Load only the canonical sibling helper through normal LoadLibraryW. Relocate
   verified module exports; allocate remote data only, never executable payloads.
   Keep remote parameters alive until worker completion or target exit. No work
   under loader lock and no unload/reinstall of resident forwarding trampolines.
4. The helper installs its factory observations before signaling the passive
   renderer to create chains. Use parent-owned, explicitly duplicated kernel
   control handles with bounded waits, not discoverable unauthenticated named
   channels or stdin shared between two protocol readers. Validate remote handle
   ownership/type and include the selected session in the fixed-size bootstrap
   envelope. Do not treat a readiness timeout as capture permission.
5. Discover only creations observed in that helper module. Both modern HWND and
   legacy creation must work; ambiguous or pre-existing unobserved chains fail
   closed. Query direct queue/device/chain identity exactly as today. Never infer
   a D3D12 queue by using a D3D11 device shortcut or recycled COM addresses.
6. Refactor worker startup away from executable-private present counters. The
   helper's installed hook supplies its own bounded progress/capture evidence;
   fixture renderer controls remain an explicit separate test-only transport.
   Require parent approval before media admission and exact ACK/fence ownership.
7. Validate retained pixels, selected-versus-unrelated content, worker failure,
   startup cancellation, stale target, wrong approval, stop/continued Present and
   saved/decoded software/hardware replay. Then add the recorder adapter. Do not
   route production selections through it until a separate shipping decision.

## Excluded from this first route

Pre-existing-chain discovery, arbitrary third-party loading, protected processes,
anti-cheat changes, monitor fallback, automatic third-party ResizeBuffers recovery,
in-flight removal/TDR recovery and exclusive-fullscreen acceptance remain separate.
Real games need permissible explicit targets and actual acceptance tests. The
quiescent RemoveDevice tests do not satisfy in-flight recovery.

No dependency addition is proposed. Actual helper/loader/IPC changes need tests
and an accepted implementation decision before this proposal can be marked done.
