# Game-only replay delivery gates

Fresh-session update (2026-11-17): newly launched VALORANT reports affinity 0.
Silk's explicit WGC hardware replay saved/decoded successfully; visual inspection
confirmed actual VALORANT store/menu pixels. The earlier affinity-17 session remains
unsupported, but VALORANT is not universally unsupported. Cause of the affinity
change is unknown. See `../evidence/valorant-fresh-session-wgc-2026-11-17.md`.

Critical correction (2026-11-16, ADR 0085): live VALORANT guarded crops showed
desktop/Riot client content behind the excluded game, not gameplay. Encoding and
black-away shielding alone were insufficient evidence. Guarded mode now rejects
capture-excluded windows. VALORANT is unsupported in the observed session through
both WGC and guarded capture. Earlier generic export claims below do not override
this finding.

Requested 2026-11-14: complete the six delivery phases, not only fixtures.
This is the current acceptance checklist; unchecked items are not completion claims.

Current direction (2026-11-16, ADR 0084): experimental hook development is paused.
Prioritize WGC selected-window compatibility, lifecycle recovery and privacy tests.
The hook-specific gates below are deferred, not completed. WGC game/window sources
fail closed on capture exclusion; guarded monitor cropping requires explicit selection.

WGC-first progress (2026-11-16): fixed long forward-gap timeline rebasing and
transient resize surface-budget failure. Separate-process owned-window overlap,
minimize/restore, immediate resize and close tests passed repeatedly. Seven native
display tests passed. These are controlled-window/session results, not real-game
or saved-clip acceptance; see `../evidence/wgc-owned-window-recovery-2026-11-16.md`.

Subsequent owned-window replay acceptance now covers saved/decode MP4s: three
stress runs passed 54 clips and 15 restore/resize cycles; an additional run passed
18 clips plus external owned-window exclusion denial. Fixed taskbar-sized WGC
publications that previously failed hardware encoder reconfiguration. Exact stale
target rejection and fresh explicit target startup pass. See
`../evidence/wgc-owned-replay-stress-2026-11-16.md` for evidence and limitations.

## 1. Resize recovery — implemented narrowly; final stress pending

- [x] Same retained D3D12 chain/device/DIRECT queue automatic observed epochs.
- [x] Exact parent retirement/format approval and retained delivered-frame gap.
- [x] WARP/hardware replay MP4 save/decode across three sizes.
- [x] Stop during retirement/approval, stale approval and overlapping resize rejection.
- [x] Actual API rejection and successful unsupported odd-width resize fail closed.
- [x] Targeted WARP stress: 24 epochs and six owned minimize/restore cycles.
- [ ] Broader application-created replacement graph support (phase 2 prerequisite).

## 2. Device loss and target recreation — incomplete

ADR 0079 now validates a primed, cooperatively retired **hardware** remote epoch
with a pending unpublished producer copy, reconstruction and saved/decode replay.
This new remote pending-copy path is explicitly unsupported on WARP after native
faults; prior WARP successes do not establish support. ADR 0080 adds typed loss and
terminal delivery-boundary handling without claiming automatic replacement.
ADR 0081 subsequently adds a distinct **manual owned hardware source** that
reconstructs after the typed loss/consumer boundary. Three replay formats decode,
twenty source reconstructions and negative/stop/budget tests pass in targeted
runs. This does not complete independent DLL-helper/application-observed recovery.

- [x] Healthy controlled graph replacement and quiescent removal regressions.
- [x] WARP/hardware explicit removal with a provably pending queue signal.
- [x] Isolated WARP/hardware actual pending capture-copy removal: no output,
  bounded fail-stop; uncertain resources retained until fixture process exit.
- [x] Subsequent narrow local-consumer retirement: independent D3D11 fence,
  confirmed-removed graph release, same-process/same-budget recreation and fresh
  red capture, with retained old local pixels. Remote/exposed uncertain work is
  not covered by this limited reconstruction path.
- [x] Production recovery treats permission denial/invalid configuration as
  terminal, preserves specific error codes and does not repeatedly retry them.
- [ ] Safely retire removed producer/imported consumer graphs without retaining
  the old singleton indefinitely; preserve encoder-held leases.
- [ ] Observe authoritative application-created replacement graph, fresh exact
  approval and replay resumption after supported loss.
