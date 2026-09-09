# Segment S1 Progress Log — Synthetic replay engine

**Status:** Exited 2026-08-26
**Started:** 2026-08-26
**Plan reference:** [implementation-plan.md](../implementation-plan.md) §5

## Task checklist (plan §5.4)

1. [x] `media-clock`: monotonic clock abstraction (`MonotonicClock`/`InstantClock`), `MediaTimeline` with initial-offset compensation and discontinuity detection (2-stamp calibration, configurable threshold)
2. [x] Replay-buffer insertion per spec §15.1: validation → global sequence → stream-aware storage → newest-position update → eviction → memory/duration metrics
3. [x] Eviction per §15.2: retain from newest video keyframe ≤ `T_b`; audio aligned to earliest retained video; nothing evicted before a safe keyframe exists
4. [x] Snapshot creation per §15.3: keyframe-anchored selection with shared payload references; lock released before muxing; snapshots outlive live eviction (BUF-009)
5. [x] Timestamp normalization per §15.4: zero-origin rebase, order-preserving, distinct PTS/DTS kept, idempotent, handles negative jittered minima
6. [x] Audio alignment groundwork (BUF-011): overlap-inclusive snapshot selection; final trim lands at mux stage in S5
7. [x] Live duration change (BUF-012): `set_retention` re-evicts immediately, no restart
8. [x] Concurrency (BUF-008): serialized save requests over immutable snapshots; multi-saver concurrency test
9. [x] Synthetic generators (§27.3): seeded backwards-PTS jitter on scripted capture; B-frame-style DTS lag option on mock encoder; existing format-change/source-loss steps retained
10. [x] Pre-roll reporting (BUF-013): `SnapshotOutcome{requested_start_pts, actual_start_pts, keyframe_preroll_ticks}` + diagnostics line at save

## Requirements verified

| Requirement | Evidence |
|---|---|
| BUF-001 continuous retention | worker inserts every encoded packet; e2e lifecycle |
| BUF-002/012 configurable duration live | `set_retention` unit tests |
| BUF-003 range 15–120 s supported | config validation (S0) + i64 ms retention in buffer |
| BUF-004 strict bound | property P2: span ≤ retention + GOP slack; packet cap valve |
| BUF-005 timestamp-based eviction | eviction tests incl. boundary keyframe case |
| BUF-006 keyframe-anchored start | snapshot starts-on-keyframe test; property P3a |
| BUF-007 saves never stop ingestion | mux-failure e2e: buffer intact after failed save |
| BUF-008 serialized/concurrent-safe saves | concurrent-savers property test |
| BUF-009 snapshots survive eviction | refcounted payloads + post-eviction structural checks |
| BUF-010 normalized timestamps | normalize unit + property P4 |
| BUF-011 audio trimmed to interval | overlap selection test (final trim in S5) |
| BUF-013 pre-roll reported | SnapshotOutcome fields + save diagnostics |

## Command log

```text
cargo test -p media-clock      → 7 passed
cargo test -p replay-buffer    → 12 behavior + 2 property suites passed
                                 (160 seeds × 700 randomized steps = 112k operations,
                                  0.8 s; concurrency: 1 producer + 4 savers)
cargo check --workspace --all-targets → clean after fixes below
scripts/check.ps1              → "All gates passed."
```

## Issues found and fixed during development

1. Discontinuity detector initially fired on the first cadence interval;
   now calibrates for two stamps before judging jumps.
2. Normalization originally skipped negative shifts, breaking jittered
   PTS near stream start; shift is now the true global minimum.
3. Engine's redundant sequence counter removed — the buffer assigns the
   authoritative ingestion sequence (single source of truth).
4. Ready is now asynchronous (priming-driven) rather than set during
   start; controller gained `poll()` and pumps transitions on every
   command; shell/E2E tests updated to wait for readiness explicitly.

## Deviations and decisions

- ADR 0004 records storage layout, ms-only time-base rule, bound slack,
  priming semantics, epoch handling, and memory overhead factor.
- Engine default retention is 400 ms for synthetic pipelines; the UI will
  plumb the real user setting when settings wiring lands (S7).
- Disk-assisted buffering stays rejected (open decision #3).

## Untested / known limitations

- Property scenarios are synthetic by design; no hardware claim applies.
- Memory accounting excludes allocator overhead (~200 B/packet factor is
  documented, not measured under load); endurance runs in S8 validate it.
- Audio alignment currently includes partial-overlap chunks; exact sample
  trimming happens with real audio in S4/S5.
