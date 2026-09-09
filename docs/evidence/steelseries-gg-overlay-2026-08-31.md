# SteelSeries GG Overlay Architecture & Comparison Report (2026-08-31)

## Executive Summary & Scope

This report records factual technical metadata gathered from the locally installed
SteelSeries GG application and public SteelSeries documentation to compare its
overlay topology against Silk's native DirectComposition capture confirmation
HUD.

**Scope and constraints:**
- Investigation is limited strictly to public documentation, non-invasive Win32
  process/module queries, and window inspection APIs.
- **No decompilation, disassembly, memory dumping, graphics hooking, anti-cheat
  bypass, process injection, or proprietary code/asset copying was performed.**
- Findings represent a single local installation and public documentation state
  as of 2026-08-31.

---

## Local Installation & Process Topology

- **Product Version:** SteelSeries GG `118.0.0.0`
- **Authenticode Signer:** `GN Hearing A/S` / `SteelSeries` (Valid signature)
- **Host Process:** `SteelSeriesGG.exe`
- **Loaded Presentation Modules:**
  - `SteelSeriesGameOverlay.dll` (v0.1.0.0)
  - `d3d11.dll` (Direct3D 11 runtime)
  - `dxgi.dll` (DirectX Graphics Infrastructure)
  - `dcomp.dll` (DirectComposition API)
  - `d2d1.dll` (Direct2D runtime)
  - `DWrite.dll` (DirectWrite runtime)

Capture functionality is isolated into a separate out-of-process service
(`SteelSeriesCaptureSvc.exe`). Official documentation specifies capture modes
including "Game Capture (WGC)" and "Screen Capture". Windows Graphics Capture
(WGC) is a standard non-invasive OS desktop/window capture API and does not
indicate graphics process injection.

Regarding game process interaction:
- Module enumeration on `VALORANT-Win64-Shipping.exe` was blocked by Riot
  Vanguard; injection status is **Unknown**.
- There is no empirical or architectural evidence supporting notification overlay
  injection into target game processes. This statement notes the absence of
  evidence and does not claim proof of absence.
- The SteelSeries GameSense SDK is an external REST/JSON event protocol for
  peripherals and game state integration, not an on-screen visual overlay
  injection framework.

---

## Window Hierarchy & Style Attributes

Inspection of the top-level window owned by `SteelSeriesGG.exe` revealed the
following properties during idle state:

- **Window Handle (HWND):** `0x161220` (*ephemeral observation-time value*)
- **Window Class / Title:** `GameOverlay` / `GameOverlay`
- **Window Geometry:** `0, 0` to `2560, 1440` (full desktop resolution)
- **Visibility State:** `IsWindowVisible = false` (hidden while idle)
- **Window Style:** `0x84000000` (`WS_POPUP | WS_CLIPSIBLINGS`)
- **Extended Window Style (`ExStyle`):** `0x08280028`
  - `WS_EX_TOPMOST` (`0x00000008`)
  - `WS_EX_TRANSPARENT` (`0x00000020`)
  - `WS_EX_LAYERED` (`0x00080000`)
  - `WS_EX_NOREDIRECTIONBITMAP` (`0x00200000`) — *Note: `0x00200000` is `WS_EX_NOREDIRECTIONBITMAP` in modern Windows SDKs, not `WS_EX_COMPOSITED`.*
  - `WS_EX_NOACTIVATE` (`0x08000000`)

---

## Architectural Comparison: SteelSeries GG vs. Silk Native HUD

