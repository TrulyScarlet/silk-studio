# ADR 0004 — Replay buffer design (Segment S1)

**Status:** Accepted
**Date:** 2026-08-26
**Segment:** S1

## Context

Spec §15 defines insertion, eviction (`T_b = T_n − D`), snapshotting, and
timestamp normalization; §8.5 adds thirteen BUF requirements including a
strict bound (BUF-004), keyframe-anchored clip starts (BUF-006), saves that
never stop ingestion (BUF-007), snapshots surviving live eviction (BUF-009),
live duration changes (BUF-012), and pre-roll reporting (BUF-013).

## Decision

1. **Storage:** per-stream `VecDeque<Arc<EncodedPacket>>` in DTS order plus
   a shared monotonic sequence counter. Payloads are immutable refcounted
   bytes (media-types), so snapshots clone handles, never bytes.
2. **Shared time base:** every registered stream must use the millisecond
   base; sources convert before buffering (enforced at `register_stream`,
   error `UnsupportedTimeBase`). This makes retention arithmetic exact.
3. **Eviction rule:** retain from the newest video keyframe with
   `dts <= T_b`; drop strictly older video. Audio streams drop packets that
   end *before* the earliest retained video DTS; partial overlaps are kept
   for mux-stage alignment (BUF-011). If no eligible keyframe exists yet,
   nothing is evicted — an undecodable prefix is never possible.
4. **Bound semantics:** the duration bound holds within documented slack of
   one GOP + two frames (keyframe pre-roll); an absolute per-stream packet
   cap acts as a safety valve against pathological input. Memory is tracked
   as payload bytes (`memory_bytes`) with the overhead factor documented:
   packet struct + Arc + deque node ≈ 200 B/packet, i.e. <1 % at realistic
   bitrates (spec §15.5).
5. **Snapshots:** take `Vec<Arc<..>>` clones under the buffer lock, then
   release it; muxing happens lock-free (§14.2). Outcome reports requested
   start, actual start, and pre-roll (BUF-013).
6. **Normalization:** subtract the global minimum timestamp across streams
   (may be negative when PTS jitters below stream start); result is ≥ 0,
   order-preserving, idempotent, and keeps distinct PTS/DTS.
7. **Epochs:** `start_new_epoch()` clears all data on codec-parameter
   change; sequences never reset so cross-epoch mixing stays detectable
   downstream (S5 mux validation).
8. **Priming:** `is_primed()` = buffered video span ≥ retention. The engine
   transitions Buffering → Ready on this signal, matching §9.2.

## Consequences

- All five §27.2 property invariants are enforced by randomized tests
  (48 committed seeds × 350 steps) plus a multi-saver concurrency test.
- The engine's former count-bounded ring placeholder was removed; S2+
  consume the real buffer unchanged.
- Disk-assisted buffering (open decision #3) remains rejected for now;
  revisit only if 120 s × maximum bitrate exceeds memory budget.
