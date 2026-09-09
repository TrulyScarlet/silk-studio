# Segment S5 Progress Log - Muxing and replay saving

**Status:** In progress
**Started:** 2026-08-26
**Plan reference:** [implementation-plan.md](../implementation-plan.md) §9
**Design references:** [ADR 0009](../decisions/0009-muxer-snapshot-validation.md), [ADR 0010](../decisions/0010-initial-matroska-container.md)

Post-save share export is tracked under [ADR 0017](../decisions/0017-ffmpeg-post-save-export-toolchain.md). The verified-toolchain executor,
bounded worker, optional desktop runtime wiring, persisted enable/preset setting,
progress events, and cancellation command are implemented. Capture continues
with MKV-only output when the exact approved toolchain is unavailable.

## Task checklist

1. [x] Define a backend-independent snapshot validation boundary.
2. [x] Apply the validation boundary to the deterministic staging muxer.
3. [x] Choose and implement the first real container backend.
4. [x] Add a bounded asynchronous save worker.
5. [x] Add temporary-file staging and atomic publication for real output.
6. [x] Add disk-space checks and structured save metrics.
7. [x] Add output naming, sanitization, and collision policy.
8. [x] Validate combined audio/video clips with `ffprobe` and full decode using
   the approved toolchain and generated fixtures.
9. [x] Expose post-save export enablement, target-size presets, progress, and
   cooperative cancellation without moving work onto capture or UI threads.

## Implementation

- `crates/muxer/src/lib.rs` now validates the shared millisecond time base,
  unique stream IDs, descriptor/packet identity, positive durations, monotonic
  decode timestamps, positive audio format metadata, and a keyframed video
  start. Empty snapshots and audio-only snapshots are rejected.
- `crates/test-support/src/mocks.rs` uses the same validator before its staged
  placeholder write, so deterministic save tests exercise the S5 boundary.
- AAC initialization bytes remain in each audio `StreamDescriptor` through
  replay snapshots; a real container backend must write those bytes as codec
  private data.
- `crates/muxer/src/matroska.rs` writes H.264/AAC Matroska tracks without a
  media framework, converts H.264 Annex B access units to AVCC, interleaves
  packets by DTS, and validates the staged EBML structure before rename.
- The desktop shell now uses `MkvMuxer`; its deterministic mock video source
  emits Annex B-shaped packets so the shell save path exercises the real
  container writer.
- `crates/app-controller/src/jobs.rs` now runs one bounded `SaveWorker` per
  controller session. It owns the muxer and disk I/O, accepts immutable
  snapshots through a queue of eight jobs, and returns completion/failure
  results for controller polling. On Windows it enters background processing
  mode so synchronous staging, validation, and flush work stays below the
  capture/game scheduling path.
- `ControllerEvent::SaveQueued` reports accepted work immediately; the
  controller reserves output names before submission and maps worker results
  to `ClipSaved` or `SaveFailed`.
- The save worker checks free space with the Windows `GetDiskFreeSpaceExW`
  API before writing, retaining a 512 MiB safety reserve plus a bounded output
  estimate. `SaveMetrics` records queueing, completion/failure, mux failures,
  save duration, and the latest available space.
- Controller startup removes stale `.part` files without touching completed
  clips; cleanup errors are warnings and do not prevent capture startup.
- `crates/clip-export` accounts for every selected audio track when calculating
  the target-byte budget. `crates/encoder-ffmpeg` builds a direct MP4 argument
  vector with per-track audio mapping and a sibling `.part` output. Its gated
  executor runs encode/probe/decode work only on the export worker and never
  on capture or UI threads. Coarse preparing/encoding/probing/decoding/
  publishing progress is reported through the bounded queue state.
- The post-save export worker also enters Windows background processing mode;
  its external FFmpeg child starts below normal priority and requests process
  background mode so automatic MP4 encoding is less likely to compete with a
  running game.
- `configuration::OutputSettings` now controls whether post-save MP4 export is
  enabled and selects the small, medium, or large target-size budget. Existing
  v2 documents receive additive defaults without changing the config version.
- `Command::CancelExport` reaches the bounded worker cancellation token. Active
  jobs remain cooperative until the child process exits; queued jobs become
  terminal immediately. The desktop UI displays active jobs and a cancel action.

## Verification log

