# ADR 0008 - WASAPI audio capture and synchronization

**Status:** Accepted for S4 implementation
**Date:** 2026-08-26
**Segment:** S4

## Context

S4 needs independent desktop-loopback and microphone capture, device-format
discovery, conversion to the audio encoder format, and timestamps on the same
timeline as video. Capture must remain Windows-specific while the conversion
and timestamp rules stay deterministic and testable without an audio device.
The project must not add a DSP dependency or move media work to the UI.

## Decision

1. Use shared-mode WASAPI through the existing `windows` 0.62.2 bindings. A
   `WasapiAudioCapture` instance represents either render-loopback or capture
   input; endpoint enumeration and default-device selection are independent for
   each kind.
2. Run WASAPI with an event callback and read packets through
   `IAudioCaptureClient`. Register an `IMMNotificationClient` on the same
   enumerator; its callbacks only latch a bounded atomic state and signal the
   capture event. Native COM and WASAPI objects remain owned by the worker that
   starts, reads, and stops the instance. Device invalidation and selected
   endpoint notifications are mapped to `AudioCaptureEvent::DeviceLost`;
   recovery orchestration belongs to the engine/controller recovery work.
3. Keep the shared `audio-api` contract platform-neutral. Native format
   discovery is exposed after `start` through `AudioCapture::format`; malformed
   or unsupported endpoint formats are rejected rather than guessed.
4. Put deterministic conversion in `audio-sync`. Each source has its own
   `AudioSynchronizer`, which converts source timestamps through `MediaTimeline`,
   produces interleaved F32 output at the configured target (48 kHz stereo by
   default), performs channel conversion and linear resampling, applies bounded
   gain, and tracks drift/discontinuity metrics.
5. Use the shared millisecond time base for synchronized output. Output PTS is
   monotonic; a timestamp jump beyond the configured threshold is an error so a
   malformed region cannot enter the replay buffer silently.
6. Do not add a CPU readback, UI callback, or unbounded queue to the audio path.
   Engine wiring will add one bounded worker per enabled source and preserve
   desktop audio when the optional microphone fails.
7. Use the Windows Media Foundation AAC LC MFT for the first native audio
   encoder. The backend converts synchronized interleaved F32 to the MFT's
   required 16-bit PCM input on the audio worker and requests raw AAC access
   units at the configured 44.1/48 kHz, 1/2/6-channel target.

## Consequences

- Audio conversion and timestamp behavior are unit-testable on every platform;
  actual WASAPI device access is compiled only on Windows.
- Linear resampling is intentionally small and deterministic, but is not a
  studio-quality resampler. A higher-quality implementation requires a future
  dependency and a separate licensing/performance review.
- `RecorderEngine::new_with_audio` owns one worker per injected source and
  registers distinct stream IDs. Deterministic shell helpers inject scripted
  sources, while the persisted-configuration desktop path selects native WASAPI
  capture on Windows; the legacy controller factory remains available for
  callers that do not provide audio.
- The implementation detects device invalidation, registers endpoint
  notifications, and performs bounded client recreation in the engine, but it
  does not provide real-container mux integration. Native AAC output is
  available as raw access units through the Media Foundation backend, and its AAC-LC
  AudioSpecificConfig is carried into the replay snapshot stream descriptor.
- End-to-end A/V sync and measured drift remain pending S4/S5 integration.

## Verification

- `audio-sync` tests cover S16-to-F32 conversion, channel conversion, gain,
  resampling, monotonic jitter handling, discontinuity rejection, and truncated
  payload rejection.
- Windows-only `audio-wasapi` tests cover supported S16/F32 format parsing,
  rejection of unsupported/incomplete formats without requiring a physical
  endpoint, and deterministic notification filtering for default and explicit
  endpoints.
- Windows-only `encoder-media-foundation` tests cover AAC configuration and
  deterministic F32-to-S16 conversion; the ignored native test produced AAC
  packets from synthetic audio on the validation machine.
- The ignored native loopback test and native shell path produced packets from
  a physical endpoint on the validation machine. Microphone capture, physical
  endpoint swap behavior, and 30-minute drift validation remain unverified.
