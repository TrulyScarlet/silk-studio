# 0040: Cross-vendor encoding and system-memory fallback

Status: Accepted, 2026-11-11

Silk must select usable encoders without requiring a particular GPU vendor.
Windows Media Foundation discovery remains the backend boundary. No vendor SDK,
process hook, new dependency, or UI media processing is introduced.

Auto selection tries the requested codec first, then H.264 candidates for HEVC
or AV1 requests. Each candidate receives its own codec in its configuration.
This covers missing optional encoders, configuration rejection, and runtime
failure. Explicit HardwareOnly and SoftwareOnly policies retain codec selection
and backend restrictions; Auto is the compatibility policy.

MFTs without D3D11 surface support now receive system-memory samples. A reusable
staging texture reads captured BGRA on the encoding worker; nearest-neighbor
scaling and existing BT.709 limited-range NV12/AYUV conversion prepare input.
This path does not require the D3D11 video processor used by hardware encoding.
Software H.264 uses CAVLC and low-complexity encoding settings. The previous
forced CABAC / maximum-complexity combination produced invalid H.264 in the
software path on this Windows installation; strict FFmpeg decoding now covers
both software and hardware bitstreams across multiple keyframe intervals.
The readback owns at most one staging texture and one frame-sized BGRA vector;
dimensions are bounded to 7680x4320. Texture layout, device changes, and padded
rows are checked. GPU synchronization may stall the encoder worker, and bounded
capture queues can drop frames; capture and UI never perform this readback.

The encoder can explicitly report its active codec. The recorder updates saved
clip metadata to that codec and starts a clean replay epoch on runtime fallback,
discarding incompatible video and old codec metadata while retaining existing
audio recovery behavior. Placeholder capability metadata is not interpreted as
an active codec. Encoded-frame metrics count actual output packets, including
drained packets, rather than successful calls that may have dropped input.

Validation includes deterministic codec-selection and replay-epoch tests, WARP
readback with known colors, padding and resizing, live WGC software encoding at
640x360/30 and 1600x900/60 with 45 Mbps requested, and continued NVIDIA hardware
encoding. Optimized software runs with the final compatibility settings produced
90/90 and 39/60 submitted frames,
respectively, without encoding failures on this machine. Software fallback
prioritizes continued recording; it cannot guarantee requested FPS. Lower
resolution/FPS may be necessary on slower CPUs. Intel/AMD physical devices have
not been tested here. Drivers, Windows media components, protected content,
unsupported texture formats, and the existing encoder size limits can still
prevent capture. This does not claim universal hardware support or ship an
additional software codec implementation.

References:
- https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_cpu_access_flag
- https://learn.microsoft.com/en-us/windows/win32/api/d3d11/ne-d3d11-d3d11_map
- https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder
