# ADR 0046: Controlled D3D11 Present-hook prototype

- Date: 2026-11-11
- Status: Accepted for a development-only prototype, not production capture

The cooperative transport extension is documented separately in ADR 0047; this
record describes the initial in-process hook milestone and its restrictions.

## Design and first implementation boundary

Start with x64 Windows and D3D11/DXGI `IDXGISwapChain::Present` in an original,
single-render-thread test application owned by this repository. The application
explicitly opts in with a command-line flag. Hook its own Present implementation
using the permissively licensed MinHook wrapper; do not load code into games yet.
This exercises a real function detour and backbuffer capture without an injector,
anti-cheat interaction, or a claim of game compatibility.

The selected identity is PID, process creation FILETIME, and HWND. The native
prototype rejects a target outside its own process and validates window ownership
at attach. Capture only the approved swap chain on the attach/render thread;
other calls forward to the original Present without capture. One tap per process
is supported. Attach/detach must occur with other Present callers quiesced; this
restriction prevents unsafe trampoline teardown and is suitable only for the
controlled fixture, not arbitrary game processes.

Allocate exactly three BGRA8 textures before enabling the detour. Each published
frame owns an Arc lease; a texture is reusable only when no consumer retains it.
Use a fixed-capacity event queue and try-send/try-borrow in the callback. Drop
capture work when busy; always call the original Present. No disk, logging,
encoding, texture allocation, or waits for the consumer inside the callback.
Driver-internal GPU command submission latency is not asserted to be bounded.

Use the game's D3D11 device/context for GPU copies with D3D11 multithread
protection, then expose the existing GpuTextureSlot type for downstream validation.
Check size, format, device, target HWND, and sample count. Resize/device mismatch
invalidates the current tap instead of copying into incompatible resources.
Reattachment outside Present is required; no automatic monitor fallback exists.
Frame sequences and QPC timestamps are validated against a session/epoch identity.
Track retired texture leases outside Present and reject reattachment when more
than three old surfaces remain live: at most six live leased/capture surfaces
across an active ring and retired epochs. Weak leases prevent ring reuse too.

This first transport is in-process only. The future cross-process protocol will
use unnamed NT shared texture handles duplicated through an authenticated,
explicitly approved helper session, with adapter LUID checks and fixed ownership
slots. Do not use guessable global names or legacy DXGI shared handles. A process
identity change, consumer disconnect, or epoch change invalidates old frame
metadata. The portable contract and deterministic lease/session tests land before
cross-process transport is connected.

## Dependencies and release boundary

`minhook` 0.9.0 is MIT licensed and bundles the BSD-2-Clause MinHook C library
(including its BSD-licensed HDE disassembler); record the shipped versions and
license text in the dependency inventory before enabling the feature.
Use only this library's implementation, not OBS code or third-party game-capture
samples. Pin the dependency. The feature is off by default; the fixture binary
requires `experimental-hooks` and is not an installer/updater artifact.

## Exit evidence and remaining scope

The controlled fixture must prove that calls reach the detour, captured pixels
match rendered pixels, retained leases are not overwritten, and full queues do
not stop Present. Record whether hardware or WARP was used. Fullscreen/game,
anti-cheat, cross-process transport, DLL/helper lifecycle, and production settings
integration are later gates, not consequences of this prototype compiling.
