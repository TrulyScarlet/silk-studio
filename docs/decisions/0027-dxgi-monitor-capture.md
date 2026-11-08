# ADR 0027: Desktop Duplication for monitor capture

## Decision

Use `DisplayCapture` at the desktop shell's existing `VideoCapture` boundary.
Try DXGI Desktop Duplication first, retaining WGC as a startup fallback for
unsupported displays (including rotated displays). Failed DXGI setup is released
before WGC starts; fallback and its possible Windows capture border are logged.
Invalid configuration/source IDs are returned rather than selecting a different
display. No graphics hooks or process injection are involved.

Windows 10 build 19045 cannot use the WGC borderless-consent API. DXGI avoids
creating that indicator while preserving the existing encoding, audio, replay
buffer, and muxing interfaces. This remains **whole-monitor capture**, including
when the existing Game/Auto source resolves to the primary monitor.

## Ownership and timing

- One dedicated worker owns DXGI resources, including matching the D3D11 device
  to the selected output's adapter. The immediate context has multithread
  protection for sharing with the encoder.
- Eight GPU output textures plus one clean desktop texture bound frame storage.
  Encoder and queue leases prevent overwriting a texture in use. Old handles
  are invalidated before reuse. The event queue holds at most two frames, and
  congestion drops frames without blocking on the encoder, disk, or UI.
- Every acquired DXGI frame is released by a scope guard, including errors.
  Cursor-only updates cannot initialize the desktop image. Static desktops
  repeat the last image at the selected cadence to keep the engine responsive.
- Monotonic sampling timestamps start at zero, matching the existing capture
  contract. Fractional deadlines derive from an epoch, skipping missed ticks
  instead of accumulating rounding error or emitting catch-up bursts.
- A worker-owned high-resolution waitable timer supports 30/60/120 fps without
  changing the system-wide timer resolution or busy-waiting.
- DXGI color, masked-color, and monochrome pointer shapes are composited into
  each output frame. Only a cursor-sized rectangle is read back; cursor storage
  is bounded to 512 pixels per edge and 1 MiB of shape data. Full desktop frames
  remain GPU-resident. Cursor readback introduces a small synchronization cost
  which still needs gameplay benchmarking.
- Device/access loss reports `SourceLost` to the existing engine recovery path.
  The backend never silently swaps GPU contexts under an active encoder.

## Dependencies

No new package or license. Existing `windows` 0.62.2 (MIT OR Apache-2.0) bindings
provide DXGI, D3D11, and timer APIs. `Win32_Security` is enabled for the timer
creation signature. THIRD_PARTY_LICENSES.md records the expanded purpose.

## Validation

Deterministic tests cover lease exhaustion, handle invalidation, shutdown leases,
fallback ordering/restart, ten minutes of fractional frame deadlines at each
target rate, pointer clipping, alpha composition, AND/XOR and masked color.

`cargo test -p capture-windows --test native_dxgi -- --ignored --test-threads=1 --nocapture`
captures both attached monitors at 30/60/120 fps, checks advancing timestamps,
reads GPU pixels, verifies a held texture stays byte-identical across hundreds
of subsequent frames and stop, and verifies repeated startup chooses DXGI.
Observed approximately 61/121/241 frames per two-second check after switching
from coarse thread sleeps to a high-resolution timer. This measures capture
delivery, not game FPS or unique rendered frames. Local evidence is saved in
`target/dxgi-evidence/`.

`cargo test -p encoder-media-foundation --test native_gpu media_foundation_encodes_dxgi_gpu_frames -- --ignored`
verifies 120 real DXGI frames reach H.264 packets through the existing GPU path.

The installed app selected DXGI and NVIDIA H.264, saved a 25.949-second 2560x1440
replay, and browser playback/seek decoded real video pixels without errors.
A follow-up one-second 880 Hz test tone was present in the saved AAC track.
That test also exposed the existing silent-loopback clock issue described below.

## Audio timing issue discovered during validation

The former WASAPI timeout path always emitted 50 ms of silence after a 50 ms
wait. On this machine the scheduler often rounded that wait to about 62.5 ms,
causing a 25.949-second video to have only 20.757 seconds of decoded audio.
Silence is now counted from elapsed QPC time, with fractional sample accounting
and a maximum 100 ms allocation. Pending real packets are drained before waiting
and checked again after timeout. Synthetic timestamps identify the beginning of
the interval, and raw QPC fallback timestamps are converted to WASAPI's documented
100 ns units using the counter frequency.

Two deterministic tests cover ten minutes of rounded waits at 44.1/48 kHz,
bounded allocation, and transitions back from real audio. The opt-in native test
`cargo test -p audio-wasapi loopback_sample_duration_matches_elapsed_capture_time -- --ignored --nocapture`
observed 5.052958 seconds of samples spanning 5.052958 seconds of source time.

The final installed build saved an 8.965-second 2560x1440 replay with 8.810667
seconds of decoded AAC and an audible one-second test tone. A known green flash
at 4.2864 seconds preceded the detected tone onset at 4.361375 seconds (about
75 ms with 25 ms video sampling). This includes the test player's output latency
and is not a calibrated capture-latency measurement. The earlier multi-second
silence compression was absent. Browser playback and seeking decoded video
without errors. Results are in `target/dxgi-evidence/av-validation.json` and the
saved clip path is in `target/dxgi-evidence/final-clip-path.txt`.

Final validation: formatting, Clippy for affected crates and desktop, 111
non-ignored tests across capture, audio, clock, replay buffer, engine and desktop,
the explicit native capture/encoding/audio checks, production frontend build,
NSIS build and silent upgrade all passed. The app was left stopped on Studio.

## Limits

Verified native platform: this Windows 10 build 19045 machine and its two
landscape displays. Rotated displays use WGC, which may show the yellow border.
Windows 11, hybrid-GPU hardware, HDR fidelity, exclusive fullscreen games,
display unplug/replug and lock/unlock still require dedicated testing. A
successful desktop capture test is not a claim of measured gameplay overhead or
perfect audio/video synchronization across games and devices. Screen contents, including other applications,
are recorded; this is not per-game isolation.

## References

- https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe
- https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/ne-dxgi1_2-dxgi_outdupl_pointer_shape_type
- https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createwaitabletimerexw
- https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer
