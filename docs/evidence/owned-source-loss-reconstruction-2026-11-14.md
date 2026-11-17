# Owned manual-source typed-loss reconstruction

ADR 0081. Original exact-owned hardware source only; no hooks, DLL load or game.

## Targeted results

- Adapter: two losses/reconstructions in the same process and retained HWNDs,
  red → green → blue pixels, increasing PTS, original local red lease surviving
  both transitions and stop. No parent capture resize command.
- RecorderEngine: three MP4s saved/approved-FFmpeg-decoded at 318x178, 640x360 and
  800x450. Decoded primary-color samples: 16, 23 and 23 in the first targeted run.
  Zero reported encoder/capture readbacks.
- Source controls: wrong retirement/approval and missing proof deadlines return
  **normal error exit one**. No media before approval. Both stop boundaries exit
  normally and retain independent old red pixels.
- Before-first-ACK recovery request is rejected without removal. WARP is rejected
  before graphics startup, also with normal error exit one.
- Retained parent contexts obey six surfaces; a third allocation fails instead
  of increasing the limit. Stop at the old-frame delivery boundary is below two
  seconds in the targeted test.
- Safe tests cover exact profile names and one-shot session/slot/sequence ACKs.
  All-feature/cross-only Clippy passed. Full new combined regression is pending.
- Twenty successive same-process hardware loss/reconstruction epochs pass with
  the original red local pixels retained, fresh green/blue pixels, increasing
  PTS, unchanged target and unchanged budgets. About 10.5 seconds in the targeted
  adapter test; short stress, not long-run/endurance acceptance.

The source owns its graph and calls real renderer APIs after retaining HWND
guards and releasing the old graph. The parent retires old imports with a
non-shared D3D11 fence, never a removal sentinel, and validates the fresh exact
epoch before approval. The separate terminal profile still stops without a
silent new source.

## Limits

Not independent DLL-helper or third-party replacement observation. The source is
a manual hardware fixture and owns reconstruction. Its probe proves a pending
queue signal, not a partial consumer-copy failure or TDR. Real-game/fullscreen,
audio, release and long-run recovery acceptance remain open. WARP remote loss
remains unsupported after earlier native faults. The separate two-hour WGC
snapshot cannot validate endurance of this newly changed recovery mode.

No security/driver/game setting change, anti-cheat/exclusion bypass, monitor
fallback, installed replacement, commit/push or PR.

Combined native regression subsequently **passed, actual exit 0**, 14:21:41–
14:42:23 UTC: 73 recorder, 23 attachment, 30 device-lifetime and fourteen cross-
process tests plus previous suites:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-loss-reconstruction-native-e68e6b2689d34afeb1a3b53100cc34a7`.
Safe/workspace/frontend, feature matrix, strict dependency and settings gates pass:
`C:\Users\Home\AppData\Local\Temp\opencode\silk-reconstruction-checks-a22382c58d2c4ed593aa6522c00c7d07.log`.
