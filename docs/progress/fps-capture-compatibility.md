# FPS capture compatibility plan

Research date: 2026-11-12. **All five game families remain unverified in Silk.**
This is a planning/evidence document, not permission to attach the experimental
helper to protected games or a guarantee against account penalties.

## Current architecture

- Production game/window selection uses window-targeted Windows Graphics Capture
  (WGC), not the experimental helper. Isolated selection must never fall back to
  a monitor: desktop exposure on Alt-Tab is a failure.
- ADRs 0069–0070 bind new selected-window IDs to process creation time, preserve
  WGC encoder leases/resize resolver gaps, reject queued media after target loss
  or exclusion, and permit stoppable Idle periods. Odd RGB window surfaces now
  feed GPU conversion with even NV12 output; HardwareOnly discovery excludes
  software transform classes. Native owned-window lifecycle and production-route
  replay/decode pass, not real-game acceptance. Legacy selections require explicit
  reselection. Evidence: `../evidence/production-wgc-lifecycle-2026-11-12.md`.
- The default-off helper captures bounded BGRA8 D3D11 surfaces in an owned fixture.
  Its loader accepts only the passive child it launched. Replay/MP4 decoding on
  WARP and RTX 2080 Ti is fixture evidence, not protected-game evidence.
- Default-off D3D12 fixtures now cover offscreen/windowed swap-chain capture,
  same-adapter D3D12/D3D11 shared-fence interop, owned-child GPU transport,
  actual Present detouring and recorder/replay decode (ADRs 0054–0060).
- ADRs 0062–0064 add bounded canonical queue ownership and actual owned
  CreateSwapChainForHwnd interception. The renderer calls DXGI directly; capture
  adopts the observed queue rather than accepting a queue at claim time.
  WARP/hardware replay, native faults, window states and short soaks pass.
- ADR 0065 adds independent **in-process owned-startup** bootstrap: worker creates
  its own factory, receives only PID/lifetime/HWND, discovers graphics objects,
  waits for twelve renderer Presents, then initializes media/Present capture.
  WARP/hardware native tests and replay/decode pass. This is not remote loading or
  arbitrary already-running application attachment.
- Production D3D12 routing, remote D3D12 loading, pre-existing-chain
  discovery, other DXGI creation variants, device-loss rearm, general-game
  authenticated IPC and protected-game acceptance remain unimplemented. The
  factory observation still starts before the renderer's creations. Selection is
  exact HWND metadata, not automatic foreground/desktop capture.
  Latest evidence: `../evidence/d3d12-worker-bootstrap-2026-11-12.md`.
- ADR 0066 adds controlled same-chain live ResizeBuffers recovery in the owned
  bootstrap source, with bounded epoch/handle replacement and ordered recorder
  context refresh. Three resize replay epochs save/decode on WARP/hardware.
  This is explicit test control, not automatic third-party resize interception,
  device replacement/loss or game/fullscreen acceptance. Evidence:
  `../evidence/d3d12-live-resize-2026-11-12.md`.
- ADR 0068 adds explicit healthy device-graph teardown/recreation in the same owned
  process/HWND. Worker/renderer references are released before new creation; fresh
  observed association, GPU handle epoch and ordered recorder context are required.
  WARP/hardware replay/decode and failure/repeated-epoch tests cover this route,
  not actual device removal or driver reset. Evidence:
  `../evidence/d3d12-healthy-recreation-2026-11-12.md`.

## Per-title assessment

VALORANT's explicitly selected window reported WDA_EXCLUDEFROMCAPTURE (17).
Two WGC attempts failed before frames at CreateForWindow (0x80070057). ADR 0061's
diagnostic now confirms PermissionDenied, no backend/context, zero frames and no
fallback. This configuration is blocked, not supported; no attribution to Vanguard
or universal title claim is made. See the separate evidence record below:
`../evidence/d3d12-owned-hook-and-valorant-2026-11-12.md`.

| Target | Official evidence and limits | Concrete next gate |
| --- | --- | --- |
| Fortnite | Epic says EAC is required [1]. Current rendering-mode guidance lists DX12 and DX12 Performance [2]; older legacy modes do not identify the user's active renderer. BattlEye's general overlay guidance permits game-specific restrictions [3], not Silk approval or proof of the current session's stack. | Record exact build/mode without changing settings. Test the existing non-injected window route separately. Develop D3D12 only in owned fixtures before considering any permitted game integration. |
| VALORANT | Riot's third-party policy concerns game integrity [4]. Riot's Vanguard developer FAQ says there is no allowlist and DevRel cannot make exceptions [5]; its detailed rollout discussion is largely League/TFT, not blanket VALORANT capture authorization. Published specs require DX11 support [6], not acceptance of our hook. | Observed WGC configuration blocked by window capture exclusion; report unsupported and preserve it. Seek clarification of capture-specific restrictions where needed; do not presume an approval/allowlisting program exists. |
| Counter-Strike 2 | Valve's Trusted Mode FAQ says foreign software is blocked and there is no safe-injection whitelist [7]. Signing alone does not enable injection in Trusted Mode. This is CS2, not legacy CS:GO. | Preserve default Trusted Mode. Evaluate non-injected window capture; do not change launch/security options to make the hook load. Confirm whether the requested target is CS2 or legacy CS:GO. |
| Call of Duty | Activision's policy prohibits security circumvention and unauthorized software used for cheating/modification, including injectors [8]. It does not establish a public recorder approval pathway. Current Warzone requirements specify DX12 compatibility [9]. | Identify the exact title/build first. WGC acceptance and owned D3D12 development are separate tracks; Warzone evidence cannot establish every historical CoD title. |
| Overwatch | Blizzard announced DX12 beta alongside continued DX11 support in 2025 [10]; that announcement does not establish today's selected runtime. Defense Matrix guidance restricts cheating/integrity violations [11], not approval of custom capture injection. | Record exact build/API. Evaluate WGC independently; any future hook integration needs its own policy and runtime evidence. |

