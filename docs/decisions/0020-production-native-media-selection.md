# ADR 0020 - Native media selection for the configured desktop path

**Status:** Accepted
**Date:** 2026-08-28
**Segment:** S8

## Context

The desktop shell had a single dependency factory that selected deterministic
scripted capture and mock encoders unless `SILK_REAL_*` environment variables
were present. That behavior was useful for shell tests, but the same factory
was used by normal application startup. It allowed user-requested clips to be
published with deterministic mock H.264/AAC payloads that are not decoder-valid.

## Decision

1. Keep environment-controlled backend selection for deterministic shell and
   native smoke tests.
2. Make `build_controller_from_config`, the persisted-configuration startup
   path, select native WGC capture, Media Foundation H.264, and native AAC on
   Windows.
3. Keep mock backends available only through explicit test-oriented builders;
   a missing native backend must surface as a startup/worker error rather than
   silently creating an unplayable user clip.

## Consequences

- Normal Windows clips are produced by the native media pipeline and can be
  validated by standard media tools.
- Deterministic unit and shell tests retain their bounded scripted sources and
  mock encoders.
- Native device, Media Foundation, and WASAPI availability remains a runtime
  requirement for production capture; failures are reported through the
  existing controller error path.
