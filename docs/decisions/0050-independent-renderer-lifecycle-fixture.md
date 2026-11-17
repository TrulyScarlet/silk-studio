# ADR 0050: Independent rendering and cooperative hook lifecycle tests

- Date: 2026-11-11
- Status: Accepted for controlled development fixtures only

## Scope

Exercise the real Present detour with two independently paced renderer threads,
each owning its window/device/swap chain. Only the explicitly selected renderer
has capture state. An unrelated renderer calls the same patched Present entry
from another thread and must continue without being captured. Neither rendering
loop is driven by frame-consumer requests or encoder progress.

Add an atomic admission gate with RAII in-flight work tracking. Capture cancellation
closes admission and invalidates the media epoch; it does not remove the detour.
Already queued frames predate cancellation and must be discarded by a closing
consumer. A drained capture gate proves only capture work ended, NOT that no
trampoline/original Present call is executing.

The fixture separately gates EVERY owned render/Present call. Only after both
renderers acknowledge pause and that fixture gate is closed/drained may the owner
remove or replace its detour. This cooperative guarantee is not available in
arbitrary games. On uncertain cleanup, retain the detour/trampoline rather than
freeing executable code under a caller. No injected DLL unload is introduced.

## Fault instrumentation

The separate default-off `fixture-probes` feature enables a bounded checkpoint
inside the selected callback and a one-shot synthetic post-Present HRESULT.
These are controlled fault tools, not production callback behavior. The checkpoint
lets a controller cancel while capture work is in flight; it releases automatically
at its deadline even if a test fails. The error probe calls the real Present first,
then injects DXGI_ERROR_DEVICE_REMOVED into the normal invalidation branch. Report
that as simulated loss notification, never as physical GPU removal verification.

Native tests also resize the selected source, verify fail-closed invalidation,
reattach explicitly in a new epoch while both renderers are paused, recreate only
the fixture's own D3D11 device, and verify pixels from the new source. Retained
staging leases must keep their original pixels. No driver reset, system-wide GPU
fault, game injection, anti-cheat interaction, or actual user application is used.

## Remaining boundaries

This tests independent rendering and explicit quiescence, not safe arbitrary-game
detach, automatic cross-process resize/rearm, production independent export workers,
device recovery under actual driver removal, or fullscreen/game compatibility.
Driver calls and thread joins still lack hard real-time completion guarantees.
