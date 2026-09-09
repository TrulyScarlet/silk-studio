# Segment S3 Progress Log - Video encoding

**Status:** Validated CPU/native-MF and local WGC GPU-surface paths; cross-vendor/decode checks deferred
**Started:** 2026-08-26
**Plan reference:** [implementation-plan.md](../implementation-plan.md) §7
**Design reference:** [ADR 0007](../decisions/0007-media-foundation-h264-encoder.md)

## Task checklist

1. [x] Extend `encoder-api` with configuration validation, capability ranking,
   fallback, epochs, events, and bounded metrics.
2. [x] Add the Windows Media Foundation H.264 MFT backend.
3. [x] Configure H.264 output/NV12 input and convert synthetic CPU frames.
4. [x] Wire worker-side GPU context installation and encoder configuration.
5. [x] Add deterministic ranking, fallback, and pixel-conversion tests.
6. [x] Record the backend, dependency, and threading decisions.
7. [x] Run the ignored native MFT synthetic-frame test on the Windows machine.
8. [x] Add same-device BGRA GPU-to-NV12 conversion for WGC frames.
9. [ ] Verify GOP cadence and decode the produced elementary stream on hardware.

## Implementation

- `crates/encoder-api/src/lib.rs` owns common settings validation, stable
  capability ranking, fallback orchestration, epoch changes, and metrics.
- `crates/encoder-media-foundation/src/lib.rs` owns COM/MF discovery,
  activation, H.264 MFT configuration, DXGI-backed GPU samples, sample/output
  processing, and CPU pixel conversion.
- `crates/gpu-windows/src/lib.rs` owns the shared D3D11 device wrapper,
  captured texture slot, and same-device video-processor conversion.
- `crates/recorder-engine/src/engine.rs` configures encoders on the video
  worker, installs the capture session GPU context, handles worker startup
  failure, drains delayed output, and publishes encoder telemetry.
- `apps/desktop/src-tauri/src/shell.rs` selects native capture, Media Foundation
  encoding, and native AAC for the persisted-configuration application path;
  deterministic shell tests remain mock-backed.

## Verification log

```text
cargo fmt --all                                      -> clean
cargo clippy -p encoder-api -p encoder-media-foundation \
  -p recorder-engine -p app-controller -p test-support \
  --all-targets -- -D warnings                       -> clean
cargo test --workspace                               -> passed
cargo test -p encoder-media-foundation               -> 2 passed, 2 ignored
cargo test -p encoder-media-foundation -- --ignored \
  --test-threads=1                                     -> 3 library tests + 2 WGC GPU integration tests passed
cargo test -p capture-windows --test native_display \
  gpu_video_processor_converts_wgc_frame_to_nv12 -- --ignored --nocapture
                                                         -> passed (local D3D11 converter)
cargo test -p encoder-media-foundation --test native_gpu \
  media_foundation_encodes_wgc_gpu_frame -- --ignored --nocapture
                                                         -> passed (local AMD hardware MFT)
cargo test -p integration-tests --test encoding \
  -- --test-threads 1                                  -> 2 passed
$env:SILK_REAL_ENCODER=1; cargo test -p silk \
  shell_tests::command_handlers_drive_full_lifecycle_and_emit_events \
  -- --test-threads 1                                  -> passed (native MF)
$env:SILK_REAL_CAPTURE=1; cargo test -p silk \
  shell_tests::command_handlers_drive_full_lifecycle_and_emit_events \
  -- --test-threads 1                                  -> passed (native WGC + mock encoder)
scripts/check.ps1                                       -> All gates passed
```

The native MFT tests were run on the interactive Windows development machine
and passed for synthetic audio plus CPU-backed BGRA streams at 64x64 and 1080p.
The native WGC converter probe and the combined WGC GPU-frame to Media
Foundation encoder probe also passed on the local AMD hardware MFT. The MFT
path includes the documented asynchronous unlock/event handling. This does not
verify NVIDIA/Intel behavior, GOP cadence, elementary-stream decoding, device
swap recovery, or long-run resource trends.
The existing ignored WGC tests and S2 hardware checks are not changed by this
segment.

## Known limitations

- The WGC-to-MF path is validated only on the local interactive AMD machine;
  hardware MFT availability, vendor ranking, overload behavior, GOP cadence,
  and elementary-stream decoding remain cross-machine checks.
- Format-change handling now drains and reconfigures the encoder, reattaches the
  current GPU context, and starts a fresh buffer epoch. The deterministic engine
  test covers the boundary; native physical resize/format transitions remain
  untested.
- `encoder-ffmpeg` contains a shell-free invocation builder, verified-toolchain
  checks, and a staged executor. The licensing/build review still gates any
  FFmpeg binary, library linkage, or application process execution.