| Dimension | SteelSeries GG `GameOverlay` | Silk Native HUD (Production) | Silk Native HUD (`detached-shown` Diagnostic) | Silk Native HUD (`attached-hidden` Diagnostic) |
|---|---|---|---|---|
| **Rendering Stack** | Direct3D 11 + DirectComposition + Direct2D | Direct3D 11 + DirectComposition + Direct2D | Direct3D 11 + DirectComposition + Direct2D | Direct3D 11 + DirectComposition + Direct2D |
| **HWND Dimensions** | Fullscreen (`2560x1440`) | Small fixed canvas (`320x56` DIP) | Small fixed canvas (`320x56` DIP) | Small fixed canvas (`320x56` DIP) |
| **Idle HWND Visibility** | **Hidden** (`IsWindowVisible = false`) | **Shown** (`ShowWindow(SW_SHOWNOACTIVATE)`) | **Shown** (`ShowWindow(SW_SHOWNOACTIVATE)`) | **Hidden** (`IsWindowVisible = false`) |
| **DirectComposition Root** | Unknown (Hidden HWND) | Attached continuously (opacity 0) | Detached while idle, attached for cue | Attached continuously (opacity 0) |
| **Capture Exclusion** | Not observed / unconfirmed | Configurable / `WDA_EXCLUDEFROMCAPTURE` | Off for Phase 4B tests | Off for Phase 4B tests |
| **`WS_EX_LAYERED`** | Present (`0x00080000`) | Omitted (DComp recommended) | Omitted | Omitted |

### Key Differences & Inferences

1. **Idle Window State:**
   The confirmed primary difference between SteelSeries GG and Silk's production
   HUD is that SteelSeries maintains its `GameOverlay` HWND hidden while idle.
   Silk's production HUD shows a small, topmost, transparent window with an
   attached zero-opacity DirectComposition visual tree continuously.
2. **Diagnostic Policy Alignment:**
   Silk's benchmark-only `detached-shown` diagnostic toggled DirectComposition root
   attachment while leaving the HWND shown, which differs from SteelSeries's hidden
   HWND topology. Silk's unmeasured diagnostic policy `attached-hidden` (prewarmed
   attached root, hidden HWND, calling `ShowWindow` only during active cues)
   most closely matches the confirmed idle topology of SteelSeries GG.
3. **Unknowns:**
   It remains unknown whether SteelSeries detaches its DirectComposition root while
   hidden, what exact commit sequence it executes when presenting a notification,
   or whether its notifications preserve hardware independent flip during active
   display. Direct PresentMon measurement is required before claiming performance
   parity or proof of behavior.
4. **Style Caution:**
   `WS_EX_LAYERED` is present on SteelSeries's window, but its performance impact
   was not isolated. It should not be assumed causal or adopted without an
   isolated empirical test.

---

## Continuation Status & Measurement Blockers (2026-08-31)

- **Optional Reference Measurement Blocked:** Live PresentMon empirical
  measurement of SteelSeries GG overlay cues is blocked because Valorant is not
  running in a stable foreground scene, the user is unavailable for interactive
  elevation/scene setup, and no safe pre-configured overlay notification trigger
  was established.
- **Prohibited Automation:** In accordance with safety policies, no game input,
  process injection, memory tampering, or protected anti-cheat state automation
  was attempted.
- **Topology Findings Unchanged:** Read-only module inspection and HWND
  hierarchy findings remain the primary technical reference points for the
  hidden-idle overlay model.

---

## References

- SteelSeries Support — What is Moments Capture Mode:
  https://support.steelseries.com/hc/en-us/articles/34379253751309-What-is-Moments-Capture-Mode
- SteelSeries Support — How does Moments Capture Work:
  https://support.steelseries.com/hc/en-us/articles/360060558211-How-does-Moments-Capture-Work
- SteelSeries Support — Auto-clip Valorant with SteelSeries Moments:
  https://support.steelseries.com/hc/en-us/articles/4416847694349-Auto-clip-Valorant-with-SteelSeries-Moments
- SteelSeries Support — How to enable/disable Moments tag on my clips:
  https://support.steelseries.com/hc/en-us/articles/43442485371917-How-to-enable-disable-Moments-tag-on-my-clips
- SteelSeries GameSense SDK API Documentation:
  https://raw.githubusercontent.com/SteelSeries/gamesense-sdk/master/doc/api/sending-game-events.md
