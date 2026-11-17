# Owned D3D12 healthy teardown/recreation evidence

Date: 2026-11-12. ADR 0068. Default-off owned fixture, not a game backend.

## Scope completed in this pass

1. **Implementation:** release old producer media/worker references and factory
   offers; drain/drop the whole renderer before creating device/queue/chains on
   the retained HWNDs. Rediscover the selected chain independently and validate
   DIRECT/device/HWND/format/adapter/session before fresh handle approval.
2. **Failure paths:** exact retirement/approval, stale controls, missing proofs,
   injected teardown/recreation failures, terminal queued-media rejection and an
   explicit stopped/new-child restart. Failures do not silently retry/fallback.
3. **Bounded stability tests:** repeated native epochs, 60-second native runs,
   retained old pixels and ten saved/decoded recorder epochs on both paths.
4. **Cleanup/validation:** shared resize/recreation test helpers, typed bounded
   controls, launch flags struct, feature-isolation checks, ADR/progress/dependency
   records, full native/hardware/decode and workspace/frontend regression gates.

This completes those rounds for **healthy controlled recreation only**. No actual
driver reset/device-removal recovery, production integration or real-game/fullscreen
acceptance was implemented. No old device is retained for a pointer comparison:
D3D12's singleton constraint still applies. Calls and our object-graph release are
observed; runtime-internal destruction is not independently established.

## Implemented route and bounds

Feature `d3d12-device-replace-fixture`; source `owned-d3d12-device-recreation`;
exact owned child with `--allow-owned-d3d12 --producer --present-hook --factory-hook
--bootstrap --rearm --replace-device`. No remote loader or third-party targeting.

Validated kind-6 requests use the current exact session and bounded even sizes.
The existing retirement/delivery barrier prevents an already-delivered frame's
resolver from being invalidated early. Parent retires/drains its old consumer;
worker releases old bridge sources/media, chain/queue/device and pending factory
offers. Initial discovery locals no longer retain old COM interfaces through the
transport loop. Owner retains only two HWND guards across renderer drop/rebuild,
including failure paths through worker join. New capture is staged only if the
chain uses the same installed Present entry and selected HWND. No hook reinstall,
unhook or unload. New context/FormatChanged is ordered before new frames.

One in-flight transition plus one queued request; three shared ring slots and
six-local-surface retirement budgets remain enforced. Factory offers remain bounded
at eight; old unrelated-chain offers are cleared before recreation. Actual fences
must drain; uncertain completion remains terminal. No UI/disk/control I/O is added
to Present. Renderer progress accumulates monotonically across renderer resets.
Native producer lifetime remains bounded (180-second owner / 175-second transport
deadlines), not an unlimited-running production capture service.

## Tests actually run

Hardware: NVIDIA GeForce RTX 2080 Ti. WARP: Microsoft Basic Render Driver.

Native lifecycle validates two recreations, each with twelve fresh publications,
same target/generation and adapter/QPC frequency, exactly next epoch/requested size,
increasing QPC and expected green/blue pixels. Old red pixels and retired local
context remain safe; new media does not publish before exact approval. Both pass.

Timed native results from the final full gate:

- WARP: **20 same-HWND recreations, 6,624 checked publications, 60.1 seconds**.
- Hardware: **20 same-HWND recreations, 5,515 checked publications, 60.2 seconds**.

Every checked publication has expected pixels and monotonic QPC. Retained initial
red pixels remain unchanged across all epochs and after child exit. Both original
Present paths continue after capture cancellation. These counts are test traffic,
not performance/encoder FPS claims or proof of hours-long stability/leak freedom.

Native rejection tests pass for stale request/ACK, invalid dimensions, wrong
retirement, wrong approval, missing retirement/approval timeout, and replacement
without distinct opt-in. After-renderer-release and before-hello injected failures
pass on WARP and hardware. Other stale/proof negatives run on WARP.

Recorder tests pass for:
- Delivered-frame resolver gap and retained old lease pixels through recreation.
- Explicit six-surface failure when retaining multiple old contexts.
- Cancellation while waiting at the delivery barrier (under two seconds).
- Same-dimension recreation interleaved with same-chain resize on WARP/hardware.
- After-release/before-hello/bad-hello terminal failure, old context retirement,
  no new format/frame delivery and successful explicit new-child restart (WARP).
- Three saved/decoded replay epochs and ten-epoch replay runs on WARP/software
  and hardware-only NVIDIA H.264. Every saved frame decodes with expected colors.

Three-epoch final observations:

| Path | 318×178 red | 640×360 green | 800×450 blue |
| --- | ---: | ---: | ---: |
| WARP/software | 833 ms | 849 ms | 873 ms |
| Hardware-only | 838 ms | 864 ms | 834 ms |

Ten-epoch clips alternate 640×360 green / 800×450 blue after initial red. WARP clip
durations: 837–860 ms. Hardware: 834–859 ms. Capture readback counters are zero;
hardware encoder readback counters are zero. WARP explicitly uses software
encoding with encoder readback. Pixel verification itself uses test-only readback;
no zero-copy or vendor-internal behavior claim is made.

## Final validation

- `scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode`: passed,
  including 49 safe library tests (two native owner tests run separately), seven
  safe recorder tests, three helper tests, all prior native suites, eleven device-
  lifetime/recreation native tests and **49 native recorder tests (33 WARP / 16
  hardware)**. Ten of those recorder tests are new recreation coverage.
- All-feature, bootstrap-only, rearm-only and existing minimal-feature Clippy:
  passed with warnings denied.
- `scripts/check.ps1`: formatting, workspace Clippy/tests and frontend typecheck,
  lint/build passed.
- Strict dependency verification and standalone settings-save regression passed.
- Final formatting and whitespace checks passed, including explicit no-index
  checks of new/untracked implementation/tests/evidence. The first wrapper treated
  no-index's normal "files differ" exit 1 as a whitespace failure; the corrected
  wrapper checks diagnostics and actual error status. No source defect was found.
- No owned D3D11/D3D12 fixture processes remained; about 2.10 GB free disk space.

Earlier targeted runs also passed the new native and recorder suites. All source
changes remain uncommitted; no push, PR, installer or release produced.

## Remaining risks / untested scope

Actual device removal/reset during submission or drain is still terminal and not
recovered. No TDR/driver reset was induced. Soaks are minute-scale, not long-run
endurance; GPU memory/driver-internal allocation lifetimes were not independently
profiled. Only one hardware model and adapter 0 plus WARP were tested. Other
vendors/adapters and actual fullscreen behavior remain unverified. Remote D3D12
loading, pre-existing-chain discovery, other factory variants, arbitrary resize
interception, authenticated production IPC and shipping integration remain future
work. Protected-game boundaries, including VALORANT capture exclusion, are unchanged.
