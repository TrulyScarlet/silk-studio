# ADR 0083: Game-Guarded Window-Cropped Capture via DXGI Desktop Duplication

Superseded in part by ADR 0085: capture-excluded targets must be denied. Live
VALORANT duplication omitted the game and exposed underlying desktop content.

- Date: 2026-11-15
- Status: Accepted

Amended by ADR 0084: this is explicit opt-in monitor cropping, not isolated WGC
window capture. No silent game/window fallback is permitted.

## Context

Certain games (e.g., VALORANT protected by Riot Vanguard) enable OS window-capture exclusion
(`WDA_EXCLUDEFROMCAPTURE`, display affinity 0x11) to hinder malicious screen-scraping, and
Vanguard strictly blocks unvetted third-party DLL injection. Under ADR 0061, direct OS window
capture (`WgcDisplayCapture`) respects this affinity and returns `PermissionDenied` to prevent
silent monitor fallback and accidental desktop exposure.

Monitor cropping offers a separate non-injected capture option, but does not
provide the privacy guarantees of isolated window capture. This ADR does not
establish how commercial tools implement capture. The topology here is:
1. Capture the display containing the game via DXGI Desktop Duplication (`IDXGIOutputDuplication`).
2. Crop the GPU texture to the target game window's client bounds.
3. Enforce an active privacy guard (`GetForegroundWindow` / process-affinity check): if the game
   loses focus (Alt-Tab, minimize, desktop, browser, or private chats), the engine immediately
   blanks the frame. Overlapping windows and overlays can still be captured while
   the game remains foreground; focus-transition privacy requires further testing.

## Decision

Introduce `GuardedWindowCapture` in `capture-windows`:

1. **Target Validation & Monitor Resolution**:
   - Accepts a validated window target (`pid:X:created:Y:hwnd:Z`).
   - Verifies process identity and creation timestamp (`resolve_window`).
   - Identifies the host monitor via `MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)` and
     matches it to the corresponding `IDXGIOutput`.

2. **GPU Texture Cropping**:
   - Captures the monitor frame into a DXGI desktop staging texture.
   - Calculates the intersection between the window's visual bounds (`DwmGetWindowAttribute` /
     `GetWindowRect`) and monitor coordinates.
   - Employs Direct3D 11 `CopySubresourceRegion` to crop the game region directly into leased
     pool textures on the GPU with zero CPU readback.

3. **Active Privacy Guard**:
   - Queries foreground state on every frame tick (`GetForegroundWindow`, `GetWindowThreadProcessId`).
   - If the game is minimized (`IsIconic`) or a foreign process is in the foreground:
     Clears the frame slot to solid black (`ClearRenderTargetView`) and suppresses cursor drawing.
      This reduces exposure but is not a complete privacy guarantee.

4. **Lifecycle & Fail-Closed Behavior**:
   - If the target window is destroyed or the process exits, capture terminates with `SourceLost`.
   - If the window changes dimensions or moves across monitors, the session adapts or rebuilds cleanly.
   - Exposes explicit source identifiers `game-guarded:<target>` and `guarded:<target>`.

## References

- [Direct3D 11 CopySubresourceRegion](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-copysubresourceregion)
- [DXGI Desktop Duplication](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api)
- [ADR 0061: Respect Window Capture Exclusion](0061-respect-window-capture-exclusions.md)
