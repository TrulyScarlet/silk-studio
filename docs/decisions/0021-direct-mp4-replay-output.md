# ADR 0021 - Direct MP4 replay output

**Status:** Accepted
**Date:** 2026-08-29

## Context

Silk previously captured replay snapshots to Matroska (`.mkv`) files and relied
on optional external FFmpeg post-save execution to produce `.mp4` artifacts.
While this preserved captured streams without an in-process MP4 dependency, it
created user-facing friction: common Windows media players, editors, and web
sharing targets natively expect standard MP4 containers. Furthermore, post-save
re-encoding or remuxing introduces CPU/GPU scheduling contention during gameplay
and risks generational quality loss when transcoding.

An instant-replay clipping tool requires a direct, loss-free, single-pass path to
author standard MP4 files immediately upon saving a replay snapshot, without
blocking real-time capture threads or degrading video fidelity.

## Decision

1. **Direct MP4 Muxing**: Mux normal replay snapshots directly to standard ISO
   Base Media File Format (`.mp4`) using the pure-Rust `mp4` crate (version
   `0.14.0`). No intermediate Matroska file is written, and no secondary
   re-encoding or remuxing pass is performed for standard replay saves.
2. **Supported Direct Codec Set**: Direct MP4 muxing supports H.264 (AVC) video
   and AAC audio (`AacConfig` / `AudioSpecificConfig`). Video access units are
   converted from Annex B format to length-prefixed AVCC samples with SPS and PPS
   parameter sets embedded in the track's `avcC` configuration box.
3. **Media Foundation Bitrate Delivery**: The user-configured video bitrate
   (bounded between 2,500 kbps and 100,000 kbps) or auto-calculated preset bitrate
   passes directly to the native Media Foundation H.264 encoder attribute
   (`MF_MT_AVG_BITRATE`). Bitrate enforcement and driver rate-control behavior
   require hardware validation across the release matrix.
4. **Preservation of Original Quality**: Direct single-pass MP4 save eliminates
   automatic post-save export or secondary re-encoding, preserving the exact
   bitstream fidelity produced by the real-time encoder without generational
   transcoding loss.
5. **Separate Multi-Track Audio**: Multiple audio streams (e.g., desktop audio
   and microphone) are written as separate, independent AAC audio tracks within
   the MP4 container rather than mixed into a single downmixed track.
6. **Staged Publication and Validation**: Replay snapshots are written to a
   sibling `.part` file. Before publishing, the muxer validates the staged
   artifact (verifying `ftyp`, `moov`, track definitions, sample tables, and
   non-decreasing decode timestamps). Validated files are published via an
   atomic same-volume rename. A failure guard automatically removes unvalidated
   or aborted `.part` files.
7. **Box Ordering and Local Playback**: Top-level boxes are written sequentially
   as `ftyp -> mdat -> moov`, placing the movie metadata box (`moov`) at the end
   of the file. This supports single-pass streaming disk I/O without speculative
   box pre-allocation or a secondary file rewrite pass. Local Windows decoders
   and media players seek to the trailing `moov` box in O(1) time without network
   buffering. Silk makes no FastStart streaming layout claims.
8. **Configuration Migration (v4)**: Configuration schema v4 removes obsolete
   container selection, remuxing, and post-save export fields (`remuxToMp4`,
   `postSaveExport`, `postSaveExportPreset`), normalizing to direct MP4 output
   while preserving all user-configured bitrates, resolutions, frame rates, base
   directories, and audio device selections.
9. **Future Codec Scope**: Direct MP4 encapsulation for HEVC (H.265) and AV1 is
   deferred until a separately reviewed MP4 writer and parameter-set parser are
   implemented.
10. **Hardware and Player Scope**: Third-party player and hardware playback
    compatibility are documented as supported targets but not claimed as verified
    until tested on the release test matrix.

## Alternatives Considered

- **Intermediate MKV + background FFmpeg remux/transcode**: Preserves the
  original MKV engine but requires external process execution, increases disk
  I/O, risks background CPU/GPU contention during gameplay, and requires the
  approved external FFmpeg toolchain for standard clip generation.
- **Windows Media Foundation Sink Writer**: Native Windows API for MP4 muxing,
  but introduces tight coupling to Windows COM APIs, complicates bounded
  cross-platform snapshot testing, and makes custom multi-track audio staging
  less flexible.
- **FastStart MP4 generation (`qt-faststart` / leading `moov`)**: Requires either
  two full disk passes (copying entire media payload) or speculative `moov` space
  reservation. This adds disk overhead during gameplay for progressive web
  streaming benefits that are unnecessary for local clip files.
- **In-tree custom MP4 box serializer**: Writing a custom MP4 muxer from scratch
  adds maintenance complexity and risks ISO/IEC 14496-12/14 compliance issues
  compared to using the established pure-Rust `mp4` crate.

## Consequences

- Replay clips are immediately saved as standard, widely compatible `.mp4` files
  without background transcoding delay or quality degradation.
- Direct MP4 generation operates entirely in pure Rust behind the project's
  `Muxer` trait, enabling deterministic unit and integration tests.
- Real-time video bitrate configuration is delivered via `MF_MT_AVG_BITRATE`
  to the Media Foundation encoder (hardware rate control requires matrix testing).
- Multi-track audio capture is preserved for editing applications supporting
  multi-track MP4 files.
- Staged `.part` writes protect the clip library from partial or corrupted files
  during crashes or cancellations.
- Direct MP4 output is currently restricted to H.264/AAC; other codecs or
  containers require dedicated writer implementations.

## Verification

- Deterministic unit tests in `crates/muxer` verify H.264 AVCC transformation,
  single/multi-track AAC layout, timestamp monotonicity, error handling, staged
  guards, and structural box validation.
- Migration tests in `crates/configuration` verify schema upgrades, field
  normalization, and bitrate preservation.
- Full hardware encoder bitrate enforcement and external player playback
  verification remain governed by device smoke tests and the release preflight
  checklist.
