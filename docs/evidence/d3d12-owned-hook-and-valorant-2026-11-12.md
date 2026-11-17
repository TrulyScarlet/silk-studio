# Owned D3D12 milestones and separate VALORANT diagnostic

Date: 2026-11-12. Development-only; changes remain uncommitted.

## Owned fixtures (ADRs 0057–0060)

`scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode` passed:
33 feature-enabled library tests, six safe recorder tests, three helper tests,
native suites and 30 native recorder tests (20 WARP/10 hardware).

Transport validates launched PID/creation time/HWND, LUID, QPC and five shared
handles. Three texture slots require exact ACK and completed consumed fence before
reuse. Private bounded pipe workers stay outside Present. Tests cover full rings
with continued rendering, malformed/truncated packets, stale ACK, EOF, producer
death, local saturation, retained completed pixels and queued-media rejection.
Six-output-surface bounded restart passed three repeated regression runs.
Parent-owned retirement fencing avoids depending on shared fences after child
death; rejected output admission is not marked uncertain GPU submission.

Actual resident Present detouring passed selected-only pixels, unrelated forwarding,
cancellation and WARP/hardware replay/decode. Minimize/restore and monitor-sized
borderless/windowed transitions required three fresh post-action-QPC frames with
changed expected colors per phase. Each driver passed 500-publication short soaks
with two consumer stalls and ten explicit pixel checks.

Final full-gate hooked hardware replay: 834 ms, 22 NVIDIA H.264 GPU inputs, zero
capture/encoder CPU readbacks, 20 decoded red samples. Manual-boundary hardware:
842 ms, 18 GPU inputs, zero readbacks. Hooked WARP: 841 ms, zero capture readbacks,
37 software-encoder readbacks, 24 decoded red samples. Counters do not establish
zero-copy or vendor-internal behavior. Approved FFmpeg SHA-256:
`57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A`.

## VALORANT: blocked, zero frames (ADR 0061)

User authorized testing the running game. Only query-limited inspection and OS WGC
were used. Selected PID 25976, HWND 15467458, creation FILETIME 134389450895967168,
image VALORANT-Win64-Shipping.exe. Valid top-level background non-minimized window,
2560×1440; build/API not observed. Two attempts failed before frames at
GraphicsCaptureItem.CreateForWindow with 0x80070057. Read-only affinity query
returned 17 (0x11, WDA_EXCLUDEFROMCAPTURE); which component set it is unknown.

The new exclusion diagnostic passed by confirming PermissionDenied, no backend,
no GPU context, zero frames, no monitor fallback, no injection and no game/security
changes. JSON evidence:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-valorant-wgc-20261112-034107-7049\evidence.json`.
No screenshot/video captured. This is blocked-result validation, not support.

## Validation and limits

After exclusion changes: capture-windows Clippy passed; 13 unit and two safe
integration tests passed; explicit blocked diagnostic passed. Full check.ps1
passed Rust formatting/Clippy/workspace tests and frontend typecheck/lint/build.
Settings-save regression, strict dependency verification (545 Cargo registry
packages/11 direct frontend dependencies), all-feature prototype Clippy and
git diff --check also passed. No owned fixture processes remained at final check;
approximately 2.6 GiB free space remained.
Earlier save tests correctly rejected low disk space. Cleaning only generated
capture-hook-prototype package artifacts recovered 2.7 GiB; unchanged disk guards
and save integration tests subsequently passed. Source/clips were not deleted.

Fixture still supplies known DIRECT queue and selected swap chain. Generic queue
discovery, independent bootstrap, production integration and live rearm remain
unimplemented. Window tests do not prove exclusive fullscreen, FSO/independent
flip, real Alt-Tab or game behavior. Short soaks are not performance/endurance
benchmarks. No protected game was hooked; all five game families remain without
verified hook compatibility or account-safety evidence. No commit/release created.
