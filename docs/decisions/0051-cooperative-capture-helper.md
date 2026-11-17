# ADR 0051: Cooperative DLL capture helper for the owned D3D11 fixture

- Date: 2026-11-11
- Status: Accepted for default-off development testing

Use a separate Rust cdylib with an explicit experimental-helper feature, Windows
x64 and D3D11 only. The selected owned fixture loads its sibling helper DLL with
LoadLibraryExW and restricted dependency search, after the controller verifies
PID, process creation time, and HWND against its retained child process handle.
Both host and helper require explicit fixture opt-in. No remote thread creation,
arbitrary-process injection, privilege changes, or anti-cheat interaction.

Expose one versioned C ABI using fixed-size byte packets, numeric operations and
status codes; no Rust ownership, allocations, COM wrappers or references cross
the ABI. A borrowed raw swap-chain pointer is accepted only on attach; the caller
guarantees validity and owner-thread quiescence. The helper independently verifies
the chain's window identity. Reject ABI/packet errors before attachment. Catch
Rust panics at the ABI boundary and close capture on failure; never unwind into
the host. Invalid pointers/access violations are not recoverable by this contract.

No work runs in DllMain. The DLL owns its tap, bounded captured-frame receiver,
and three-slot shared GPU transport. Rendering is independently paced on the
fixture's owner thread. Pipe IO runs on separate threads with bounded queues;
the rendering loop never waits on the controller or pipe IO. Pipe failure closes
capture while the fixture continues presenting for an observed bounded interval.
Explicit Stop detaches capture while the app continues; rearm requires a detached
helper, a fresh approved epoch, and no owned Present calls in flight. The host
may resize/recreate only its own graphics resources, then attach again. Old frame
ACKs/epochs are rejected. Never substitute a desktop or other source.

Because duplicated remote handles may survive Stop, permit at most three successful
attachments per loaded helper lifetime (nine exported surfaces cumulatively).
Refuse a fourth attachment; do not silently allocate more rings. This deliberately
restrictive fixture quota requires process relaunch, not production endless rearm.

Keep the helper loaded until process exit, including after Stop, to avoid unsafe
DLL unload with callback/TLS state. This intentionally does not claim arbitrary
game detachment/unload safety. No self-unload thread, loader-lock hook operations,
or production packaging. DLL path selection is restricted to the canonical build
directory sibling; this is not signed-binary trust or protection against a local
attacker who can modify the build directory.

Native gates exercise rejected identity, protocol/ABI errors, actual cross-process
pixels, bounded pressure, stop/restart, resize/device rearm, target exit, and
controller disappearance. This is cooperative loading, not game injection. Actual
driver removal, fullscreen, protected games, antivirus/signing, performance and
other-vendor acceptance remain outside this milestone.

Official loader reference: [LoadLibraryExW](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-loadlibraryexw).
Use only DLL_LOAD_DIR and SYSTEM32 search flags; never bypass code-authorization
checks. Loader errors are unsupported conditions, not grounds to weaken policy.

Observed WARP limitation: terminating the producer immediately after frame-header
delivery can interrupt queued GPU writes and leave partial pixels. A delivered
header/acquired shared mutex is not proof of post-crash pixel completion. The
retention crash test explicitly establishes completion by readback before killing
the owned producer. In-flight frames at process death are not promised valid;
production recovery must discard a dead source's pending frames rather than
presenting this fixture as proof of crash-safe publication.
