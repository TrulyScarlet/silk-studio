# 0038: Recover Media Foundation encoder output changes

Status: Accepted, 2026-11-11

Silk 0.1.1 diagnostics from an Intel Iris Xe machine show successful WGC
capture followed by Quick Sync returning MF_E_TRANSFORM_STREAM_CHANGE.
Treating that notification as an encoder failure exhausted GPU-capable
fallbacks and stopped the replay buffer.

Following Microsoft's Handling Stream Changes documentation, the encoder now
selects the advertised updated output type, sets it, refreshes output buffer
requirements, and retries ProcessOutput. Recovery stays within the encoding
worker and the same output request, preserving asynchronous output credits,
queued timestamps, and the replay timeline. It allows four renegotiations per
output request, then uses the existing error/fallback path to avoid an infinite
driver loop. Codec or dimension changes are rejected because the existing replay
track cannot safely combine incompatible formats.

No dependencies changed. Existing GPU requirements for fallback encoders remain;
this change does not add GPU readback or a software encoding pipeline.

Validation includes formatting, encoder lint/tests, real WGC encoding, and a
360-frame 1600x900 regression across multiple keyframe intervals on this machine.
This does not establish recovery on the reporting Iris Xe driver: that machine
must install the update and retry. Desktop Duplication's unsupported pixel format
is separate; its working WGC fallback remains available.

Reference: https://learn.microsoft.com/en-us/windows/win32/medfound/handling-stream-changes
