# Early owned D3D12 remote helper evidence

ADRs 0072–0075. Date: 2026-11-13. Exact opted-in fixture children only.

## Implemented and actually exercised

The passive executable creates two HWNDs without graphics chains. The parent
pins the selected PID/creation FILETIME/HWND, loads the canonical sibling helper
DLL using normal OS loading, starts its explicit export, and waits for factory
readiness before approving chain creation. The worker lives in the DLL, not the
renderer executable. It discovers the actual queue/chain from its own installed
factory hooks. No cooperative COM pointer or renderer-private counter is supplied.

Forwarding-only warmup requires twelve actual selected Presents with zero media
submissions before initial capture arming. The worker's own counts also verify
continued forwarding after cancellation. One-shot/sticky cancellation admission
has deterministic tests. No hooks are uninstalled or reinstalled during this work.

Six native tests pass in three consecutive repetitions:
- WARP and hardware: twenty GPU publications with increasing QPC, selected red
  pixels despite the unrelated covering HWND, retained pixels through worker stop
  and target exit. Remote worker exit is explicitly checked. The target stays alive
  after capture stop; only the test later terminates its own child.
- Wrong process creation time fails before attachment; a valid attempt then works.
  A duplicate attachment attempt is rejected.
- Integrated/non-passive producer cannot enter the loader.
- Wrong media approval terminates the helper without admitted media and without
  terminating the renderer. A failed attempt cannot be retried in that process.
- Wrong owner creation approval is terminal before chain creation.
- Target exit after factory readiness but before chain creation retires the remote
  thread; no GPU surfaces were allocated from an unobserved chain.

The recorder adapter additionally compares GPU Hello with the pinned passive
selection. Invalid attached/integrated/rearm/legacy route combinations fail before
media startup. The helper's test-process rejection happens before parameter reads
or module pinning; all four helper library regressions pass.

## Saved and decoded replay

Actual owned remote DLL loading -> shared GPU surfaces/fences -> local D3D11 ->
RecorderEngine -> MP4 -> approved FFmpeg decoding:
- Latest hardware replay: **836 ms**, fourteen decoded primary-color samples,
  NVIDIA H.264 MFT, fifteen GPU inputs, **zero encoder readbacks**.
- Latest WARP/software replay: **835 ms**, twenty-five decoded samples,
  H264 Encoder MFT, thirty-eight explicit software encoder readbacks.
- Both report zero capture readbacks. Pixel readbacks in native acceptance are
  test-only and are not hidden in the application's capture counters.

These are application counters and functional checks, not measured zero-copy,
latency/FPS or long-duration hardware-encoder claims.

## Combined regression record

A tracked all-feature native run completed with actual exit **zero**, status
**passed**, in about fourteen minutes. It passed **57 recorder tests**, plus two
native queue-owner tests, five attachment tests (before the sixth was added),
seven bootstrap, four cross-process, fifteen device lifetime/recreation/removal,
two GPU interop, sixteen hook, four basic D3D12, four resize and nine WGC tests.
The WGC minute soak observed 3,597 frames and 143 fresh checks. This covered the
new forwarding warmup, actual helper DLL and all previous owned media routes.

Record/stdout/stderr:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-remote-helper-native-8a2473f3f49e4b848fe8daa6df3592ca`.

After adding target-exit and exact adapter greeting checks, the six-test attachment
suite passed three repetitions and both attached replay/decode tests passed again.
All-feature prototype/helper Clippy and fifty-two safe prototype tests passed.
Resident-only and bootstrap-only feature Clippy also passed, preserving the old
feature boundaries. Final `scripts/check.ps1` passed formatting, workspace Clippy/
tests, frontend typecheck/lint/build and tracked-runner regressions. Strict
dependency verification (545 registry packages / eleven direct frontend packages)
and standalone settings-save regression passed. Normal desktop dependency graph
still excludes prototype/helper/MinHook. Final combined workspace log:
`C:\Users\Home\.local\share\opencode\shell\9557bddafa1dd60b88c72ff62971acb7803c3213\sh_1d42d4e85001rESWuyL3pZeT1g.out`.

After native validation, Cargo dry-run identified 667 MiB of generated prototype
cache. Only that package's generated artifacts were cleaned; desktop executable,
source, saved clips, result logs and the independent endurance snapshot were
preserved. About 1.25 GB free space remained after the final workspace checks.

Earlier attempts are **not** reported as clean full-gate passes: a multi-feature
gate was deliberately stopped when disk fell below the save reserve (result
recorded as failed). Only generated prototype artifacts were cleaned. Other runs
caught fixed 160/180-ms assumptions about twelve Presents; pressure assertions now
wait up to three seconds for the same actual progress count while requiring the
publication count to stay saturated. No capture requirement was waived. The final
tracked all-feature run above passed after these corrections. Final workspace/
frontend checks are separately recorded; do not infer them from native success.

## Boundaries and remaining work

### Legacy remote follow-up

The remote route now also explicitly selects legacy CreateSwapChain before the
owned creation handshake. All **eight** native attachment tests pass together
(the previous six plus legacy WARP and hardware). Legacy remote replay additionally
saved/decoded **843-ms hardware** and **833-ms software** clips; NVIDIA reported
fifteen GPU inputs and zero encoder readbacks, while software used thirty-eight
readbacks. This closes the earlier modern-only restriction in ADR 0075. The prior
tracked 57-recorder full run predates these two new replay tests; targeted new
native/replay validation must not be mislabeled as a new 59-test full-gate pass.

### Final combined validation including legacy remote mode

The subsequent tracked all-feature run **passed** with actual exit zero in about
sixteen minutes: **59 recorder tests, ten remote attachment tests**, and all other
native suites listed above. Record/logs:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-remote-legacy-native-1336ddefa6dd413e949867760b164b35`.
Both sixty-second remote-helper soaks passed again: 6,972 WARP/modern and 4,997
hardware/legacy publications, with retained pixels preserved through helper stop
and process exit. These are observations, not throughput or leak-profile claims.
This final run includes the two legacy replay tests that were previously targeted
only. The earlier 57-test result is retained as history, not the final total.

### Outstanding acceptance

This is **early owned-fixture attachment**, not discovery of pre-existing swap
chains in already-running games. Passive creation has an explicit approval barrier
that ordinary games do not provide. General IPC/loading, late attachment,
remote resize/replacement, in-flight device loss,
exclusive fullscreen and target-game acceptance remain unfinished. Earlier legacy
factory evidence covered integrated bootstrap only; the follow-up above is the
separate actual remote-legacy acceptance.

The owned recorder adapter owns/kills its launched fixture child on teardown, as
its existing controlled peers do; that behavior is not a production process-stop
policy. The native helper stop test separately proves continued renderer execution.
No third-party application, protected process or exclusion policy was modified.
No release/install/signing, push or PR. Changes remain uncommitted.

The eight-hour test uses its separate preserved pre-warmup snapshot, so it cannot
verify the new remote-helper route or this startup change. Its result remains
separate and must be read from durable tracking, not inferred from elapsed time.
