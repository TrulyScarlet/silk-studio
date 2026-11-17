# ADR 0059: Owned D3D12 Present detour with explicit queue ownership

- Date: 2026-11-12
- Status: Accepted for default-off owned-fixture child testing only

After cross-process D3D12 recorder validation, replace the fixture's manual
pre-Present capture call with an original Present detour in a separate feature.
Only the exact `silk-d3d12-owned-fixture.exe` process with explicit owned/hook CLI
opt-in can install it. No remote loading, arbitrary PID, injection or game target.

The fixture still supplies its exact selected swap chain and known DIRECT queue.
This is not generic queue discovery or renderer-independent helper bootstrap.
Selected Present obtains the current owned backbuffer and submits the bounded GPU
copy; unrelated chains/TEST calls only forward. Global state uses try-lock in the
callback; contention drops capture, never waits for controller/UI/encoder/pipe work.
No texture allocation, Map/readback, disk, pipe or UI work occurs in the healthy
callback. Submit failure closes admission without locking the output publication
pool; the controller retires that pool outside Present.

Keep original entry's module pinned. MinHook installs the forwarder using its own
thread management; no renderer-assisted pause is assumed, but no zero-suspension
claim. Store original trampoline before enabling. One installation attempt per
process; never disable/remove/unload it. Cancel capture and retire/drain media,
but leave executable forwarding resident until process exit. After cancellation,
selected and unrelated Present must continue calling original code.

Use ADR 0057's same owned-child protocol and ADR 0058's recorder adapter. Distinct
test source/launch option prevents relabeling manual-boundary evidence as detour
evidence. Test actual callback counters, selected-source GPU pixels, full rings,
cancellation, cross-process replay and approved decode on WARP/hardware.

No D3D12 graphics queue discovery, third-party game lifecycle, safe arbitrary DLL
unload, fullscreen/Alt-Tab/minimize, anti-cheat compatibility or release follows.
Uses already reviewed optional MinHook 0.9.0 and existing Windows bindings only.
