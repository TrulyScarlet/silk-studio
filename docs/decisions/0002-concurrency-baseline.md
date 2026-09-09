# ADR 0002 — Concurrency baseline for Segment S0

**Status:** Accepted
**Date:** 2026-08-26
**Segment:** S0

## Context

Spec §14 mandates bounded channels, short critical sections, and workers
that never block on disk or UI work. Segment S0 only has mock backends;
full worker topology arrives through S4/S5.

## Decision

- **Primitives:** `std::thread` + `std::sync::mpsc::sync_channel` (bounded)
  + short-lived `Mutex` sections. No async runtime in S0 — the media path is
  thread-based by design and adding tokio now would be speculative weight.
- **S0 topology:** control methods run on the caller's thread; exactly one
  worker (`silk-video-worker`) exists while a session runs. It owns the
  capture and encoder objects outright; nothing GPU/platform-ish crosses
  threads except immutable frame/packet values.
- **Bounds:** replay packet store capped at `EngineConfig::max_buffered_packets`
  (default 4096) as a placeholder until the duration-based buffer lands in
  S1; controller save queue bounded at 8 (`SAVE_QUEUE_BOUND`), overflow
  rejects instead of growing (BUF-008 rehearsal).
- **Shutdown protocol:** `Arc<AtomicBool>` flag checked between events +
  `JoinHandle::join` on stop. Blocking waits inside real WGC/WASAPI events
  will be revisited in S2/S4 where those APIs dictate wait semantics.
- **Lock discipline:** no lock is held during muxing; snapshots clone
  reference handles (`Arc<EncodedPacket>`), never payload bytes (§14.2).

## Consequences

- Deterministic tests without runtime flakiness.
- Worker count grows in later segments; each addition updates
  `docs/architecture/threading.md` and this ADR's successor.
