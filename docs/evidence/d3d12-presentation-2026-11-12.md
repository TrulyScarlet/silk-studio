# Owned D3D12 windowed presentation evidence

Date: 2026-11-12. Windows x64; WARP and DXGI hardware adapter 0. ADR 0055.
Default-off `d3d12-fixture`, no new dependencies, no desktop runtime integration.

## Scope versus previous offscreen slice

Two real HWND flip-discard swap chains now use the D3D12 capture device's known
DIRECT queue. Both have two fixed backbuffer render slots, allocator/list/RTV and
a separate renderer fence. The fixture renders selected red and unrelated blue.
Only selected GetCurrentBackBufferIndex enters capture. Renderer restores PRESENT;
capture copies PRESENT -> COPY_SOURCE -> PRESENT, then original Present is called.
The copy route never paints the source. No compositor/desktop pixels are sampled.

Capture reuses the bounded three-slot readback ring. It retains the actual source
resource lease per slot. CPU frame metadata records copy-submission Instant, not
the later poll/encode time. Renderer fences signal after capture/Present, covering
the entire queue iteration. Rendering skips busy renderer slots and capture drops
on occupied capture slots; neither submission path waits for a GPU fence on CPU.
Present success counts are calls, not proof of scanout or presentation mode.

The presentation object is !Send/!Sync to enforce HWND owner-thread operations.
Replay creates/steps/pumps/destroys it on a dedicated worker, with a three-frame
nonblocking channel. Capture timestamps use checked nanosecond-to-millisecond
timeline conversion, whose deterministic regression tests already exist.

## Native tests

On both WARP and hardware:

- Created exactly two owned fixture HWNDs at 318x178 backbuffer size; moved/raised
  the unrelated owned blue HWND over the selected red HWND. Imported selected
  pixels remained exactly red, never the unrelated blue source. This is owned-window
  occlusion evidence, not another process, Alt-Tab or a protected-game test.
- Held all three completed CPU frames. Both Present paths completed at least 12
  further calls, capture count stayed three, and full-ring drops were reported.
- Explicit cancellation rejected publication while both Present paths completed
  at least 12 more calls. Existing CPU bytes remained unchanged.
- ResizeBuffers changed selected buffers to 640x360 after cancel/drain and release
  of source leases AND recorded command-list references. Both chains continued
  presenting. Capture stayed terminal instead of publishing wrong-size data.
- A fresh 640x360 fixture resized to 800x450 with capture still active/unpolled.
  Resize terminated capture, allowed further rendering, and teardown succeeded.
- Retained red CPU frames remained valid after native teardown. Each fixture left
  zero HWNDs of its class in the test process. Fresh instance/in-flight-render
  teardown also completed normally; forced drain-timeout/device-removal not tested.

## Recorder tests

The selected swap-chain worker supplied actual CPU BGRA frames to RecorderEngine,
software Media Foundation H.264, replay buffering and direct MP4 muxing. No CPU
readback is hidden behind a GPU-only label. The existing offscreen cases remain
separate, as do all D3D11 hardware-encoder/GPU-transport cases.

Tests check 318x178 dimensions, 700–2000 ms duration, sample count, keyframe start,
monotonic sample timestamps and nonempty packets. Approved hash-pinned FFmpeg
decodes every saved frame and checks its downscaled expected red-color sample;
this is not equality testing every full-resolution decoded pixel.

The unread-consumer WARP test holds the adapter's three frame messages without
consuming them. Both selected/unrelated Present counts pass 20, readback count
stays three, and stop/idempotent stop cleanly exits the worker. Unsupported source
IDs/cadence and double-start are rejected without targeting another window.

Reproduction:

```powershell
.\scripts\test-game-capture-prototype.ps1 -Native -Hardware -Decode
```

## Final validation

- Full prototype native/hardware/decode gate passed: 26 feature-enabled library
  tests, four safe recorder tests, one safe dimensions test, three helper tests,
  existing D3D11 native scenarios, four D3D12 native cases and 21 native recorder
  cases (14 WARP/seven hardware). The D3D12 windowed replay clips were 866 ms WARP
  and 841 ms hardware, with 36 capture readbacks and 22 decoded expected-color
  samples checked in each. Both clips had zero encoder GPU-surface inputs.
- `scripts/check.ps1` passed formatting, workspace Clippy/tests, frontend typecheck,
  lint and build. Existing settings-save browser regression and strict dependency
  verification passed (545 registry packages / 11 direct frontend dependencies).

## Lifetime boundaries and remaining risks

Resize is a control-side operation with bounded fence polling; driver/API calls
may themselves stall. Resize failure is terminal. A resize is not automatic capture
rearm; only a fresh fixture establishes a new capture contract. CPU publications
remain valid, but there is no multi-epoch global consumer-memory guarantee.

On uncertain completion, the fixture keeps presentation/window/GPU resources and
marks the core's single-instance reservation terminal, preventing repeated native
allocations. This branch was not forced in hardware tests. Worker join and Windows
Present/destruction have no hard completion guarantee. Unused offscreen targets
remain part of the reused three-slot core's fixed memory budget in this first
presentation slice; it is not a minimum-copy or performance-optimized design.

Not implemented/tested: D3D12 Present detours, external queue discovery, multiple
queues, shared-texture/fence GPU-only transport, hardware encoder interop, arbitrary
game targeting, live capture rearm, HDR, other GPU vendors/multi-GPU, fullscreen
mode transitions, Alt-Tab/minimize, physical device loss, stress/soak performance,
anti-cheat acceptance, signing/antivirus or account safety. Microsoft's current
D3D12 swap-chain documentation distinguishes FSO from classic exclusive fullscreen;
no fullscreen claim follows from this explicitly windowed test.

Next milestone is bounded D3D12 GPU publication/shared-fence ownership and encoder
interop in owned fixtures, before any broader hook/game integration. No game was
attached, external security setting changed, installer published or commit made.
