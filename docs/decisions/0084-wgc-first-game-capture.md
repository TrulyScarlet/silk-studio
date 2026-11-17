# ADR 0084: Prioritize WGC game-window capture; pause hook development

Date: 2026-11-16

## Decision

Pause development of the experimental in-process hook. Retain its code and
evidence; it is not the shipping game-compatibility strategy.

Prioritize Windows Graphics Capture (WGC) of the selected game window, using
existing process-lifetime/window identity validation and GPU surface delivery.
Game and window source selection must propagate capture-affinity errors rather
than silently switching to monitor capture.

Guarded DXGI monitor cropping remains a distinct, explicitly selected mode. It is
not WGC or render-pipeline game capture: visible overlays and overlapping windows
can enter its crop even while the game remains foreground. Foreground blanking
does not establish complete privacy. Neither mode guarantees anti-cheat approval
or capture of excluded/protected content.

## Follow-up acceptance priorities

- Verify selected-window isolation when unrelated windows overlap the game.
- Verify minimize/restore, Alt-Tab, resize and cross-monitor recovery with saved
  clips, not only session startup.
- Verify game exit/restart and fresh identity selection without unrelated capture.
- Publish per-game, display-mode and hardware compatibility evidence; distinguish
  WGC from guarded cropping and list blocked combinations.
- Validate privacy around focus transitions before making shielding guarantees.

These are pending acceptance tasks, not claims of hardware verification. Prior
hook delivery gates remain incomplete and deferred.
