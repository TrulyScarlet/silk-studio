# ADR 0017 - FFmpeg post-save export toolchain review

**Status:** Accepted - exact Gyan GPL build approved
**Date:** 2026-08-28
**Segment:** S5/S9

## Context

Silk already has backend-neutral target-byte planning and a bounded export
queue in `crates/clip-export`, but it has no decoder or post-save executor.
The saved capture is Matroska containing H.264 and AAC access units. A
Discord-sized export needs to read that file, transcode or remux it, validate
the result, and publish it atomically without blocking capture.

The official [Media Foundation format list](https://learn.microsoft.com/en-us/windows/win32/medfound/supported-media-formats-in-media-foundation)
does not list Matroska as a native file container. A native Windows source
reader therefore cannot be assumed to decode the current input format.

## Behavioral reference

SteelSeries GG Moments was reviewed only through its public product and
support pages; no client binaries, source, assets, or implementation details
were copied. The pages describe these user-visible behaviors:

- Clips are kept locally in an editing-oriented folder and must be exported
  before they are expected to play correctly outside the application.
- A trimmer produces a separate playable result without destructively changing
  the source, and multiple audio tracks can be edited independently.
- Moments describes quality-based encoding that spends more bits on complex
  frames and fewer on simple frames, uses hardware encoding when available,
  and offers direct sharing destinations including Discord.
- The public CRF explanation describes a quality target that varies bitrate by
  scene rather than a fixed bitrate or guaranteed byte count.

Silk adopts the product lessons, not the implementation: keep the captured
source stable, make export produce a separate playable artifact, keep export
off capture/UI threads, preserve independent audio tracks, and treat sharing
as a later destination layer. Silk's first share preset remains deterministic
target-size planning because its current acceptance contract is based on byte
limits; a future quality-based preset can be added without changing the
capture path.

## Proposed boundary

1. Use `ffmpeg.exe` and `ffprobe.exe` as optional external processes behind
   the independent `encoder-ffmpeg`/`clip-export` boundary. Do not link FFmpeg
   libraries into Silk or pass FFmpeg types across the project interfaces.
2. Use the Gyan full Windows build of FFmpeg `9.0.1`. The corresponding
   [release archive](https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-full.7z)
   has published SHA-256
   `4b9c814cb07a1f90d05b768ef4eb2abbf89af94bbb924df5b7dbd6e64e1e2b96`, and
   the build identifies source commit
   [`bf1b838f2a`](https://github.com/FFmpeg/FFmpeg/commit/bf1b838f2a). The
   installed WinGet package is `Gyan.FFmpeg` version `9.0.1`.
3. This exact full build is approved as GPLv3. Its configuration may contain
   `--enable-gpl` and the external libraries recorded by Gyan; it must not
   contain `--enable-nonfree`. The required runtime capabilities are the
   Matroska demuxer, H.264/AAC decoders, MP4 muxer, native AAC encoder, and the
   Media Foundation H.264 encoder (`h264_mf`) where available.
4. Accept only the recorded binary pair. The verified Windows x86_64 hashes
   are `ffmpeg.exe` 
   `57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A` and
   `ffprobe.exe`
   `AFE05347CAAABE479B3C4EAE71992B6EC1E11C57266A1D665DEB0F9FE9847208`.
   Silk verifies both hashes before executing either tool, then runs
   `-version`, `-buildconf`, `-formats`, `-decoders`, `-encoders`, and
   `-muxers`. A missing capability, hash mismatch, or forbidden build flag
   must produce an unavailable-backend result, not a partial output.
5. Invoke the executable directly with an argument vector, never through a
   shell. Write to a sibling `.part` path, run `ffprobe` and a complete decode
   check, enforce the planned byte limit, and rename only a validated result.
   Cancellation must terminate the child process through the bounded export
   worker; capture and UI threads must not perform process or file I/O.
   Validation requires per-stream monotonic decode timestamps; a negative
   leading audio timestamp is allowed for codec priming.
   The desktop shell accepts explicit `SILK_FFMPEG_PATH`/
   `SILK_FFPROBE_PATH` overrides, bundled binaries beside the application, the
   approved Gyan WinGet layout, or matching tools on `PATH`; every discovered
   pair still passes the exact hash gate.

The first implementation slice is file export, not a Discord API client. It
will create a validated MP4 beside the source or in a user-selected export
directory. A later share adapter may hand that artifact to an OS or service
integration after account, network, privacy, and size-limit requirements are
defined.

## Compliance requirements

- The official [FFmpeg legal guidance](https://ffmpeg.org/legal.html) is the
  source for the LGPL/GPL distinction and the required build/source/notice
  review. Gyan identifies its Windows full builds as GPLv3; the optional GPL
  parts therefore apply GPL obligations to the full FFmpeg build. The build
  does not use `--enable-nonfree`.
- Record the exact source hash, signature verification, configure command,
  compiler/toolchain, linked libraries, `-buildconf` output, and binary hashes
  for every distributed Windows build.
- Ship the applicable GPLv3 text, notices for FFmpeg and included libraries,
  corresponding source or source offer, and the exact build configuration with
  any release artifact containing this build.
- `libx264`, `libx265`, and the other libraries present in this exact Gyan
  build are approved only as part of this recorded GPLv3 artifact. A different
  FFmpeg build or library set requires a new review.
- The Gyan build is an external process and is not linked into Silk. Release
  bundling remains subject to the source, notice, and distribution checklist.

## Open implementation gates

1. [x] Confirm the exact maintainer-approved FFmpeg source/build artifact;
   release bundling versus user-provided distribution remains a release task.
2. Verify `h264_mf` and the required Matroska/H.264/AAC/MP4 capabilities on
   the supported Windows release matrix.
3. [x] Extend the export request to account for every enabled audio track;
   `PlannerRequest::audio_bitrate_bps` is now per track and
   `audio_track_count` is included in the byte budget and invocation mapping.
4. [x] Add a gated executor that accepts only a `VerifiedFfmpegToolchain`.
     The executor owns staged output, FFprobe metadata checks, full decode, byte
     enforcement, cancellation, and atomic publication. The desktop shell now
     constructs the bounded worker when the exact pair is discoverable and
     verified; missing or rejected tools leave capture in MKV-only mode.

The repository command `scripts\verify-ffmpeg-toolchain.ps1` records SHA-256
hashes and runs the same exact-hash, capability, and build-policy verifier used
by the Rust boundary. It requires explicit paths to the approved `ffmpeg.exe`
and `ffprobe.exe`; it does not download a build.

## References

- [SteelSeries Moments](https://steelseries.com/gg/moments)
- [SteelSeries: Getting to know Moments](https://support.steelseries.com/hc/en-us/articles/24542109368973-Getting-to-know-Moments)
- [SteelSeries: What is Moments Capture Mode?](https://support.steelseries.com/hc/en-us/articles/34379253751309-What-is-Moments-Capture-Mode)
- [SteelSeries: What is CRF encoding?](https://support.steelseries.com/hc/en-us/articles/360052042551-What-is-CRF-encoding)
- [FFmpeg documentation](https://ffmpeg.org/ffmpeg.html)
- [ffprobe documentation](https://ffmpeg.org/ffprobe.html)
- [FFmpeg codec documentation](https://ffmpeg.org/ffmpeg-codecs.html#MediaFoundation)
- [FFmpeg release download and verification](https://ffmpeg.org/download.html)
- [Gyan Windows builds](https://www.gyan.dev/ffmpeg/builds/)
