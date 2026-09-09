# ADR 0009 - Muxer snapshot validation boundary

**Status:** Accepted for S5 groundwork
**Date:** 2026-08-26
**Segment:** S5

## Context

The replay buffer produces immutable snapshots, but a muxer must remain an
independent trust boundary. A future container backend must not write a corrupt
or undecodable prefix merely because a recovery or test producer supplied a
malformed snapshot. S5 also needs to preserve audio stream descriptors,
including AAC codec initialization bytes, without coupling the contract to a
specific container library.

## Decision

1. Keep backend-independent snapshot validation in `muxer` and expose it as
   `validate_snapshot`. It checks the shared millisecond time base, unique
   stream IDs, descriptor/packet identity, positive packet durations,
   per-stream monotonic DTS, and a keyframe at the beginning of the writable
   video stream.
2. Require positive sample rate and channel metadata for writable audio
   streams. Descriptor `extradata` is opaque to this layer and is preserved for
   the eventual container writer.
3. Treat empty snapshots and snapshots without writable video as validation
   failures. Empty optional tracks may be omitted by a backend, but a clip
   cannot be published without video.
4. Leave timestamp normalization and interleaving to the eventual backend at
   the mux boundary. The validator accepts non-zero origins so it can validate
   a snapshot before a backend clones and normalizes it.
5. Have the deterministic `MockMuxer` call the same validator before staging;
   it remains a contract test double and is not claimed to produce a playable
   container.

## Consequences

- Real container implementations inherit one stable set of input invariants.
- Malformed packets fail before output I/O, while codec-specific validation
  remains the responsibility of the selected container/decoder backend.
- No new media or container dependency is added by this groundwork.
- The MKV versus fragmented-MP4 choice, real mux implementation, async save
  pool, and `ffprobe` validation remain open S5 work.

## Verification

- `muxer` unit tests cover accepted audio/video metadata, empty/audio-only
  snapshots, descriptor/packet mismatches, non-monotonic DTS, non-keyframe
  starts, and invalid audio parameters.
- `test-support` continues to pass its deterministic staging tests using the
  shared validator.
