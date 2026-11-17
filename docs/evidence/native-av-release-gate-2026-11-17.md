# Native A/V release gate — 2026-11-17

## Follow-up: recovery failure resolved; measured acceptance passed

Root cause: the endpoint supplied QPC timestamps approximately 10ms ahead of the
current sample. `capture_instant` rejected those timestamps, silently reverting
to the uncorrelated synchronizer timeline. Supporting positive and negative QPC
deltas preserves recovery downtime. A deterministic regression covers clock lead,
lag, and repeated elapsed-time recovery points.

The stimulus now drives the visual timer from `waveOutGetPosition` sample position,
not a stopwatch started before asynchronous SoundPlayer playback. The previous
91ms result used the less accurate stimulus and is retained as historical evidence.

Current 16-second native runs with two video restarts:
- Video-only recovery: four marker offsets -80, -80, -80, -63ms. PASS.
  Evidence: `%LOCALAPPDATA%/Temp/opencode/silk-av-A7U6W3`.
- Two additional 500ms audio outages: four offsets -97, -97, -97, -81ms. PASS.
  Audio endpoint 9580ms vs video endpoint 9617ms (37ms difference).
  Evidence: `%LOCALAPPDATA%/Temp/opencode/silk-av-7UqVAt`.
- Acceptance bound stayed at 100ms; no tolerance was relaxed.

These measurements validate this machine/default loopback endpoint after recovery,
not every endpoint, microphone, or GPU. They are not frame-perfect latency claims.
The replay selected after video recovery does not span all earlier recovery gaps.

## Historical failed runs

Test tooling:
- `node scripts/verify-native-av-sync.mjs`
- `$env:SILK_AV_AUDIO_RECOVERY='1'; node scripts/verify-native-av-sync.mjs`

The test opens an owned full-screen flashing window and plays a generated pulse
WAV on the default output. It records native DXGI plus WASAPI loopback with two
injected video losses. It decodes the saved MP4 with the hash-approved installed
FFmpeg and compares image transitions with decoded audio pulse onsets.
This is software-scheduled stimulus, not externally calibrated latency measurement.
No game/injection access or user config edits are involved.

## Video recovery with uninterrupted audio

PASS within the explicitly chosen 100ms acceptance bound:
- Visual marker times: 1.199, 3.199, 5.199 seconds.
- Audio marker times: 1.290, 3.290, 5.290 seconds.
- Stable audio-minus-video offset: +91ms.
- Native packets and decoded output validated after two video recoveries.
- Evidence: `%LOCALAPPDATA%/Temp/opencode/silk-av-YU27S1/results.json`.

This does not prove frame-perfect sync, delayed-start behavior, all hardware,
microphone behavior, or correctness of clips spanning every recovery boundary.

## Combined audio/video device recovery

FAIL: the opt-in wrapper stops only its owned audio capture session twice and
waits 500ms each time before requesting normal engine recovery. It does not alter
Windows device settings.
- Video packet endpoint in final replay: approximately 5605ms.
- Audio packet endpoint: 4352ms (second trial: 4309ms).
- The native test correctly fails its existing 250ms endpoint tolerance.
- Reconfiguring the AAC encoder after recovery was experimentally tried, did not
  solve the failure, and was removed. Root cause is not yet established.

## Confirmed muxer issue fixed separately

The MP4 writer serializes sample durations into stts, not Mp4Sample.start_time.
Audio pause timestamps previously updated only start_time and were lost on disk.
The muxer now extends the preceding sample's timing-table duration over a real
pause. `audio_recovery_gap_survives_mp4_timing_tables` verifies a 521ms second
AAC packet starts at tick 25008 on a 48kHz track after serialization/readback.
This unit test passed. Native decoded audio-outage acceptance is still failing
upstream and this change is not evidence that the release gate is resolved.

## Follow-up

Trace captured frame time, synchronized PCM time, AAC input sample time and AAC
output time across the simulated outages. Fix the point where elapsed downtime
is removed, then rerun both native tests and the complete automated suite before
building or publishing 0.1.5. The publisher-signing exception remains authorized;
updater signatures remain mandatory.
