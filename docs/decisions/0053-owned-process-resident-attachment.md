# ADR 0053: Owned-process attachment with a resident Present forwarder

- Date: 2026-11-12
- Status: Accepted for default-off Windows x64 controlled testing only

## Scope and target approval

Remove renderer-assisted loading, swap-chain pointer handoff, owner-thread control,
and renderer pause from a separate owned-fixture route. The parent launches the
exact canonical fixture executable with explicit experimental opt-in, retains its
process handle, discovers its HWND by PID, and validates PID/creation time/HWND.
It has no API accepting arbitrary PIDs. Never select real games or protected/
unrelated processes as attachment targets. No elevation, bypass, stealth, manual mapping,
executable payload allocation, security-policy changes, or driver installation.

Use normal Windows LoadLibraryW via CreateRemoteThread only in that retained owned
child. Check native AMD64 architecture, revalidate identity immediately before
loading, restrict the DLL to the canonical build-directory sibling, and resolve
the actual system export's owning module/relative address instead of assuming
identical remote addresses. Use Toolhelp module snapshots to locate the loaded
helper and its versioned worker export. Remote allocations contain data only,
are bounded, and remain reserved on uncertain remote-thread completion. Failure
is unsupported/terminal; do not weaken Windows policy or try another process.

Reserve at most one remote attachment attempt per owned process after identity
validation. Never retry an uncertain load into the same process or allocate more
data buffers. Reclaim the worker's 128-byte bootstrap allocation only after its
thread is signaled complete; a received packet is not authorization to free it.

The helper has no custom DllMain. Start its worker explicitly only after ordinary
loader completion. The worker validates the fixed target packet and its own
fixture opt-in. Existing inherited private child pipes carry the bounded protocol;
the renderer neither loads the DLL nor exposes a swap-chain pointer nor handles
capture commands. This private-pipe fixture bootstrap is not a general-game IPC
or authenticated arbitrary-game attachment implementation.

## Resident hook lifecycle

The worker creates its own hidden D3D11 probe swap chain to obtain a Present entry.
MinHook installs the normal opt-in detour while the target keeps rendering; its
thread-suspension machinery is an implementation dependency, not proof of all
game/driver compatibility. The first selected-HWND Present publishes only a bounded
COM lease to a discovery queue. The worker prepares GPU textures outside Present,
rejecting single-threaded-device creation flags. No UI/disk/encoding or texture
allocation occurs inside the capture callback. Unrelated swap chains only forward.

Use global try-lock capture state rather than TLS owner-thread capture. Busy or
reentrant callbacks drop capture work, never wait for the controller/encoder.
Stopping closes capture admission; the worker retires capture state when possible,
but leaves the detour, trampoline and helper module resident until process exit.
Every subsequent Present still calls the original. Never disable/remove the hook
or FreeLibrary after installation. A timed-out drain retains state, not executable
memory reclamation under a caller. No restart/rearm within this resident session;
resize/loss invalidates media and requires a fresh controlled process for now.
Pin both the helper module and the original Present entry's module with the
official Windows module-pin API, so dropping other normal loader references cannot
invalidate either executable path while the forwarder remains installed.

The recorder adapter exposes this as explicit ResidentDll mode, reusing existing
bounded GPU ownership, liveness, replay and muxing. Cooperative rearm controls are
rejected in that mode. Its cleanup may terminate only the exact child it launched;
that fixture cleanup is not an arbitrary-game shutdown policy.

## Validation and limits

Test WARP and local hardware, selected/unrelated source isolation, actual shared
pixels, full-ring rendering progress, cancellation while rendering continues,
continued original Present after the worker exits, and owned process cleanup.
Keep prior cooperative DLL/recorder gates. No real game is automatically selected.
No fullscreen/Alt-Tab/anti-cheat/Valorant, safe arbitrary DLL unloading, physical
GPU loss, generic swap-chain discovery, or production distribution is claimed.
Driver calls, MinHook thread suspension and OS loading have no hard completion
guarantee. The existing interrupted-WARP-publication limitation remains.

Official platform references:
- [CreateRemoteThread](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createremotethread)
- [LoadLibraryExW](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-loadlibraryexw)
- [Toolhelp module snapshots](https://learn.microsoft.com/en-us/windows/win32/toolhelp/traversing-the-module-list)
- [CreateToolhelp32Snapshot](https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/nf-tlhelp32-createtoolhelp32snapshot)

Observed startup can race the target's module initialization. Retry only the
documented ERROR_BAD_LENGTH snapshot condition, at most 32 times with 10 ms spacing;
other errors are terminal. This retry is read-only module metadata collection,
not repeated remote loading or a security-policy fallback.
