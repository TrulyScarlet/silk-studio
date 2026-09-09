# ADR 0007 - Media Foundation H.264 encoder backend

**Status:** Accepted for S3 implementation
**Date:** 2026-08-26
**Segment:** S3

## Context

S3 needs a redistributable H.264 encoder with hardware preference, capability
discovery, fallback, keyframe configuration, and overload telemetry. FFmpeg is
not approved for integration until its build flags and licensing are reviewed.
The S2 capture contract supplies BGRA D3D11 GPU handles through ADR 0006.

## Decision

1. Use the Windows Media Foundation H.264 MFT as the first native backend.
   The new `encoder-media-foundation` crate contains all Windows and COM
   bindings; `encoder-api` remains platform-neutral.
2. Enumerate synchronous/local MFTs and hardware MFTs with `MFTEnumEx`, rank
   compatible hardware candidates ahead of software candidates, and trial
   configure each candidate before selecting it.
3. Keep Media Foundation COM interfaces, samples, and buffers on the
   `silk-video-worker`. COM is initialized and Media Foundation is started by
   the factory on that worker; factory shutdown releases activation objects,
   transforms, Media Foundation, and COM in that order.
4. Configure H.264 output before NV12 input, set 8-bit progressive video,
   bitrate, profile, and maximum keyframe spacing, and emit encoded packets in
   the shared millisecond time base. Both media types explicitly signal BT.709
   matrix, BT.709 primaries, BT.709 transfer function, MPEG-2 progressive 4:2:0
   chroma siting, and limited/studio nominal range (16-235).
5. Accept CPU BGRA, RGBA, I420, and NV12 synthetic frames. WGC BGRA GPU
   surfaces are resolved into the shared D3D11 adapter and converted to an
   NV12 DXGI surface on the capture device before they reach the MFT. No CPU
   readback is introduced as a hidden fallback.
6. On an encode failure, reinitialize the active backend once, then try the
   next compatible candidate. A successful reinitialization or backend switch
   advances the encoder epoch and emits a fallback event; the engine publishes
   the latest bounded metrics snapshot.

## Consequences

- S3 has a real Windows H.264 MFT implementation without adding FFmpeg or GPL
  obligations.
- Synthetic CPU frames and native WGC GPU frames exercise the complete MFT path;
  WGC source dimensions are converted to the configured encoder dimensions on
  the same D3D11 device.
- Hardware MFT discovery and runtime behavior are platform-dependent and must
  be verified on actual NVIDIA, AMD, and Intel configurations before making a
  hardware-support claim.
- Deterministic shell helpers keep mocks by default. The persisted-configuration
  desktop path selects the Media Foundation fallback factory and native WGC
  capture on Windows; `SILK_REAL_CAPTURE=1` and `SILK_REAL_ENCODER=1` remain
  available for environment-controlled smoke tests.

## Verification

- Deterministic tests cover candidate ranking, configuration validation,
  runtime reinitialization/fallback, BT.709 limited-range BGRA/RGBA/I420-to-NV12
  conversion (including primary colors and a one-pixel 4:2:0 pattern), complete
  Media Foundation color attributes for NV12/H.264/HEVC/AV1 types, and engine
  worker context/configuration handoff.
- The ignored Windows tests were run and drove synthetic BGRA frames through
  the MFT at 64x64 and 1080p; they produced nonempty keyframed H.264 packets.
- Native hardware MFT selection and WGC GPU-surface encoding passed on the
  interactive AMD development machine. NVIDIA/Intel coverage, GOP/decode
  validation, device-swap recovery, and S2's one-hour soak remain deferred.
- The deterministic color tests establish the CPU conversion and media-type
  contracts only. Driver-specific VUI emission and decoded color values still
  require a newly encoded native clip to be inspected on each hardware family.