## Implementation sequence

1. **Owned D3D12 fixture and transport design.** Document queue selection, resource
   state ownership, bounded shared textures/fences and nonblocking frame dropping
   before implementation. Capture/encoding must not wait on UI or disk. Test
   resize, stale epochs, full queues, cancellation, device recreation and retained
   surfaces on WARP/hardware, then save and decode replay clips. Do not adapt the
   D3D11 keyed-mutex contract blindly to D3D12. No protected-process attachment.
2. **Non-injected acceptance, Fortnite first.** Explicitly select the game's
   process/window in the existing UI with anti-cheat/security settings unchanged.
   Record backend, build, Windows/driver/GPU, rendering mode and display mode.
   This validates WGC only, even if it happens to work while a game says fullscreen.
3. **Production lifecycle and target identity.** Design authenticated bounded IPC,
   PID + creation-time + HWND validation (production source IDs currently lack
   creation time), rearm, process-exit handling and packaging/signing. Preserve
   aspect ratio, source-save/retry behavior and background-window detection.
4. **Per-title integration gate.** Review applicable published restrictions and
   any vendor response before selecting a permissible route. Signing is package
   provenance, not universal permission. If blocked, report unsupported; never
   weaken anti-cheat, Trusted Mode, antivirus or Windows policy, or disguise loading.
5. **Release gate.** Run the acceptance checklist below on each claimed route and
   title/build; retain regression artifacts. No blanket “anti-cheat compatible” claim.

## Acceptance checklist and evidence rules

For every title/build/backend/API/display-mode combination:

- Save/decode replay with expected game content and aspect ratio; check timestamps,
  duration, GPU/CPU readback counters and bounded memory under encoder pressure.
- Cover selected game behind another application, Alt-Tab, minimize/restore,
  resolution change, display change, restart and exit. Never show desktop or another
  application's pixels; stop/drop frames when the source is unavailable.
- Distinguish windowed, borderless and **actual exclusive fullscreen**. A settings
  label alone is insufficient to prove exclusive presentation. Record how verified.
- Test with the normal anti-cheat/security configuration unchanged. Record failures,
  blocked modules, instability and missing frames; stop rather than retrying bypasses.
- Keep policy evidence, fixture tests, WGC observations and hook observations in
  separate fields. Runtime success is not vendor endorsement or account safety.

No protected-game capture test, account-safety test, exclusive-fullscreen test or
vendor approval was obtained during this research. No running game/settings were
modified. The connected browser was unavailable; Valve and some Riot/EAC pages
returned page chrome, so the noted findings there rely on official-domain search
excerpts rather than full rendered-page verification. Recheck policy before release.

## Vendor clarification draft (not sent)

Describe Silk as an original user-selected replay recorder. State the exact proposed
route (OS window capture versus a separately specified user-mode graphics helper),
graphics API and lifecycle. Ask which published restrictions apply and whether a
documented third-party capture compatibility/contact process exists. Offer build
hashes, signing identity and minimal reproduction only if requested. Do not claim
to have approval, request an anti-cheat exception, or treat a support reply as a
universal guarantee. Riot game-data API registration is not capture approval; EAC
game-studio licensing is not an established third-party-recorder approval program.

## Official sources

1. [Epic: Fortnite requires EAC](https://www.epicgames.com/help/fortnite-battle-royale-c-34254770/technical-support-c-38015632/how-do-i-install-easy-anti-cheat-on-my-computer-to-play-fortnite-a20611857)
2. [Epic: rendering-mode options](https://www.epicgames.com/help/c-34254770/a22616652?lang=en-US)
3. [BattlEye FAQ](https://www.battleye.com/faq/)
4. [VALORANT third-party applications](https://support-valorant.riotgames.com/hc/en-us/articles/38353516078227-Third-Party-Applications)
5. [Riot developer Vanguard FAQ](https://support-developer.riotgames.com/hc/en-us/articles/28021427366163-Vanguard)
6. [VALORANT PC specifications](https://support-valorant.riotgames.com/hc/en-us/articles/360044136134-Minimum-Recommended-PC-Specs)
7. [Valve: CS2 Trusted Mode](https://help.steampowered.com/en/faqs/view/09A0-4879-4353-EF95)
8. [Activision enforcement policy](https://support.activision.com/articles/call-of-duty-security-and-enforcement-policy)
9. [Warzone PC requirements](https://support.activision.com/warzone-2/articles/pc-system-requirements-for-warzone-2)
10. [Blizzard: Overwatch DX12 introduction](https://overwatch.blizzard.com/en-us/news/24173128)
11. [Blizzard: Defense Matrix](https://overwatch.blizzard.com/en-gb/news/24173281/defense-matrix-keeping-overwatch-2-fair-and-fun-for-every-player)
12. [Microsoft: WGC screen capture](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture)
13. [EAC support and developer FAQ routing](https://www.easy.ac/support/articles)
