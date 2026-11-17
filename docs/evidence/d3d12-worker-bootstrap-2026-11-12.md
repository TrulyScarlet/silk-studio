# Independent in-process owned D3D12 worker bootstrap

Date: 2026-11-12. ADR 0065. Default-off, exact launched owned child only.

## Changed boundary

`d3d12_bootstrap.rs` starts a capture worker before the renderer creates its
chains. Worker independently constructs a factory and installs the resident
observer. Renderer uses its own direct DXGI creation path, does not register/claim
associations or initialize media, and sends only PID/process-creation-time/HWND
on a one-item channel. No queue/device/chain/resource/descriptor pointer handoff.

Worker claims the unique observed exact HWND and validates current process
lifetime, actual chain HWND, DIRECT queue and canonical device relationship.
It waits for at least twelve completed renderer Present iterations before creating
media/exporting handles/installing the Present hook. Renderer continues its own
Present/message loop during installation, transport, parent stalls and cancellation.
MinHook's thread management is not a zero-suspension guarantee.

The selected association stays fixed; eight pending factory offers plus one adopted
association are bounded. Window actions use a two-item owner-thread mailbox and
one-item replies. Parent transport retains existing fixed packets, bounded queues,
shared textures/fences and exact approval/ACKs. Startup/error/cancel paths join
the worker before releasing renderer window guards. Source ID:
`owned-d3d12-independent-bootstrap`. Flags include existing owned/Present/factory
opt-ins plus `--bootstrap`.

## Tests actually run

Native WARP and NVIDIA GeForce RTX 2080 Ti tests passed:
- two renderer creations observed, one exact HWND claimed by worker;
- pre-capture renderer progress at least twelve Presents;
- three-frame no-ACK pressure while selected/unrelated Present keeps progressing;
- selected red pixels, unrelated content excluded;
- minimize/restore and observed borderless/windowed states, three fresh
  post-action-QPC frames with expected changed colors per phase;
- capture cancellation followed by twelve further renderer Presents;
- 500-publication short transport soak per driver, two consumer stalls and ten
  explicit pixel checks; no performance/endurance inference;
- WARP startup stale process lifetime and incorrect parent session approval fail;
- deliberate bootstrap producer exit rejects queued recorder events and retires
  GPU context. Targeted rerun also asserts queued events were actually discarded.

Safe tests reject bootstrap in the test executable before graphics/thread startup,
and reject invalid target metadata without defaulting to foreground/monitor.

Full `scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode` passed:
44 safe feature-enabled library tests, six safe recorder tests, three helper tests,
two explicitly run native owner tests, 14 hook tests, five bootstrap native tests,
other native suites and 35 native recorder tests (23 WARP/12 hardware).

Final independent-worker replay/decode observations:
- Hardware: 937 ms, 11 NVIDIA H.264 GPU inputs, zero reported capture/encoder CPU
  readbacks; approved FFmpeg decoded every video frame, checking ten red samples.
- WARP/software encoder: 837 ms, zero capture readbacks, 35 encoder readbacks;
  every video frame decoded, 22 red samples checked.

Workspace check passed formatting, Clippy/tests and frontend typecheck/lint/build.
Strict dependency verification and standalone settings-save regression passed.
No new package/version/license; existing optional MinHook/windows/internal APIs.
No owned fixture processes remained after validation; no game/security changes.

## Still unimplemented / not established

This is owned **in-process startup** bootstrap, not a remote DLL loader or discovery
of chains created before observation started. Renderer and worker both remain in
the exact fixture process, with controlled window lifetimes. Third-party factory
implementations, all creation variants, arbitrary COM/apartment/lifecycle behavior,
actual exclusive fullscreen, FSO/independent flip, real Alt-Tab, game compatibility
and account safety are unverified.

Live resize/device-loss rearm is **not implemented**. Next work must include a
bounded epoch/handle handshake, exact old-epoch event/ACK rejection, source-resource
release before ResizeBuffers, safe drain/device-loss handling and GPU-context
replacement ordered with FormatChanged. RecorderEngine already refreshes the GPU
context on format changes; CrossCapture still needs a correct transition instead
of its single startup context. Fresh process/renderer recreation is not live rearm.

Render-only mode retains unused baseline offscreen/readback allocations; no minimal
memory or zero-copy claim. Readback counters do not establish vendor internals.
All changes remain uncommitted; no release/installer/push/PR produced.