```text
cargo fmt --all -- --check                         -> passed
cargo test -p muxer -- --test-threads 1            -> 8 passed (includes staged-cleanup test)
cargo test -p test-support -- --test-threads 1     -> 2 passed
cargo clippy -p muxer -p test-support --all-targets \
  -- -D warnings                                   -> passed
cargo test -p integration-tests --test encoding \
  native_encoded_packets_write_an_mkv_snapshot \
  -- --ignored --test-threads 1                    -> 1 passed (Windows)
cargo test -p app-controller --lib                  -> 20 passed (disk/name/export/progress/cancellation coverage)
cargo test -p silk                                -> 2 passed
cargo test -p integration-tests --test e2e          -> 4 passed
npm run typecheck (apps/desktop)                    -> passed
npm run lint (apps/desktop)                         -> passed
scripts/check.ps1                                  -> All gates passed
scripts\validate-media.ps1 -ClipPath <clip> -FfmpegPath <path> -FfprobePath <path>
                                                   -> passed on generated H.264/AAC MKV and MP4 fixtures
scripts\verify-ffmpeg-toolchain.ps1 -FfmpegPath <path> -FfprobePath <path>
                                                    -> passed for the approved Gyan build
SILK_FFMPEG_PATH=<ffmpeg.exe> SILK_FFPROBE_PATH=<ffprobe.exe>
cargo test -p encoder-ffmpeg --test real_export \
    approved_toolchain_exports_a_generated_h264_aac_mkv -- --ignored --exact
                                                     -> passed through ExportWorker
SILK_FFMPEG_PATH=<ffmpeg.exe> SILK_FFPROBE_PATH=<ffprobe.exe>
  SILK_REAL_ENCODER=1 cargo test -p silk \
    shell_tests::real_shell_save_reaches_the_verified_export_worker \
    -- --ignored --test-threads=1
                                                     -> passed through shell save/export lifecycle
SILK_FFMPEG_PATH=<ffmpeg.exe> SILK_FFPROBE_PATH=<ffprobe.exe>
  SILK_REAL_AUDIO=1 SILK_REAL_ENCODER=1 cargo test -p silk \
    shell_tests::real_shell_save_reaches_the_verified_export_worker \
    -- --ignored --test-threads=1
                                                      -> passed with native WASAPI loopback audio and native MF video
SILK_REAL_CAPTURE=1 cargo test -p silk \
    shell_tests::command_handlers_drive_full_lifecycle_and_emit_events \
    -- --exact --test-threads=1                        -> passed with native WGC capture and mock video encoder
native WGC + native MF encoder shell run (`SILK_REAL_CAPTURE=1`,
`SILK_REAL_ENCODER=1`)
                                                       -> passed with 640x360 scaling, MKV save, and approved-pair MP4 export
Installed Gyan.FFmpeg 9.0.1 binary hashes: ffmpeg
57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A;
ffprobe AFE05347CAAABE479B3C4EAE71992B6EC1E11C57266A1D665DEB0F9FE9847208
```

## Known limitations

- `MockMuxer` writes a Silk test marker, not a playable media container.
- The default test audio source still emits synthetic payload bytes; only
  native Media Foundation AAC packets are expected to be decoder-valid.
- The current implementation uses one bounded save worker per controller;
  a multi-worker pool and library-index worker remain future work.
- A process crash can leave staged `.part` and `.part.lock` files until the next
  controller start; startup cleanup removes both, while crash-time recovery
  remains part of S8.
- `scripts\validate-media.ps1` now provides the required `ffprobe`/`ffmpeg`
  stream, metadata, timestamp, and full-decode checks. It passed against a
  generated 2.02-second H.264/AAC Matroska fixture and a 2.005-second MP4
  export using a temporary static build. The installed Gyan build is approved
  only as the exact recorded binary pair; its GPL and external-library
  obligations remain part of release packaging.
- The desktop shell now discovers and verifies the exact approved pair, then
  queues a separate sibling MP4 after a successful MKV save when the persisted
  setting is enabled. Fake-tool coverage exercises controller submission,
  progress, and cancellation; the real-media integration test covers the
  verified worker through publication; and the Windows shell smoke covers the
  Media Foundation video-to-MKV-to-export path, including native WASAPI audio
  when requested. Native WGC GPU-surface-to-MF encoding and
  the 640x360 shell save/export run pass locally; cross-vendor/release bundling
  evidence remains open.
