# 0042: Apply source changes and follow the foreground monitor

Status: Accepted, 2026-11-11

A local black-clip report became visible after capture was stopped and restarted
on DISPLAY2. Isolated WGC and DXGI capture/encoding produced visible pixels on
both monitors. This does not prove the exact cause of the earlier black frames,
but exposed two concrete source-selection defects: saved source changes did not
reconfigure the active recording, and Auto always resolved to the primary display.

Changing the source now restarts an active controller session automatically.
Accepted clip saves drain through the existing save worker, and their completion
events remain available to the UI pump. Pending transition batches are bounded
to four. A synchronous startup failure restores previous settings and attempts
to restart the previous source. Asynchronous worker failures still use the normal
error events. Stopped sessions stay stopped; retention-only changes stay live.
Persistence rollback can reapply the previous source through the same path.

Auto resolves the monitor containing the foreground external application. While
capturing, a 500 ms check requests existing engine recovery if that monitor changes.
No GPU context changes behind the running encoder. Silk's own foreground windows
and missing/minimized foreground windows do not move Auto capture. Explicit display
selections remain fixed. New configurations use Auto; saved explicit choices are
preserved. This remains display capture, not isolated game or process capture.

The selected Auto display reports its initial dimensions before video frames so
Native resolution is rebuilt correctly even when the primary display differs.
Source changes start a fresh replay buffer; earlier unsaved replay history is lost.
Monitor changes can therefore briefly interrupt priming. Protected content and
driver-specific black frames are not diagnosed or bypassed by these changes.

No dependencies changed. Tests cover live source replacement, rollback, stopped
sessions, retention-only updates, fixed versus automatic monitor decisions,
native GPU conversion of known white pixels, and opt-in per-display source/output
sampling. A live Auto capture test confirms monitor dimensions arrive before
resolvable GPU frames. Physical foreground switching and both games still need acceptance
testing; Goomy's black recordings are not claimed fixed without a retest.
