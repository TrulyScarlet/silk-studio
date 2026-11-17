# ADR 0092: Recording timeline origin anchoring and audio/video synchronization

Date: 2026-11-17

## Context

Silk maintains independent capture and encoding workers for video and up to 4 audio
streams. Audio capture (WASAPI loopback and microphone) begins capturing continuous
audio and silence packets immediately at engine startup, with timestamps calibrated
to the start of the recording session.

Video capture initialization (DXGI duplication, NVIDIA/Intel/AMD Media Foundation encoder
transforms, or automatic game discovery waiting for focus) starts delivering video frames
some time after engine start. In automatic game mode (`game:auto`), `DisplayCapture`
idles until a recognized game window is in the foreground, which can take seconds or
minutes. Similarly, display recovery after game termination or device loss leaves
video capture in an idle or re-attaching state while audio continues running.

Previously, `VideoTimeline::default()` started with `offset_ms = 0` and unanchored
initial frames, assuming the first delivered video frame always represented $T = 0$.
Furthermore, `restarted()` pre-sampled recovery timestamps when recovery was scheduled
rather than when the first frame actually arrived after an idle wait. Consequently,
any startup delay, auto-game wait, or recovery downtime caused video timestamps to
fall behind audio by the duration of the wait (e.g. 2.5 minutes in live Valorant testing),
resulting in desynchronized playback in saved clips.

## Decision

1. **Origin-Anchored Video Timeline**:
   Initialize `VideoTimeline` with the session's recording origin `capture_started_at: Instant`.
   When the first video frame arrives, its presentation timestamp is anchored to the
   actual elapsed time since recording started (`current_elapsed = origin.elapsed()`),
   placing the initial video frame at the exact millisecond matching the concurrent audio
   stream.

2. **Accurate Recovery and Format-Change Re-Anchoring**:
   When recovery is initiated or `VideoCaptureEvent::FormatChanged` occurs, signal
   `request_restart()`. Rather than freezing a stale timestamp at the start of recovery,
   anchor the new video epoch to the elapsed timeline time at the exact moment the
   first frame of the new stream actually arrives.

3. **Detection of Source Clock Resets and Idle Gaps**:
   If a source capture reset is observed (`source_ms < last_source_ms`) or an idle gap
   exceeding 1000 ms elapses without video frames, re-anchor the offset so video resumes
   at the current real-world recording time without backward jumps.

4. **Strict Monotonicity**:
   Target timestamps are clamped to `last_ms.map_or(current_elapsed, |last| current_elapsed.max(last + 1))`,
   guaranteeing strictly monotonic packet timestamps in all edge cases.

5. **Recovery Scope**:
   Treat `VideoCaptureError::SourceUnavailable` (such as game window termination) as a
   recoverable event in `engine.rs` alongside `DeviceLost`, allowing automatic game mode
   to return to an idle discovery state without terminating the video worker.

## Consequences and Limits

- Video and audio streams in replay clips remain synchronized within milliseconds
  regardless of startup delays, window focus timing, or recovery periods.
- Re-anchoring preserves natural video frame cadence during continuous streaming and
  only adjusts the base offset when a session discontinuity actually occurs.
- If a game is closed for several minutes and then reopened, clips that cross the gap
  will reflect the downtime appropriately on the shared timeline.
- WASAPI hardware clocks and DXGI/system QPC clocks on Windows share the same hardware
  counter origin, eliminating long-term drift.