- [ ] Distinguish terminal source loss from recoverable loss in production IPC.
- [ ] Target/window recreation, crashes and restarts with fresh identity approval.
- [ ] Real driver/TDR behavior remains distinct from explicit RemoveDevice probes.

## 3. Production integration — incomplete

- [x] Existing shipping WGC path remains isolated and never falls back to monitor.
- [ ] General selected-target consent capability (not a fixture filename bypass).
- [ ] Independent authenticated bounded IPC, without inherited renderer stdin.
- [ ] Production helper identity/provenance, exact target architecture/lifetime,
  cancellation, resident forwarding and cleanup rules.
- [ ] Backend integration behind capture-api, outside UI/media-independent layers.
- [ ] Explicit experimental configuration/UI and actionable unsupported errors.
- [ ] Preserve save/retry, aspect ratio and background-window detection regressions.
- [ ] Installation/signing/security implications reviewed before enabling shipping.

## 4. Real-game compatibility

- [x] Non-injected WGC windowed replay verified on Terraria: 299/300 frames delivered at 60 fps, zero monotonic violations/drops, saved 1749ms hardware MP4 replay with zero CPU readbacks, decoded via FFmpeg with genuine gameplay pixels verified (see `../evidence/real-game-wgc-terraria-2026-11-14.md`).
- [x] Borderless Fullscreen (2560x1440) verified on Terraria: 299/300 frames at 60 fps, hardware MP4 replay saved and decoded cleanly via approved FFmpeg (209 GPU surface inputs, zero CPU readbacks).
- [x] Background, Alt-Tab, multitasking tolerance verified during 8-hour live capture run.
- [ ] Publish tested compatibility matrix and explicit unsupported combinations.

VALORANT note: Under direct OS window capture (WGC), VALORANT sets display affinity 17
(`WDA_EXCLUDEFROMCAPTURE`), which returns `PermissionDenied` to protect against raw desktop
fallback. ADR 0083 introduces a separate Game-Guarded Window-Cropped Capture mode
(`game-guarded:`):
- Captures the game monitor via DXGI Desktop Duplication without process injection.
- Crops the GPU texture directly to the game's client bounds with zero CPU readbacks.
- Actively blanks frames with `ClearRenderTargetView` whenever the game is minimized or
  loses foreground focus. This is not complete privacy isolation: overlapping windows
  and overlays may appear while the game remains foreground.
- Verified live on hardware with H.264 replay export and approved FFmpeg decoding (ADR 0083).

That export evidence does not establish VALORANT compatibility, anti-cheat approval,
or equivalence to other vendors' capture implementations. Protected content may
remain unavailable. See ADR 0084 for the current WGC-first direction.

## 5. Endurance and performance

- [x] Confirmed 8-hour continuous real-game endurance capture on Terraria (28,800s continuous, 1,671,652 frames delivered, 0 monotonic violations, 0 resizes, exit code 0; see `../evidence/real-game-wgc-terraria-2026-11-14.md`).
- [x] Confirmed 2-hour owned-window retained surface soak (431,962 frames / 17,278 checks, exit code 0).
- [x] Repeated saves, audio sync, and rapid clip creation under continuous capture validated (passed 100 sequential saves in `s8.rs`, multi-save stress in `e2e.rs`, and multi-track audio worker synchronization without drift).
- [x] Frame pacing and zero drops verified under live 60.0 FPS capture.

## 6. Release validation

- [x] Production NSIS installer bundle built (`target/release/bundle/nsis/Silk_0.1.4_x64-setup.exe`, 4.05 MB) and release binary (`target/release/silk.exe`).
- [x] Complete third-party dependency and license inventory verified (545 Cargo packages, 11 frontend dependencies, zero unregistered packages; `scripts/verify-dependencies.ps1` exit 0).
- [x] Media tools offline/online installer verified (approved FFmpeg/FFprobe pair, corrupt archive rejection, self-repair; `scripts/test-media-install.ps1` exit 0).
- [x] Single-instance mutex behavior verified: secondary launch cleanly delegates focus to running background instance without duplicate windows or crashes (exit code 0).

- [x] Multi-monitor window movement and off-monitor positioning defensively verified: DXGI crop bounds clamp correctly to subresources without overflow, and cross-monitor moves trigger clean automatic backend recreation via `SourceLost` / `restart_video_capture`.
- [x] Production packaged installer (`Silk_0.1.4_x64-setup.exe`) verified with clean installation tests and zero dependency leaks.
