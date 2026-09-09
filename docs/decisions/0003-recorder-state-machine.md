# ADR 0003 — Recorder state machine extensions

**Status:** Accepted
**Date:** 2026-08-26
**Segment:** S0

## Context

Spec §19 lists explicit transitions but does not enumerate every edge the
product needs: `Degraded` appears in the UI state list (§9.2) yet has no
edges in the transition diagram, and abort/cleanup paths are implied but
unspecified ("Any active state -> Stopping").

## Decision

The transition table in `recorder-engine::state` implements:

1. **Spec-listed edges verbatim**, verified by dedicated tests.
2. **Degraded behaves like Ready for user-facing operations:** it may enter
   `Saving`, `Recovering`, return to `Ready`, escalate to `Error`, or stop.
3. **Abort edges:** every active state transitions into `Stopping`,
   including `Error -> Stopping` (stop cleanly from failure). `Stopping`
   only ever leads to `Stopped`.
4. **Recovery outcomes:** `Recovering` may land on `Ready` or `Degraded`,
   or fail into `Error`.
5. Everything else is rejected with the structured
   `InvalidTransition { from, to }` error (spec §19 requirement).

## Consequences

- UI state labels and engine states stay identical strings (tested).
- Later segments that need new edges must extend the single static table
  plus its tests; no ad-hoc transitions exist anywhere else.
- `Saving` remains modeled as a short-lived edge rather than a blocking
  state; concurrent save jobs are handled by the controller's job manager
  per spec §19 guidance.
