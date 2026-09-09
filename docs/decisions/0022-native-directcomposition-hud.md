# ADR 0022 - Native DirectComposition HUD for Capture Confirmation

**Status:** Accepted
**Date:** 2026-08-31

## Context

Silk previously used a secondary, transparent Tauri WebView2 window to render
on-screen capture confirmations ("Silk Captured", "Saving replay...", etc.).
The only established root-cause evidence is the user A/B test on 2026-08-31:
the several-second game stutter disappeared when the gameplay
capture-confirmation WebView overlay was disabled under otherwise identical
conditions.

Architectural issues associated with the WebView2 overlay approach include:

1. **Subprocess & IPC Overhead**: Each WebView2 instance relies on browser
   subprocesses and web IPC to coordinate visual feedback.
2. **Presentation & Compositing Costs**: Spawning or mutating a web overlay
   window can cause presentation transitions and compositing hitches during
   gameplay.
3. **Unverified Capture Exclusion**: Capture-exclusion behavior for the legacy
   overlay was not independently verified across Windows and capture paths.

An instant-replay clipping tool requires a bounded native HUD with zero cue-time
window mutation that operates out-of-band without blocking the recorder engine,
Tauri event pipeline, or user gameplay.

## Decision

1. **Native Direct2D / DirectComposition Surface**:
   Replace the WebView overlay with an in-tree native HUD (`crates/native-hud`).
   The HUD creates a dedicated click-through, unactivated Win32 tool window
   (`WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`)
   with a DirectComposition visual tree and Direct2D render target backed by a
   Direct3D 11 DXGI flip-discard composition swapchain.

2. **Dedicated Background Worker Thread**:
   All Win32 window creation, DirectComposition setup, Direct2D rendering, timer
   ticks, and message dispatching execute exclusively on an isolated background
   thread (`silk-native-hud`). The thread runs a standard Win32 message pump and
   receives non-blocking cues via a bounded `std::sync::mpsc::sync_channel`
   (bound = 16).

3. **Bounded Non-Blocking Hot Path Dispatch & Allocation Scope**:
   The `TauriEventSink::emit` path attempts HUD dispatch before frontend IPC,
   tray updates, or desktop notifications. It retrieves the managed `HudRuntime`,
   acquires only a `try_read()` lock, and calls `NativeHud::try_show()`.
   Default queued and saved HUD cue construction and rendering is
   allocation-free. Failed and rejected event mappings currently copy owned
   reason/message strings during cue creation, while submission failure
   handling (e.g. queue full or lock contention) on the hot event path does
   not format or log.

4. **Deterministic Event Mapping**:
   Only four controller events trigger HUD visual cues:
   - `ControllerEvent::SaveQueued` -> `HudCue::Queued`
   - `ControllerEvent::ClipSaved` -> `HudCue::Saved` (with duration and size)
   - `ControllerEvent::SaveFailed` -> `HudCue::Failed` (with error message)
   - `ControllerEvent::CommandRejected { command: "save_replay" }` -> `HudCue::Rejected`
   All other controller events (capture start/stop, status changes, warnings,
   non-save command rejections) map strictly to `None` and produce no HUD work.

5. **Fixed Canvas & In-Canvas Pill Offsets**:
   The native window is constructed with a fixed physical size of 320x56 DIP
   scaled to primary monitor DPI. Both `Full` (320x56) and `Compact` (180x38)
   modes render inside this single fixed canvas without resizing the HWND.
   In Compact mode, internal Direct2D geometry offsets the pill toward the active
   anchor edge to preserve true corner and edge alignment without window
   mutations.

6. **Decoupled Tauri `HudRuntime`**:
   The HUD state is encapsulated in a standalone Tauri-managed `HudRuntime`
   struct (`src/hud.rs`), completely separate from `ShellState`.
   `HudRuntime` holds `HudInner { settings, hud: Option<NativeHud> }` behind an
   `RwLock`, plus a `Mutex<()>` serializing reconfigurations. This ensures
   `ShellState` unit tests remain 100% headless without Win32 window creation.

7. **Reconfiguration Transaction & Drop Isolation**:
   Reconfiguration is invoked from `configure_settings` only after settings
   validation and persistence succeed. Reconfiguration determines if a rebuild is
   necessary (settings changed or healthy HUD missing), constructs the replacement
   `NativeHud` outside the inner lock, atomically swaps instances under a brief
   write lock, releases the reconfiguration lock, and drops the previous
   `NativeHud` outside all locks. Because `NativeHud::drop` joins the worker
   thread, dropping outside locks prevents deadlocks and lock contention.

8. **Truthful No-HUD Fallback**:
   When disabled (`settings.overlay.enabled == false`) or if native initialization
   fails (e.g. unsupported platform, missing D3D11/DirectComposition support),
   `Option<NativeHud>` is set to `None` and a truthful diagnostic warning is
   logged. No fallback WebView window is ever created or shown.

9. **WebView Overlay Retirement**:
   The legacy `overlay.rs` module, secondary WebView configuration, and frontend
   overlay script injection are removed from the desktop shell.

10. **Limitations & Phase 3 Empirical Gates**:
    - **Exclusive Fullscreen**: DirectComposition surfaces are composited by DWM
      over desktop and borderless/windowed applications. Classic Exclusive
      Fullscreen (FSE) games bypassing DWM may occlude the HUD.
    - **Hardware Claims**: Multi-Plane Overlay (MPO) hardware planes, capture
      exclusion (`WDA_EXCLUDEFROMCAPTURE`), cross-process click-through, and frame
      pacing impact are requested or implemented but not claimed as verified until
      empirically tested.
    - **Phase 3 Frame-Pacing Gates**: Empirical validation against a real game
      will enforce:
      - No focus loss
      - No cue-time window geometry or visibility mutation
      - Present mode unchanged where baseline uses hardware independent flip (or
        hardware-composed independent flip)
      - No dropped frames attributable to the cue
      - p99 frame-time increase <= 1 ms in the 1-second window around the trigger

## Alternatives Considered

- **Tauri / WebView2 Overlay**: Spawns web processes with unpredictable
  presentation latency and window mutation overhead.
- **In-process Direct3D Hooking / Injection**: Intercepts `IDXGISwapChain::Present`
  inside target game processes. Explicitly prohibited by project architecture
  principles due to anti-cheat risks and game crash vulnerability.
- **GDI / Layered Window (`UpdateLayeredWindow`)**: Legacy Windows compositing
  path with software rasterization and CPU overhead, incompatible with smooth
  hardware-accelerated alpha blending.

## Consequences

- Capture confirmations display via native DirectComposition surface without
  spawning secondary WebView2 overlay processes.
- Desktop shell event emission and recorder pipeline remain non-blocking.
- `ShellState` remains cleanly decoupled from native window handles, preserving
  fast headless unit tests.
- Reconfiguration cleanly swaps native HUD instances with worker thread join
  occurring outside all locks.

## Verification

- Deterministic pure unit tests in `apps/desktop/src-tauri/src/hud.rs` verify all
  mode/anchor mappings, event-to-cue transformations, headless/disabled runtime
  initialization, disabled reconfiguration, and lock contention behavior.
- Shell test suite in `apps/desktop/src-tauri/src/shell_tests.rs` verifies
  settings updates without initializing native windows.
- Focused tests in `crates/native-hud` verify Direct2D/DirectWrite rendering
  geometry, string truncation, and formatters.
- Workspace format and linting checks ensure zero warnings across all crates.
