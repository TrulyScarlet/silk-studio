# Owned D3D12 factory interception and queue-discovered capture

Date: 2026-11-12. ADR 0064. Default-off, exact owned-child route only.

## Implemented boundary

`d3d12-factory-hook-fixture` installs an original resident
CreateSwapChainForHwnd detour, guarded by exact executable and explicit
`--allow-owned-d3d12 --present-hook --factory-hook` flags. The fixture's renderer
calls DXGI directly. The observer retains successful queue/chain offers in eight
fixed slots; the controller claims an exact canonical chain once without supplying
a queue. ADR 0063 validates DIRECT type, canonical device relationship and owned
HWND, then retains up to eight adopted associations with owned window guards.
Eight pending plus eight adopted is the explicit maximum association storage.

Callback always calls original first, preserves HRESULT/output, and observes only
the current-process fixture class. Try-lock contention/overflow/invalid successful
observations set terminal capture failure; original rendering creation still
returns successfully. No GPU submission, capture readback, pipe/disk work or
controller-lock wait in the factory callback. Normal creation performs QI/refcount
work; no constant-latency or zero-suspension assertion is made. Forwarding code
and original module stay resident until child exit; no unhook/unload is attempted.

Distinct source ID: `owned-d3d12-factory-and-present-hook`. The prior manual and
Present-only sources remain separate. Startup requires both expected associations
to be observed/claimed and no errors/drops; no cooperative or monitor fallback.

## Actual native evidence

WARP and NVIDIA GeForce RTX 2080 Ti passed separate child probes for:
- forwarded failed native creation (no offer);
- successful ignored-class creation (no offer), using another owned HWND only;
- observed selected/unrelated chains created with two different same-device queues;
- exact canonical claim, duplicate-claim rejection and selected-queue isolation;
- eight unclaimed offers, overflow and deliberately held observer mutex;
- successful DXGI returns despite overflow/contention, but terminal observation;
- observation cancellation followed by successful original creation;
- one installation attempt and terminal association retirement.

Each final probe reported `(forwarded, observed, claimed, dropped, creation_errors)`
as `(15, 10, 2, 2, 1)`. Pressure faults intentionally make observation unavailable;
those are not capture-success cases.

Factory-observed capture also passed full-ring continued Present, selected red
pixels with unrelated blue content excluded, retained outputs after producer exit,
minimize/restore, borderless/windowed transitions with three fresh changed-color
frames per phase, and 500-publication short soaks (two consumer stalls, ten pixel
checks) on both drivers. Constructors validate two intercepted/claimed creations;
native capture test separately checks forwarded/observed/claimed counters `(2,2,2)`.

## Replay and gates

Final full `scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode`
passed 42 safe feature-enabled library tests (two native owner tests ignored there
and explicitly run later), six safe recorder tests, three helper tests, both native
owner tests, 14 native hook tests, all other native suites and 32 native recorder
tests (21 WARP/11 hardware).

Factory-observed queue plus actual Present detour replay:
- Hardware: 836 ms, 18 NVIDIA H.264 GPU inputs, zero reported capture/encoder CPU
  readbacks; approved FFmpeg decoded every video frame, checking 17 red samples.
- WARP/software encoder: 840 ms, zero capture readbacks, 38 encoder CPU readbacks;
  every video frame decoded, 25 red samples checked.

Standalone targeted factory replay and all nine factory native tests (including
safe opt-in rejection) also passed before the final gate. Full workspace
`scripts/check.ps1`, strict dependency verification and standalone settings-save
regression passed. Existing optional MinHook 0.9.0/windows 0.62.2 only; no package,
license or production dependency added. No game/security settings changed.

## Remaining limitations

Observer installation still uses a known factory; fixture explicitly selects the
chain and initializes media outside Present. This is not independent loading/
bootstrap, discovery of pre-existing chains, all factory implementations/creation
variants, ResizeBuffers1 queue replacement or production resize/device rearm.
Contention testing deliberately holds a lock on the creation thread; arbitrary
multi-threaded third-party factory behavior is not established. Window-state tests
do not prove exclusive fullscreen, FSO/independent flip or real Alt-Tab. Short
soaks do not establish performance/endurance. Readback counters do not prove
zero-copy or vendor internals. No protected game was hooked; compatibility and
account safety remain unverified. VALORANT's exclusion is unchanged/respected.
Implementation remains uncommitted; no installer/release/push/PR produced.
