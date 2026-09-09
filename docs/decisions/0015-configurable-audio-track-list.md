# ADR 0015 - Configurable audio track list

**Status:** Accepted for S4/S7 integration
**Date:** 2026-08-27
**Segment:** S4, S7

## Context

The original configuration represented desktop and microphone audio as fixed
fields. That shape cannot express additional independent sources, stable user
labels, per-source levels, or a user-selected stream order. The recorder must
still preserve bounded workers, keep media processing out of the UI, and migrate
existing settings without silently changing their meaning.

## Decision

1. Store audio as an ordered `AudioTrackSettings` list with a hard limit of
   eight entries. Each entry has a stable `id`, display `name`, `enabled` flag,
   `source_kind`, optional `device_id`, and bounded linear `gain` in the range
   `0..=8`.
2. Treat `null` device IDs as the default endpoint. Device enumeration remains a
   platform-shell service; the controller accepts opaque IDs and does not know
   Windows endpoint details.
3. On every session start, the controller filters disabled entries while
   preserving order and passes the resulting plans to an injected
   `AudioDepsFactory`. The desktop factory creates one independently owned input
   and worker per plan. Stream IDs are assigned deterministically after the
   video stream.
4. Apply each track's gain in the source-owned `AudioSynchronizer` before the
   audio encoder. No audio mixing or real-time media work is performed by the
   React/Tauri UI.
5. Preserve each configured track as a named Matroska stream. Version-1
   desktop/microphone fields migrate to two ordered tracks; the legacy
   `separateTracks` flag is ignored because the old engine boundary did not
   implement mixed-track behavior.

## Consequences

- The UI can add, remove, reorder, rename, enable, select, and level sources
  without adding source-specific controller branches.
- A failed optional source can be reported independently while sibling workers
  continue.
- The maximum worker count is explicit and bounded, but native support for a
  given endpoint still depends on Windows WASAPI availability.
- Existing version-1 configuration files remain readable through the versioned
  migration path; malformed or unknown fields remain rejected.
- Post-save compression remains a separate backend-neutral export concern and
  is not implied by the track model.

## Verification

- Configuration tests cover migration, IDs, names, device IDs, gain bounds, and
  the eight-track limit.
- Controller tests cover ordered enabled plans on repeated starts and reject
  non-finite or out-of-range gain values.
- Audio-sync tests cover deterministic gain application and timestamp behavior.
- Matroska tests cover named multiple-audio-track output.
- Native WASAPI endpoint selection, microphone loss, and long-run drift still
  require supported Windows hardware validation.
