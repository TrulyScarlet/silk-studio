# Segment S4 Progress Log - Audio capture and synchronization

**Status:** In progress
**Started:** 2026-08-26
**Plan reference:** [implementation-plan.md](../implementation-plan.md) §8
**Design reference:** [ADR 0008](../decisions/0008-audio-capture-synchronization.md)

## Task checklist

1. [x] Define the platform-neutral audio capture contract and error mapping.
2. [x] Add deterministic audio conversion and shared-timeline synchronization.
3. [x] Add Windows WASAPI loopback and microphone backend scaffolding.
4. [x] Enumerate active endpoints and select default or requested devices.
5. [x] Read event-driven WASAPI packets with native timestamps and silent-buffer
   handling.
6. [x] Add format-validation tests that do not require a physical audio device.
7. [x] Add one worker for each enabled audio source sharing the bounded replay
   buffer.
8. [x] Wire audio encoders and distinct stream IDs into the recorder engine.
9. [x] Preserve loopback when the optional microphone reports device loss.
10. [x] Add bounded device-loss recovery and client recreation.
11. [x] Add the native Media Foundation AAC encoder and AudioSpecificConfig;
    the persisted-configuration shell path now selects it on Windows.
12. [x] Register endpoint notifications and map selected/default endpoint
    changes to bounded recovery events.
13. [ ] Validate device-swap behavior on hardware.
14. [ ] Validate combined clips, stream/sample-rate metadata, and A/V drift.

## Implementation

- `crates/audio-api/src/lib.rs` owns `AudioCapture`, typed capture errors,
  `AudioCaptureEvent`, `AudioCaptureConfig`, and discovered `AudioFormat`.
- `crates/audio-sync/src/lib.rs` owns source timestamp calibration, format
  validation, S16/F32 decoding, channel conversion, linear resampling, gain,
  monotonic output timestamps, and bounded synchronization metrics.
- `crates/audio-wasapi/src/native.rs` owns Windows COM/WASAPI endpoint
  enumeration, shared-mode event capture, packet copying, QPC/device timestamp
  selection, silent packets, device invalidation mapping, and the
  `IMMNotificationClient` callback that wakes the capture event without doing
  COM or media work.
- `crates/audio-wasapi/src/unsupported.rs` provides an explicit unavailable
  backend outside Windows so deterministic workspace tests remain portable.
- `crates/recorder-engine/src/engine.rs` starts one audio worker per injected
  source, runs synchronization before encoding, stores per-stream metrics, and
  retries invalidated sources with bounded client recreation.
- `crates/encoder-media-foundation/src/audio.rs` owns the worker-local Media
  Foundation AAC MFT, converts synchronized F32 to the required interleaved
  S16 PCM, emits raw AAC access units, and publishes AAC-LC
  AudioSpecificConfig metadata.
- `crates/encoder-api/src/lib.rs` exposes optional audio codec initialization
  bytes; `crates/recorder-engine/src/engine.rs` publishes them into the
  replay-buffer stream descriptor before packet ingestion.
- `apps/desktop/src-tauri/src/shell.rs` keeps scripted desktop/microphone
  sources for deterministic helpers and selects native loopback/microphone plus
  AAC for the persisted-configuration application path on Windows.
- `crates/test-support/src/scripted.rs` provides deterministic scripted audio
  chunks and device-loss events for engine integration tests.

## Verification log

```text
cargo fmt --all -- --check                         -> passed
cargo check -p audio-wasapi                       -> passed (Windows bindings)
cargo test -p audio-sync -- --test-threads 1      -> 5 passed
cargo test -p audio-wasapi -- --test-threads 1    -> 6 passed (Windows format
                                                        and notification tests)
cargo test -p audio-wasapi -- --ignored \
  --test-threads 1                                -> 1 passed (native loopback
                                                       endpoint on this machine)
cargo clippy -p audio-wasapi --all-targets \
  -- -D warnings                                   -> passed after API cleanup
cargo test -p integration-tests --test encoding \
  -- --test-threads 1                              -> 6 passed (audio worker/recovery tests)
cargo test -p encoder-media-foundation -- \
  --test-threads 1                                  -> 7 passed (AAC contract tests)
cargo test -p encoder-media-foundation -- --ignored \
  --test-threads 1                                  -> 3 passed (native AAC/H.264 MFTs)
cargo test -p silk -- \
  --test-threads 1                                  -> 2 passed (shell audio path)
 $env:SILK_REAL_AUDIO="1"; cargo test -p silk -- \
  shell_tests::command_handlers_drive_full_lifecycle_and_emit_events \
  -- --test-threads 1                              -> passed (native loopback through shell)
scripts/check.ps1                                  -> All gates passed (workspace,
                                                       frontend, and integration)
```

The integration tests validate worker ownership, separate stream insertion,
and microphone-loss isolation with scripted sources. The ignored loopback test
and native shell test also produced packets from a physical endpoint on this
Windows machine; no manual device-swap test has been run.

## Known limitations

- The legacy controller factory remains available for callers without audio;
  the desktop shell now injects both scripted sources by default. Production
  device selection is not yet surfaced through the UI.
- The native backend registers endpoint notifications and wakes the capture
  worker for selected/default endpoint changes; the engine still performs the
  bounded restart attempts.
- Physical endpoint swap, sleep/resume, and long-duration drift validation have
  not been run on hardware.
- The native backend accepts the common 16-bit integer and 32-bit float mix
  formats; other endpoint formats are rejected.
- Native AAC emits raw access units and now carries its AAC-LC
  AudioSpecificConfig through snapshots; real-container muxing is pending S5.
- Separate audio tracks in a real container, combined-clip validation, and
  measured A/V drift remain pending S4/S5 integration.
