# ADR 0060: Owned D3D12 window-state acceptance evidence

- Date: 2026-11-12
- Status: Accepted for opt-in owned child testing only

Test the actual owned D3D12 Present detour across minimize/restore and monitor-sized
borderless/windowed HWND transitions. Commands affect only the child's selected
window: no keyboard input, external application manipulation, display resolution
or security setting changes. This is not an automated Alt-Tab into another app.

Use typed fixed-size window commands on the same private approved child channel.
Report observed IsIconic/style/monitor-rectangle flags and an action-complete QPC
marker. Change the renderer's selected color per phase (never unrelated blue).
Require freshly submitted post-marker frames to contain the phase's expected
pixels, avoiding a false pass from retained old frames. All frames still use the
same selected backbuffer, shared-fence ownership and exact session gate.

The backbuffer size stays fixed; these window transitions are not capture resize
rearm. Borderless style + monitor rectangle is window-state evidence, not proof of
classic exclusive fullscreen, independent flip, FSO activation or a game setting.
Microsoft's D3D12 documentation distinguishes FSO from classic FSE. Keep those
claims separate. Record native WARP/hardware results, including failures, before
claiming any behavior. No protected game is selected by these tests.
