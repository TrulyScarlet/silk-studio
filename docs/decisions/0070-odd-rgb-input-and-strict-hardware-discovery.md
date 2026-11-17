# ADR 0070: Odd RGB source sizes and strict hardware encoder discovery

- Date: 2026-11-12
- Status: Accepted

Production owned-window WGC replay exposed a 336×235 BGRA source. The GPU video
converter incorrectly required even input dimensions, although even dimensions
are needed for its NV12 output, not packed BGRA input. Conversion failure then
exposed a separate discovery bug: the hardware MFT enumeration OR-ed SYNCMFT with
HARDWARE. Those type flags are inclusive; returned software transforms were tagged
as hardware, allowing HardwareOnly to fall back to a software H264 Encoder MFT.

Validate nonzero signed-rectangle-safe RGB input independently. Keep even output
validation for NV12. Preserve the full input source rectangle; GPU scale to the
configured even output rather than mislabeling/cropping source metadata. Driver
support is still required; native failure must remain an explicit failure.

Enumerate hardware candidates with HARDWARE | SORTANDFILTER, without SYNCMFT or
LOCALMFT inclusion. Hardware transforms continue through the existing event-driven
async worker. Software discovery remains a separate synchronous/local query. Do
not weaken HardwareOnly policy or relabel CPU readbacks as GPU-surface input.

Add deterministic parity and enumeration-flag regressions, extend actual hardware
video conversion checks to odd width/height, and save/decode production WGC replay
through software and hardware-only encoding. Assert zero encoder readbacks on the
tested hardware-only GPU route, with actual backend reported.

No dependency/version/license changes. Existing windows 0.62.2 and internal APIs.

References:
- [MFT_ENUM_FLAG: combinations enumerate multiple transform classes](https://learn.microsoft.com/en-us/windows/win32/api/mfapi/ne-mfapi-_mft_enum_flag)
- [D3D11_VIDEO_PROCESSOR_CONTENT_DESC](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ns-d3d11-d3d11_video_processor_content_desc)
- [DXGI format definitions, including NV12 constraints](https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format)
