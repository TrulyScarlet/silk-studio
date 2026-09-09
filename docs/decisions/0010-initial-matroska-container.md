# ADR 0010 - Initial Matroska container

**Status:** Accepted for S5
**Date:** 2026-08-26
**Segment:** S5

## Context

Silk already produces H.264 and raw AAC access units in immutable replay
snapshots. The first container must preserve those streams without re-encoding,
support separate audio tracks, and remain usable without an unapproved FFmpeg
dependency.

## Decision

1. Use Matroska (`.mkv`) as the first real container.
2. Implement the initial writer in the `muxer` crate using the project-owned
   container boundary and standard EBML elements; no new media dependency is
   added.
3. Write H.264 as `V_MPEG4/ISO/AVC`, converting Annex B access units to AVCC
   length-prefixed NAL units and deriving or preserving SPS/PPS codec private
   data.
4. Write raw AAC as `A_AAC`, preserving the stream's AAC
   `AudioSpecificConfig` as `CodecPrivate`.
5. Interleave packets by DTS, use millisecond cluster timecodes, stage to a
   `.part` file, validate the basic EBML/track/cluster structure, and publish
   with a same-volume rename.
6. Defer lossless MP4 remux until a separate implementation and licensing
   review are complete.

## Consequences

- Separate desktop and microphone audio tracks can be represented directly.
- The writer is independent of Media Foundation and does not block capture
  workers because the controller hands snapshots to a dedicated bounded save
  worker.
- H.264 encoders must provide Annex B or four-byte AVCC access units and enough
  SPS/PPS data to build the track configuration.
- This ADR does not complete external decoder validation. The current save
  worker is intentionally one worker per controller session; a pool and
  library index remain future work.
- Microsoft's [supported Media Foundation formats](https://learn.microsoft.com/en-us/windows/win32/medfound/supported-media-formats-in-media-foundation)
  list does not include Matroska as a native file container, so a Media
  Foundation Source Reader cannot be assumed to decode Silk's MKV captures.
  Post-save compression therefore needs a separately verified decoder and
  container path.

## Verification

- Deterministic muxer tests verify AVCC conversion, codec-private extraction,
  staged publication, track identifiers, cluster presence, normalization, and
  unsupported/missing codec metadata failures.
- Full `ffprobe` and decoder validation remains pending because the tool is not
  available on the validation machine.
