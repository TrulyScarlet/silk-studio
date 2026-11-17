# ADR 0080: Typed producer loss and independent consumer retirement

- Date: 2026-11-14
- Status: Metadata and terminal parent boundary implemented; automatic replacement incomplete

Follow-up ADR 0081 adds reconstruction in a distinct owned manual hardware source
profile. This original terminal profile stays terminal. Independent DLL-helper/
application-observed replacement remains incomplete.

## Problem

An unexpected producer removal can invalidate queued shared-fence headers before
the parent reads a loss notice. Immediately retiring its output resolver would
invalidate an already-delivered frame before the caller crosses the next event
boundary. Treating UINT64_MAX as completion is unsafe. A failed partial copy can
also leave consumer work whose normal retirement signal was not submitted.

## Decision and implementation

Add fixed-size session-bound DeviceLost packets with a finite supported HRESULT
reason set, and an exact owned-fixture TriggerOwnedLoss command with no payload.
Existing routes reject unexpected loss/control messages rather than enabling
recovery implicitly. Triggering requires the exact pinned owned child session and
an explicit manual hardware fixture fault; it never accepts an arbitrary PID.

DeviceLossEpoch is a pure metadata gate: exact current identity, no overlapping
transition, checked next epoch, exact retirement, healthy validated same-adapter/
QPC replacement metadata, and exact approval before publishing new current media.
Wrong phase/identity/offer, exhaustion and stale approval close it permanently.

An opt-in RemoteConsumer path rejects new copies/ACKs after actual producer fence
removal while leaving independent delivered output publications resolvable. It
continues validating each header; loss must not hide stale/malformed metadata.
At the caller's delivery boundary, retirement issues a new **non-shared D3D11 tail
fence signal** strictly above every seen/possibly attempted sequence. It checks
the consumer device and a bounded, non-sentinel completion. Failure stays terminal
and conservatively retained. The normal consumer path remains fail-stop and cannot
silently opt into this handling after it has closed.

The adapter enables this only for the explicit owned manual loss probe. Its
bounded trigger mailbox cannot resize/recreate. After a valid loss notice, it
advances delivery, crosses the caller boundary, independently retires old imports
and returns an actionable terminal error. **No replacement is offered yet.** The
engine must not silently spawn a new source or switch to monitor capture.

## Native test scope

A hardware fixture loses its owned D3D12 device with a queued signal still
pending, without a prior parent retirement exchange. Real queued headers are
held across loss. Tests validate preserved delivered-frame resolution before the
boundary, terminal invalidation after it, retained red pixels through normal
source exit, unchanged default fail-stop behavior and stale-header rejection.
The adapter and RecorderEngine tests verify terminal handling, bounded stop,
no retarget/restart and a saved/decoded hardware MP4 before loss.

The fixture intentionally retains its removed producer graph until child exit.
This is not automatic graph reconstruction, independent DLL-helper recovery,
in-flight partial-consumer-error coverage, a TDR, real-game/fullscreen or audio
acceptance. WARP is explicitly unsupported for this new remote-loss probe.
No general hook mode is exposed to production. No new package/version/license.
