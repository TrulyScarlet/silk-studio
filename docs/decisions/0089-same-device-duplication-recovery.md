# ADR 0089: Freeze through temporary Desktop Duplication loss

Date: 2026-11-17

## Problem

Fullscreen focus transitions can invalidate DXGI Desktop Duplication without
removing the D3D11 device. Guarded capture previously reported ACCESS_LOST as
DeviceLost; recorder recovery then discarded replay history.

## Decision

Guarded capture releases the invalid duplication on ACCESS_LOST or
SESSION_DISCONNECTED and retries DuplicateOutput on the existing output/device
at most four times per second. Access denied, temporarily unavailable and
unsupported duplication creation are retried (desktop/mode transitions may cause
these). Unexpected failures still surface. GetDeviceRemovedReason is checked
before each attempt; genuine device removal/reset retains engine recovery.

While interrupted, continue the existing bounded GPU snapshot/lease pipeline,
canvas, timestamps and media origin. Do not publish a new format or SourceLost
solely for duplication loss. Before the first focused game image remain Idle.
Recreation alone does not permit live pixels: require a fresh presentation while
the selected game is foreground. Refresh output coordinates/monitor and reset
cursor state on recreation. Reallocate only the private desktop texture when
incoming desktop dimensions change, not the encoder/published snapshot pool.
Evaluate crop resizing only after a fresh foreground presentation, never against
transient bounds while duplication is unavailable. Check real monitor moves only
against healthy presentations. Target lifetime and capture-affinity exclusions
remain enforced during freezing; no WGC, injection or alternate-source fallback.

## Limits

True GPU loss, game exit, moves to another output and genuine game-canvas size
changes still use the existing recovery/format-change paths and can clear history.
This does not introduce cross-adapter recovery or resolution scaling. Persistent
desktop unavailability freezes indefinitely with bounded memory/retry cadence;
automatic timeout-to-desktop capture is intentionally absent. Existing DXGI
overlap/overlay limitations remain. No dependencies added.

## Validation

Deterministic tests separate temporary duplication HRESULTs from device failure
and check repeated interruption/retry timing and presentation gating. An opt-in
native test recreated duplication three times and confirmed the same device,
shared context, frozen texture and held GPU lease survived. See
`docs/evidence/dxgi-fullscreen-recovery-2026-11-17.md` for live replay results.
