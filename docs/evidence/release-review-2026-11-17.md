# Release review — 2026-11-17

## Follow-up implementation

ADR 0093 records the subsequent shared native capture-clock implementation and
authoritative automatic-game session attribution. Publisher exception extension
through 0.1.5 was explicitly approved by the user. The findings below describe
the reviewed pre-fix implementation, not a claim that the follow-up has already
passed live decoded audiovisual acceptance. Publication remains gated on that
acceptance and release artifact validation.

## Outcome

Hold publication. Automated gates pass, but the prior claim that audio/video
synchronization is resolved is not established by the implementation or tests.
No release, tag, installer deployment, or user configuration changes were made
during this review.

## Validation rerun

- `pwsh -ExecutionPolicy Bypass -File ./scripts/check.ps1`: PASS (formatting,
  Clippy, workspace tests, TypeScript, ESLint, production frontend build).
- `node ./scripts/test-settings-save-gate.mjs`: PASS (save/discard/retry,
  capture selection, presets, independent icon, themed feedback).
- Interactive hardware/gameplay acceptance tests are ignored by the general
  suite. Passing this suite is not evidence of measured live A/V alignment.

## Release blockers found by inspection

1. **Audio and video do not share a calibrated recording origin.**
   `VideoTimeline::with_origin` maps video to elapsed recording time, but
   `AudioWorkerConfig` has no recording origin. `AudioSynchronizer::process`
   uses `MediaTimeline::to_shared`, which maps each track's first stamp to zero.
   Delayed audio initialization therefore creates a persistent relative offset.
   Video anchoring also uses worker delivery time, not capture time, so queued
   frames and encoder startup can bias its initial offset. Existing timeline
   tests check video in isolation, not a corresponding captured audio event.

2. **Audio recovery removes elapsed downtime.**
   `AudioSynchronizer::reset_source` resumes at `expected_next_pts`, whereas
   video recovery anchors to elapsed recording time. Device recovery can therefore
   leave the recovered audio behind video. A contiguous-timestamps test is not
   a wall-clock synchronization test.

3. **Automatic target attribution is not authoritative capture state.**
   `capture_target_window("game:auto")` independently enumerates foreground or
   running games. It can identify a running game before capture has attached,
   or select a different game from a frozen retained capture session. Reporting
   `Valorant · Fullscreen` proves detection, not which session supplies frames.

4. **New-version publisher signing requires a decision.**
   Latest published GitHub release is v0.1.4. The local publisher signing
   environment variables are absent. `release-preflight.ps1` only permits the
   unsigned-publisher exception through 0.1.4. An updater key does not replace
   Windows publisher signing. Do not silently extend this exception or replace
   an existing version's release assets.

## Required follow-up

- Correlate capture timestamps for all streams to a common session clock,
  retaining capture/queue latency and recovery downtime correctly.
- Add deterministic joint audio/video regression coverage for delayed startup,
  independent track startup, queued frames, first-focus waits, and recovery.
- Measure decoded output with a known synchronized visual/audio stimulus before
  claiming live sync accuracy; do not infer it from process startup or durations.
- Expose the actual selected capture identity (including waiting/frozen state)
  rather than infer it with a separate window enumeration.
- Resolve publisher signing/exception authorization, then increment the version,
  commit reviewed release sources, build signed updater artifacts, and validate
  upgrade preservation before publication.

ADR 0092's claims of millisecond synchronization and eliminated long-term drift
are unverified and should not be used as release evidence.
