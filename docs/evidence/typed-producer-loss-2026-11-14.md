# Typed producer-loss and delivery-boundary evidence

ADR 0080; original exact-owned manual hardware fixture only.

## Implemented and targeted validation

- Fixed-size DeviceLost and TriggerOwnedLoss messages: exact session metadata,
  known HRESULT reasons, reserved/truncated/malformed byte rejection.
- Deterministic loss-epoch tests: fifty sequential epochs, no offer before exact
  retirement, no approval before valid offer, stale target/lifetime/HWND/generation/
  epoch, overlap, invalid dimensions/handles/adapter/QPC and exhaustion fail closed.
- Independent consumer tail tickets strictly exceed all seen/possibly attempted
  signals; neither overflow nor UINT64_MAX can become a successful work ticket.
- Five hardware/unsupported native tests pass: actual producer removal preserves
  an already-delivered frame's resolver gap, retires at the explicit boundary,
  retains independent red pixels through normal source exit, cannot mask stale
  headers, and does not change default fail-stop semantics. Wrong trigger session
  and unsupported WARP exit with normal error code one, never crash-as-success.
- Adapter delivery-boundary and stop-at-boundary hardware tests pass. Its loss
  mailbox cannot resize or recreate, and unsupported configurations fail before
  worker launch.
- RecorderEngine hardware test saves an approved-FFmpeg-decoded red 318x178 MP4
  before loss, then reports terminal capture failure without a new source,
  recovery restart or monitor fallback. Reported encoder readbacks remain zero.
- All-feature Clippy and current safe tests pass. Combined regression after this
  new boundary is a separate required result, not inherited from ADR 0079.

The deliberately pending item in this probe is a queue signal, not proof that a
capture copy or consumer copy was pending at the removal instant. Holding actual
queued frame headers proves rejection after removal and caller-boundary behavior.
Partial-consumer API failure is covered by conservative state/tail logic, not a
native forced API failure test. The producer retains its removed graph until child
exit; automatic reconstruction or replay resumption after this loss is unfinished.

No third-party process, hook injection, TDR, security/game/driver setting change,
release artifact or installed-app replacement. VALORANT is not targeted here.

The subsequent combined native gate for this terminal-boundary implementation
**passed, actual exit zero**, 13:34:16–13:53:41 UTC: 68 recorder, 23 attachment,
30 device-lifetime and nine cross-process tests, plus previous native suites:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-typed-loss-native-5d4749d610674385b188101c980fbc52`.
Workspace/frontend, feature matrices, dependencies and settings-save also passed:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-typed-loss-checks-77e42e60a044410aa779510a9cd22918.log`.
This run predates the separate ADR 0081 automatic manual-source reconstruction.

## Separate instrumented endurance snapshot

A sixty-second WGC fixture smoke soak passed 3,602 frames / 144 fresh color
checks and retained its original red pixels through stop. Process memory/handle
samples were added using standard user-mode Windows APIs. CPU private bytes,
working set, OS peak working set and sampled handle counts are diagnostic only;
they are not GPU-memory or throughput measurements.

A **two-hour owned-window WGC** run was launched from immutable test/fixture
copies under:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-owned-soak-6bb1f1ad1cdc47f3ab6dd94566af0868`.
The manifest records SHA-256 hashes and intended duration; the atomic tracked
record and split logs are in `result`. Completion is pending, not assumed. This
does not establish an eight-hour, real-game, packaged-app or audio-sync pass.
It runs independently of workspace rebuilds. The two older overnight attempts
remain interrupted/unconfirmed and are not overwritten or reclassified.

### Confirmed completion

The immutable two-hour snapshot completed successfully, actual exit zero,
2026-11-14 13:23:25–15:23:26 UTC. Its stdout records **431,962 frames** over
7,200 seconds and **17,278 fresh magenta checks**, with original red pixels
retained through stop and no monitor capture.

One-minute sampled resource maxima (bytes): parent private 61,030,400, working
set 46,964,736, handles 400; fixture private 15,937,536, working set 27,947,008,
handles 285. Baselines: parent private 58,511,360, working set 44,068,864,
handles 396; fixture private 15,888,384, working set 27,856,896, handles 282.
These are process diagnostics, not GPU-memory, leak-freedom or throughput proof.
This pass does not establish eight-hour, real-game, audio, packaged release or
endurance of the subsequently implemented helper-loss mode.
