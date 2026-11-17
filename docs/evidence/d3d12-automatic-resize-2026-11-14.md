# Observed owned D3D12 resize recovery

ADR 0077; exact opted-in owned fixtures only. No new external dependencies.

## Implemented

- Automatic feature/launch route, selected-window bounded atomic geometry mailbox,
  actual renderer ResizeBuffers1, pre-call media suspension/source release,
  same-chain/queue validation, bounded worker offer and exact epoch approval.
- Parent adapter handles producer-originated Resize only in automatic mode,
  reuses delivery/retirement/format barriers and rejects parent resize controls.
- Pure gate tests cover repeated epochs, overlap, invalid dimensions, stale
  approval, terminal closure and epoch exhaustion without wrapping.

## Actual results

- WARP and hardware native: three actual observed resize epochs, correct green
  dimensions/pixels, original retained red pixels, bounded stop, renderer alive.
- WARP: stop while awaiting retirement and stale approval rejection pass without
  new media or target termination.
- Subsequent targeted WARP cases pass stop during fresh approval and a second
  actual resize during held retirement (terminal rejection without new media).
- A subsequent automatic parent-adapter delivery-barrier test passes: an already
  delivered old frame resolves while retirement is pending, becomes unresolvable
  from its retired context only after the caller crosses the event boundary, and
  its previously resolved lease retains pixels through fresh media and stop.
- WARP/software and hardware recorder: each saved and approved-FFmpeg-decoded
  clips at 318x178, 640x360 and 800x450. The hardware replay test asserts zero
  encoder readbacks; capture reports zero readbacks.
- All-feature prototype/helper Clippy and late-helper-only Clippy passed. Safe
  prototype tests passed 60 (two native ignored), helper tests and full workspace
  formatting/Clippy/tests/frontend typecheck/lint/build passed before final native
  acceptance. Subsequent final checks are recorded separately below if run.
- Tracked combined native run **passed, actual exit 0**, 2026-11-14
  06:32:03–06:50:02 UTC:
  `C:\Users\Home\AppData\Local\Temp\opencode\silk-auto-resize-native-b6ff91d6014f40cca50d3f1d4de6c91d`.
  This includes **63 recorder and 18 attachment tests**, plus prior native suites.
  The two subsequent targeted cases above were added after this combined run.

Final safe/workspace/frontend checks, all-feature Clippy, auto-helper-only and
late-helper-only Clippy and tracked whitespace checks subsequently passed:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-auto-final-6a969c11894d4fabb65e09db0b48736d.log`.
Strict dependency verification and settings-save regression passed separately.
The additional delivery-barrier case above was added afterward and passed its
targeted native run and all-feature Clippy. No runtime code changed afterward.

The first targeted native attempt correctly hit the six-surface budget because
the test retained the retired consumer while opening a replacement. The test now
drops the retired consumer first; the budget was not increased. One initial
combined run failed an older recreation-budget test that incorrectly expected
an immediate error after an asynchronous request. A queued current-epoch frame
was legal. It now waits boundedly for rejection and rejects any third format.
Targeted budget tests and the final combined run pass. Failed records preserved:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-auto-resize-native-38a8eb91693d4c24be931349a71ce84b`.

## Limits

Follow-up targeted cases passed actual retained-back-buffer ResizeBuffers1
failure (DXGI_ERROR_INVALID_CALL) and successful 641x360 resize rejection. Both
return terminal worker status without new media and keep the owned renderer
alive. This closes the earlier API-failure/unsupported-size evidence gap; the
broader limits below remain. Repeated window-cycle stress is separately pending.

Same retained chain/device/single DIRECT queue only. No general process loading,
real-game/fullscreen acceptance, automatic device-graph replacement, unexpected
in-flight removal/TDR recovery, release packaging or endurance pass. Native
concurrent API stress needs more coverage.
Local GPU draining is bounded but may pause the owned resize
call; no latency/throughput claim. No monitor fallback or exclusion bypass.

No commit, push, PR, installer or installed-app replacement.
