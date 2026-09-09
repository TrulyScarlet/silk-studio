# ADR 0018 - Same-device WGC GPU-to-NV12 conversion

**Status:** Accepted for local S3 validation  
**Date:** 2026-08-28  
**Segment:** S2/S3 integration

## Context

Windows Graphics Capture publishes BGRA8 D3D11 textures owned by the capture
device. The Media Foundation H.264 input contract is NV12, and a CPU readback
would add a blocking copy to the real-time path. The adapter must preserve the
bounded GPU-resource lifetime from ADR 0006 and work with hardware MFTs that
use the D3D11 device manager.

## Decision

1. Add the Windows-only `gpu-windows` crate as the shared home for the D3D11
   device/context wrapper, captured texture slot, and `D3D11VideoConverter`.
2. Create the capture device with BGRA and video support, enable
   `ID3D11Multithread` protection, and use the same device for WGC staging,
   video-processor conversion, and Media Foundation device-manager setup.
3. Use the D3D11 video processor to convert BGRA8 to a configured-size NV12
   texture. Input staging textures use `D3D11_BIND_RENDER_TARGET`; output
   textures use `D3D11_BIND_RENDER_TARGET | D3D11_BIND_VIDEO_ENCODER`. Configure
   BT.709 full-range RGB input and BT.709 limited-range NV12 output explicitly.
   In `D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE`, value 2 is 0-255 and value 1 is
   16-235; this differs from the Media Foundation nominal-range enum.
4. Wrap each converted texture with `MFCreateDXGISurfaceBuffer` and submit it
   as the MF input sample. Configure `IMFDXGIDeviceManager` before streaming,
   and only send `MFT_MESSAGE_SET_D3D_MANAGER` to MFTs advertising
   `MF_SA_D3D11_AWARE`.
5. Unlock asynchronous MFTs with `MF_TRANSFORM_ASYNC_UNLOCK` and consume their
   transform events on the existing encoder worker. Synchronous MFTs retain the
   existing pull path.
6. Resolve Native output dimensions from the selected capture source before
   encoder validation instead of substituting 1920x1080. Recreate the converter
   when a WGC texture's source dimensions change. Native output follows the new
   source dimensions; explicit output presets remain fixed and scale the new
   source to their configured dimensions. The recorder engine treats
   `FormatChanged` as a worker boundary: it drains the old encoder, updates the
   Native stream descriptor when applicable, reattaches the current GPU
   context, reconfigures the encoder, and starts a fresh replay-buffer epoch.

## Consequences

- Native WGC frames reach a hardware H.264 MFT without CPU readback.
- The conversion and MF sample path are backend-specific; `encoder-api` stays
  independent of Windows APIs.
- The converter and MF device manager require a D3D11-aware driver/MFT pair;
  unsupported candidates are rejected during trial configuration.
- The async MFT path is bounded by five-second event-progress timeouts so a
  stalled transform fails on the encoder worker instead of blocking capture.
- Source format changes cannot mix pre-change and post-change packets in one
  replay epoch; native physical resize and device-recreation coverage remains
  a manual hardware check.
- One-pixel colored details remain constrained by NV12 4:2:0 chroma resolution;
  correcting range, metadata, and unintended scaling does not make 4:2:0
  pixel-identical to the BGRA source.

## Verification

- `cargo test -p capture-windows --test native_display gpu_video_processor_converts_wgc_frame_to_nv12 -- --ignored --nocapture` passed on the interactive Windows development machine.
- `cargo test -p encoder-media-foundation --test native_gpu media_foundation_encodes_wgc_gpu_frame -- --ignored --nocapture` passed using the local AMD H.264 MFT.
- `pwsh -NoProfile -File .\scripts\check.ps1` passed after the integration.
- Deterministic tests assert the D3D11 color-space bitfields, Native source
  dimension selection, Native/fixed format-change behavior, BT.709 limited CPU
  reference patterns, and Media Foundation color attributes without a GPU.
- NVIDIA/Intel hardware, monitor/device removal during native encoding, GOP
  decode validation, decoded GPU color-bar values after this correction, and
  long-run resource trends remain untested.
