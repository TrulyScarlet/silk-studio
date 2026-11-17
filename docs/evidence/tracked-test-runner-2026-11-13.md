# Durable test execution records

The interrupted November 12 WGC endurance run has no final result and remains
unconfirmed. A launch or old progress checkpoint is not a successful test.

`scripts/run-tracked-test.ps1` runs one executable with explicit argument-list
entries (no shell command interpolation), a bounded timeout and separate streamed
stdout/stderr logs. Each run requires a new output directory. It atomically
replaces `result.json` with starting/running/final state, UTC timestamps, runner
identity, child PID and actual exit status. Timeout kills only the launched child
tree. Output draining is separately bounded; a stuck pipe cannot certify a pass.

`scripts/get-tracked-test.ps1` reads the record without changing original evidence.
An unfinished record whose runner PID/start time no longer matches is reported as
`interrupted_unconfirmed`, including PID reuse. It does not claim why a runner
stopped, kill unrelated processes or infer capture health from wall time.

The bounded regression script tests success, exit-code failure, timeout, stdout/
stderr recording, rejection of existing evidence and stale runner identity. It is
included in `scripts/check.ps1`. No external dependencies added.
Regression coverage also verifies live runner identity, actual nonzero exit code
and launch failure. The live-runner check caught PowerShell automatically parsing
ISO JSON strings as DateTime: string comparison misclassified a live runner. The
reader now compares UTC ticks. All regression cases passed after that fix.

An actual seven-test D3D12 bootstrap native run then completed through the tracked
runner in 18.5 seconds, exit code zero, final status passed. Record and streamed
logs: `C:\Users\Home\AppData\Local\Temp\opencode\silk-bootstrap-74b4538764da43eab4e589492e3da76e`.
Workspace/frontend checks and strict dependency verification passed earlier in the
same work session; subsequent D3D12 changes also passed all-feature Clippy, fifty
safe prototype library tests and invalid-source regression.

Limitations: atomic replacement is not a power-loss/fsync guarantee. Runner
termination can still prevent finalization; the reader detects this rather than
inventing a result. No recurring scheduler or guaranteed harness wakeup is added.
Log output streams to disk rather than accumulating in memory, but total log bytes
are not capped; callers should use bounded-output tests and preserve disk guards.
This runner validates process exit, not the semantic completeness of an arbitrary
executable's test suite. No new eight-hour capture acceptance is established.

### Sparse live-output regression

A subsequent review found FileStream's default managed buffer could hold sparse
progress lines until completion. New runs now disable that buffer. A bounded
subprocess regression observes both tiny stdout/stderr lines while the test child
is still running, then verifies its actual timeout result. This passed; merely
checking output after exit would not exercise the bug. OS power-loss durability
is still not guaranteed. The already launched eight-hour snapshot attempt uses
the earlier runner instance; its logs may buffer sparse progress, but its durable
running/final record and stale-run detection remain independent. Do not infer
current capture progress from those log timestamps or silently relabel that run
as using the newer live-output implementation.

## Snapshot endurance launch

**Resumed-check correction:** the durable reader now reports this attempt as
`interrupted_unconfirmed`, not passed or currently running. Its original record
has no finished timestamp or exit code; the recorded runner identity no longer
matches a live process. The original JSON/logs were not overwritten. Cause is
unconfirmed. No eight-hour acceptance was established, and no third unattended
run was launched. This demonstrates interruption detection, not reliable survival
of a long run across harness/provider termination.

To allow implementation builds while endurance runs, the WGC soak now accepts an
explicit development fixture path via `SILK_WGC_FIXTURE_EXE`; its existing Peer
validation still launches only the owned fixture basename, never arbitrary PIDs.
A preserved native test/fixture snapshot is stored at:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-wgc-snapshot-75590615b9894002a8d29f8fd47881a1`.
`provenance.json` records both SHA-256 hashes. The sixty-second snapshot smoke run
passed through the tracked runner: 3,597 frames, 143 fresh checks, actual exit zero.
This tests the relocated snapshot path as well as durable result finalization.

A new eight-hour attempt was launched from the same snapshot as shell job
`sh_1d3ee09b0001qeLvZnRCpMljmq`. **Launch only; no long-run result yet.** Its durable
record/logs live in `eight-hour-result` under the snapshot directory, independent
of the managed shell log. Timeout is 28,920 seconds. Do not overwrite the snapshot
or infer progress from launch time; inspect its final result/runner identity when
asked or on completion notification. The prior interrupted attempt remains failed
to establish acceptance. This attempt does not replace unfinished attachment work.
