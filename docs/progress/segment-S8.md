# Segment S8 Progress Log - Deterministic reliability slice

**Status:** Deterministic automation implemented; hardware recovery remains
open

**Plan reference:** [implementation-plan.md](../implementation-plan.md) section 12

## Delivered

- [x] Controller video and audio dependency factories are reusable `FnMut`
  factories, so each start creates fresh backend ownership.
- [x] Engine lifecycle tests cover repeated start/stop, settings updates, and
  clean stop after bounded recovery failure.
- [x] Engine event delivery uses a capacity-64 nonblocking channel; overflow is
  counted and reported instead of blocking capture or encoding workers.
- [x] Native WGC session control uses a capacity-1 stop channel; no S8 capture
  control queue is unbounded.
- [x] Display loss has bounded scripted reacquisition, full video-encoder
  reconfiguration, and a new replay epoch; successful and exhausted recovery
  paths are deterministic.
- [x] Audio device-loss recovery is exercised independently from video.
- [x] Resume boundaries clear old replay packets while retaining stream
  descriptors and require a fresh priming interval.
- [x] Format changes drain and reconfigure the video encoder, preserve stream
  registrations, and start a fresh replay-buffer epoch; deterministic coverage
  exercises the boundary.
- [x] Injected encoder failure verifies fallback, metrics, and epoch separation.
- [x] Sequential saves, gated concurrent saves/library scans, stale `.part`
  cleanup, and bounded save/replay resources are covered by
  `tests/endurance`.

## Verification

The short synthetic harness is intentionally not an eight-hour endurance
claim. It does not use real codecs, displays, audio endpoints, GPUs, or a
process-kill crash window.

```text
cargo test -p s8-endurance -- --test-threads 1 -> 12 passed
```

## Remaining limitations

- Native display/audio resource recreation and physical reconnect behavior
  require Windows hardware/manual procedures.
- Process termination during an in-flight native save is not reproduced by the
  deterministic tests; startup stale-file cleanup is covered.
- Long-duration memory, handle, GPU-resource, thread, latency, dropped-frame,
  and A/V-drift trends remain unmeasured.
- Native resolution-change/device-recreation behavior and the cross-vendor
  reconfiguration matrix remain outside this deterministic slice.
