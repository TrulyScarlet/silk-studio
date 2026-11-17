# Owned pending-work removal probes

ADR 0078; original Windows API tests, exact owned devices/fixture children only.
No driver reset/TDR is induced and no security setting is changed.

## Targeted results actually observed

1. WARP and hardware DIRECT queues wait on an unsignaled owned fence, then enqueue
   a completion signal. Its value is observed as zero immediately before explicit
   ID3D12Device5::RemoveDevice. After removal it is UINT64_MAX; the existing ring
   policy rejects that sentinel and stays terminal. Releasing the full test graph
   permits healthy same-adapter device creation. Both tests passed.
2. Separate WARP and hardware fixture children create an actual GPU capture
   bridge. The selected back-buffer copy is submitted behind the same kind of
   unsatisfied owned queue wait. Submitted count is exactly one, local polling
   yields no publication and a following queued signal remains zero before
   explicit removal. After removal polling rejects the frame and the output-copy
   and CPU-readback counters remain zero. Cancellation/drop is bounded at eight
   seconds inside a fifteen-second parent process deadline. Both tests passed
   (about 8.6 seconds total).

Each wait has an unwind guard that CPU-signals it. Removal happens immediately;
the tests do not wait for a TDR or change timeout registry settings. The second
case runs in a standalone exact fixture child so uncertain retained graphs are
released by process exit, not allowed to poison later singleton-device tests.

## What this does not prove

This is **in-flight fail-stop**, not recovery. Existing teardown intentionally
retains uncertain D3D12 renderer/capture/interoperability resources until child
exit. No allocation-budget bypass, forced free or same-process recovery is
claimed. Imported consumer work, encoder-held leases, authoritative replacement
discovery and subsequent replay resumption still require lifetime-safe recovery
implementation. Explicit removal is not real driver/TDR or real-game acceptance.

All-feature Clippy passed for the targeted initial implementation. Its combined
tracked native regression **passed, actual exit zero**, 07:16:35–07:35:06 UTC:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-resize-loss-native-d7dd529aae63451186e70dc1b81d0dfc`.
This baseline includes 64 recorder tests, 22 attachment tests, 19 device-lifetime
tests and previous native suites. It predates the retirement changes below.

## Subsequent narrow graph retirement/recreation

The earlier fail-stop behavior above is historical, not the current result for
the no-remote-consumer probe. An independent **non-shared D3D11 fence** now proves
local consumer retirement. Release of a confirmed-removed producer graph is
allowed only without remote exports and with no local work or completed,
non-uncertain local work. Neither shared nor local UINT64_MAX means completed work.

Four subsequent targeted native tests pass on WARP/hardware: after removing a
provably pending media copy, release the removed graph and recreate in the same
process using the **same six-surface budget**. Three fresh red GPU frames arrive.
The exposed-local-consumer variants retain an old local publication through
removal and recreation and verify its red pixels remain intact. About 2.5 seconds
total. Pure tests cover removal/exposure policy and independent retirement bounds.

The 24-epoch WARP resize test with six owned minimize/restore cycles also passes.
It is a fixture stress test, not VALORANT or arbitrary concurrent API acceptance.

Automatic application graph discovery, remote consumers/encoder-held leases,
replay resumption after loss and real TDR are still open. Later combined validation
must be recorded separately; the earlier baseline cannot validate these changes.

## Combined retirement validation and production denial handling

The later retirement implementation's combined native gate **passed, exit 0**,
07:43:12–08:01:30 UTC, with **64 recorder, 23 attachment and 21 device-lifetime
tests**, plus all prior native suites:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-local-loss-retirement-e5889ecb06cf4f03a8c0d2664aafe68a`.
Safe prototype tests passed 62, helper tests four. The current implementation
allows previously exposed local publications only after its independent D3D11
fence proves retirement; exported remote rings remain ineligible. Retained local
publications were verified before and after same-process reconstruction.

After that native run, a separate production engine fix makes PermissionDenied
and invalid configuration terminal during device-loss restart. It preserves the
specific error code instead of reporting generic recovery exhaustion. A mock
regression verifies exactly two start calls (initial success and one rejected
recovery), unchanged selected source IDs, terminal Error state and specific codes.
All 26 engine tests pass. This does not retry around VALORANT's capture exclusion.

Final workspace formatting/Clippy/tests, frontend typecheck/lint/build,
all-feature and auto-helper/late-helper feature Clippy, strict dependency checks,
settings-save and tracked whitespace checks passed:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-resume-final-02fa1482ec8d497ab173bfd53dd82479.log`.
The later engine error-path fix is covered by those tests, not retroactively by
the earlier native result.

Subsequent bounded same-process stress also passed: ten pending-copy removals and
local-consumer reconstructions per WARP/hardware child (twenty total), with old
pixels retained through each transition and fresh pixels checked each cycle.
The fixed ten-cycle CLI mode rejects unrelated flags; no unbounded count is
accepted. These two targeted tests were added after the combined 21-test device
run above, so are not retroactively included in that result. All-feature Clippy
passed again. This is short reconstruction stress, not eight-hour endurance.
