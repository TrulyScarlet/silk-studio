# Eight-hour owned-window WGC endurance run — interrupted/unconfirmed

- Requested scope: continue implementation/testing without stopping at each milestone.
- Status: **not running; no completion result and no eight-hour pass**.
- Shell job: `sh_1d200b684001WSwyxkjRyFW0Mr`.
- Checked 2026-11-13 after user asked about the elapsed time. The expected completion notification did not arrive.

## Interruption findings

The log's final checkpoint is 1,800 seconds, 108,034 frames and 4,321 fresh checks.
Its last modification was 2026-11-12 18:33:45 local time. At 2026-11-13 02:45 local
time neither the native test, owned child nor Cargo was running. There is no final
test summary/exit result in the managed log. The cause is unconfirmed; a bounded
System event-log query returned no candidate sleep/shutdown records, which does
not prove why the processes ended. Do not attribute this to a capture defect or
harness termination without additional evidence.

The earlier chat status incorrectly treated the stale thirty-minute checkpoint
as current progress. Eight hours of wall time do not establish eight hours of
successful capture. This run provides only the logged checkpoints, not endurance
acceptance. Do not rerun an unattended eight-hour job until durable result/exit
recording and interruption detection are in place.

Command:

```powershell
$env:SILK_WGC_SOAK_SECONDS = '28800'
cargo test -p capture-hook-prototype --features d3d12-bootstrap-fixture --test wgc_owned_native wgc_owned_retained_surface_soak -- --ignored --test-threads=1 --nocapture
```

Managed output log:
`C:\Users\Home\.local\share\opencode\shell\9557bddafa1dd60b88c72ff62971acb7803c3213\sh_1d200b684001WSwyxkjRyFW0Mr.out`.

The test selects the exact owned bootstrap child by PID, process creation time
and HWND. It captures only that window through production WGC. It retains an
observed red texture lease, then changes the owned renderer to magenta and checks
old retained pixels, fresh pixels and monotonic timestamps throughout. It prints
bounded progress every fifteen minutes. No monitor capture, real game targeting,
policy/security changes, media upload or clip recording is involved.

The child has an explicit strictly parsed lifetime of test duration plus thirty
seconds, capped at 28,830 seconds; its owner has a further five-second bound. The
default owned-fixture lifetime remains 175 seconds. Extended lifetime is accepted
only on the exact owned bootstrap route. Duration parser tests passed, as did the
60-second version with 3,602 frames and 144 fresh checks after this parameterization.
All-feature Clippy and fifty safe prototype library tests passed afterward.

This run is continuous retained-surface/pixel/timestamp testing, **not** a claim of
eight-hour resource/handle profiling, audio synchronization, game compatibility or
hardware-encoder endurance. Lock/sleep/source failure can terminate it; do not
reinterpret an interrupted run as passing. On completion, inspect the notification
and record actual duration, counts, result and remaining processes before updating
this status. Do not clean or replace the running prototype artifacts meanwhile.

Running-build provenance (SHA-256, captured without polling job output):
- Native test executable: `BBB35C4633BD09361ED9585D90497D7367EE88666A77DB03ED37F7FECDF812CB`.
- Owned child executable: `9335E50DB030A35F5D028DAE27C18D34A53AE114AD9D86DD0CCC994D24B49B7D`.

The desktop development binary remains `target/debug/silk.exe` (NotSigned), with
SHA-256 `FF8F59C61C440E5EF0710D7FE4E11DEF0E41FC8231E51DCCE4BF688A80C10FD1`.
It was built with embedded frontend assets, not installed or auto-launched. Normal
desktop dependencies exclude prototype/helper/MinHook. All changes are uncommitted.
