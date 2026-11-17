# Controlled hook through RecorderEngine: replay evidence

Date: 2026-11-11. Design: ADR 0048. This extends the earlier hook and transport
fixtures; it is not game compatibility or production deployment evidence.

GPU-mode results are recorded separately in `gpu-hook-replay-2026-11-11.md`.
The limits below describe this original CPU-mode gate, not the later GPU work.

## Environment actually tested

- Windows x64 10.0.19045/build 19045, as in the prior native fixture runs.
- WARP/Microsoft Basic Render Driver with Windows software H.264 encoding.
- NVIDIA GeForce RTX 2080 Ti with HardwareOnly Media Foundation encoding.
- Reported encoder backends: `H264 Encoder MFT` (software) and
  `NVIDIA H.264 Encoder MFT` (hardware).
- Original D3D11 fixture at 640x480 BGRA8, 30 FPS target, debug builds.
- No actual game, desktop source, installed recorder, or user's clip was used.

## Command

```powershell
.\scripts\test-game-capture-prototype.ps1 -Native -Hardware -Decode
```

`-Decode` requires the exact already-approved FFmpeg executable. The script
checks its SHA-256 and the native test rechecks it by bounded streaming before
execution. No binary is downloaded or installed. Default decoder path is
`%LOCALAPPDATA%\Silk\tools\ffmpeg\bin\ffmpeg.exe`; an explicit
`SILK_HOOK_TEST_FFMPEG` may override the path, not the approved hash.

## Observed results

- Both real replay saves passed through the actual Present detour, cooperative
  NT-handle GPU transport, VideoCapture adapter, Media Foundation encoder,
  ReplayBuffer, and Mp4Muxer.
- The adapter uses immutable BGRA CPU frames read back on its capture worker.
  There is no UI readback, shared-resource ownership held by the encoder, or
  claim of zero-copy recording.
- Saved files parsed as one video track at exactly 640x480 (4:3), with at least
  ten nonempty samples, a keyframe at the replay start, monotonic sample decode
  times, and at least 750 ms of video. Observed representative saves were about
  851–864 ms; size/duration depend on startup timing.
- The approved FFmpeg binary decoded every video frame without errors. A
  separately decoded/downsampled RGB pixel had one visible primary channel,
  proving the clip was not blank/black; RGB dimensions came from MP4 metadata.
- With the frame consumer deliberately idle, the three-event queue filled and
  capture dropped additional CPU frames instead of blocking on the consumer.
- Repeated start/stop and idempotent stop succeeded. Worker-exit counters were
  observed after shutdown. A deliberately killed producer produced a terminal
  capture failure after exactly three copied frames, not desktop fallback.
- Invalid source/cadence and missing executable paths were rejected; repeated
  failed starts left no live worker before retry. No UI reopen was required.
- Portable QPC tests cover fractional conversion without accumulated drift,
  invalid frequency, backward values, equal timestamps, and overflow rejection
  without advancing accepted state.

## Validation gate results

- Twelve portable prototype unit tests and two safe adapter tests passed.
- All four explicitly enabled native adapter/replay tests passed, as did the
  earlier in-process hook and cross-process transport fixtures on WARP/hardware.
- `scripts/check.ps1` passed workspace formatting, Clippy, tests, frontend
  typecheck/lint/build. Unrelated native tests remained ignored.
- Strict dependency verification and the existing isolated-browser settings
  source-save regression passed. No new third-party package was resolved.

## Limits

- This is a controlled CPU-input recording adapter, not an injected game backend.
  The three-surface GPU-only recorder bridge and cross-thread lease lifecycle
  still need implementation and validation.
- No cross-process resize/rearm, arbitrary Present-thread concurrency, game audio
  synchronization, exclusive fullscreen, Alt-Tab, minimize, Valorant/anti-cheat,
  AMD/Intel, multi-GPU, D3D12/Vulkan/OpenGL/HDR, or antivirus acceptance was tested.
- Synthetic capture is consumer-commanded; the production game renderer must
  remain independent of IPC/readback/encoding. The Present callback itself still
  performs no pipe I/O or readback.
- Reader deadlines/cancellation are bounded, but D3D11 driver calls and process
  termination are not claimed to have hard real-time latency bounds.
- Generated media lived only in fresh `Temp\opencode\silk-hook-replay-*`
  directories and was removed by the test after validation. No release was made.
