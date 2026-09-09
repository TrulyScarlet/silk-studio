# ADR 0001 — Configuration policy

**Status:** Accepted
**Date:** 2026-08-26
**Segment:** S0

## Context

The recorder needs a versioned JSON configuration (spec §18, §8.11). The
spec requires validation before settings are applied and versioned config
migrations, but leaves file-level behavior unspecified.

## Decision

1. **Envelope:** `{ "version": <int>, "settings": {...} }`. Version `0`
   means "no marker" (pre-versioning files); versions above the supported
   maximum are rejected with `CONFIG_VERSION` errors.
2. **Missing file → defaults.** Loading a non-existent path returns built-in
   defaults plus a warning; callers decide when to persist them. This makes
   first launch frictionless (user story 7.1).
3. **Strict content:** unknown keys are rejected (`deny_unknown_fields`) so
   typos fail loudly instead of silently misconfiguring capture.
4. **Validation is total:** every rule violation is collected into one
   structured error listing all field issues, not just the first.
5. **Migrations are additive merges:** moving to version N fills missing
   keys from the version-N defaults. When semantics must *change* rather
   than extend, an explicit per-step function must replace the merge for
   that step.
6. **Atomic saves:** writes go to a sibling `.tmp` file followed by rename.

## Consequences

- Corrupt JSON, unknown keys, and out-of-range values are all first-class,
  tested error paths (S0 exit criterion).
- Future migrations stay trivial until a destructive change forces step
  functions.
- Hotkey chord syntax is validated at configuration time via
  `normalize_chord`; actual registration arrives in Segment S6.
