# ADR 0019 - Background scheduling for save and export work

**Status:** Accepted
**Date:** 2026-08-28
**Segment:** S5/S8 performance hardening

## Context

Replay capture and encoding must remain responsive while a user saves a clip.
Silk already keeps container writes and post-save FFmpeg work on bounded,
dedicated workers, but asynchronous work can still compete with a game for CPU,
filesystem, and memory scheduling. The native MKV writer also flushes and
validates staged output synchronously on its worker. The optional MP4 export
re-encodes through `h264_mf` and can therefore compete for encoder resources.

## Decision

1. Enter Windows thread background processing mode at the start of
   `silk-save-worker` and `silk-export-worker`.
2. Launch external FFmpeg children with `BELOW_NORMAL_PRIORITY_CLASS` and then
   request Windows process background mode.
3. Keep capture, audio, and real-time encoding workers at their existing
   scheduling level; never trade capture continuity for save throughput.
4. Treat priority changes as best effort. A failed Windows priority call does
   not fail a save or export, and non-Windows builds retain their existing
   worker behavior.
5. Preserve the existing bounded queues and immediate `SaveQueued`/
   `ExportQueued` responses. If queues are full, reject work rather than block
   capture or the command path.

## Consequences

- Save filesystem work receives lower CPU and I/O scheduling priority while
  capture continues to own the real-time path.
- Post-save FFmpeg work receives lower process priority, reducing CPU and I/O
  contention during gameplay.
- Under heavy system load, saves and exports may finish later; this is
  preferable to delaying capture or encoding.
- Windows GPU scheduling and individual driver behavior are not fully
  controlled by these calls, so real-game validation remains necessary.

## Verification

- Deterministic save-overlap coverage continues to prove capture progresses
  while the save muxer is gated.
- Focused Rust tests and the workspace validation gate must pass after this
  change.
- A real game run with save and automatic MP4 export enabled remains manual
  evidence, not a claim made by the automated suite.
