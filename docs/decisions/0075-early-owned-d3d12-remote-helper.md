# ADR 0075: Early attachment of an owned D3D12 remote helper

- Date: 2026-11-13
- Status: Implemented and tested as default-off early owned fixture route

## Scope and loading

Add `d3d12-remote-helper-fixture` to the prototype and `d3d12-helper` to the sibling
DLL. Neither enters the normal desktop dependency graph. The remote loader's
private target sum type accepts only retained D3D11/D3D12 fixture Peer objects,
never arbitrary process identifiers. D3D12 requires a canonical exact fixture
launched in passive/external-helper mode. Verify native AMD64, pinned process
creation time/PID/HWND and module identity, allow one attempt, and load the
canonical sibling DLL using normal LoadLibraryW plus a relocated verified export.
Remote allocations are bounded data only and remain alive until thread completion
or target exit. No executable payload, unload, arbitrary game or elevation route.

The worker validates the exact passive process opt-ins before reading its unsafe
fixed-size bootstrap parameter. It pins its own DLL for resident detour lifetime,
copies/validates the selection packet and runs the independent D3D12 bootstrap.
Parent-facing worker exit status is collected explicitly in native acceptance.

## Early handshake and the ADR 0073 alternative

The passive owner creates/pumps two HWNDs but does not create a device/chain until
an exact initial selection arrives. It emits its initial session identity over the
already private inherited child pipe. The parent loads the worker with that pinned
selection. After installing both factory observers, the helper emits a fixed zero
Report readiness packet. Only then does the parent send the exact initial Select
to let the owner create chains. The owner joins its **single-packet** stdin reader
before creating them. The helper cannot begin transport stdin reads until it has
observed and claimed a subsequent chain, completed forwarding-only warmup, and
emitted GPU Hello. The next Select is media approval, read solely by the helper.

This deliberate phase ownership is the narrower implemented alternative to ADR
0073's proposed duplicated kernel control handles: no simultaneous stdin readers,
no publicly named rendezvous, no cross-module renderer pointer/counter handoff.
Neither process may send media approval ahead of GPU Hello. Wrong startup selection
is terminal; old/ambiguous/unobserved chains are not discovered retroactively.
Every queue is bounded and waits have deadlines. This is test IPC, not authenticated
shipping IPC. Do not reuse the phased inherited-pipe design for arbitrary targets.

## Media and stop

The helper obtains the actual selected chain and direct queue from its own factory
observations. ADR 0074 supplies twelve genuine forwarding-only Presents before
initial arming. Existing shared surfaces/fences, exact parent approval, ACK and
consumer leases apply unchanged. Remote mode rejects integrated bootstrap, resize,
device replacement, faults and extended lifetime flags.
Fixture window-action requests are unsupported and terminate rather than silently
manufacturing a response. Modern HWND creation was tested first; a subsequent
explicit passive legacy-factory mode also passes native WARP/hardware lifecycle
and saved/decode recorder tests. The executable's legacy creation opt-in allows
the passive-helper handshake without mistaking its separate Rust globals for the
DLL's observer state. It does not enable legacy creation for unrelated callers.

Native tests stop the helper and verify the renderer remains alive with retained
local pixels. The owned recorder adapter holds the remote worker for its session;
as with existing controlled adapters, its teardown owns/terminates its launched
fixture child. That behavior must never be presented as production process stop.

## Unfinished scope

Pre-existing-chain discovery, arbitrary/production IPC and loading, third-party
resize, in-flight device loss, game/fullscreen acceptance and protected-game
behavior remain separate. No security setting or capture exclusion is bypassed.
No new dependency/version/license: existing windows 0.62.2, MinHook 0.9.0 and
internal crates only. All hooks remain opt-in, exact owned fixture only.
