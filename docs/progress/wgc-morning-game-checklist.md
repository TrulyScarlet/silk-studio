# WGC-first gameplay acceptance

## Current direction

Experimental hook development is paused. Game/window mode uses isolated WGC and
does not silently fall back to desktop capture. Guarded monitor cropping is a
different explicit mode, with different privacy limitations.

Controlled-window acceptance passed on 2026-11-16: selected-window overlap probe,
minimize/restore and resize, hardware replay save/decode stress, stale process
identity rejection, fresh source startup, and external owned-window exclusion.
See `../evidence/wgc-owned-replay-stress-2026-11-16.md`.

## When the user is available

Use a supported game in windowed mode first, then borderless. Do not change
anti-cheat settings, use injection, or treat capture exclusion as an error to bypass.

1. Select the exact running game in Game mode and start recording.
2. Save an ordinary gameplay clip. Check video, game audio, microphone and timing.
3. Overlap the game with an innocuous test window. Save and inspect a clip: in WGC
   mode the overlapping window must not appear. Never use sensitive content as a
   privacy test pattern.
4. Alt-Tab to an innocuous app, return and save. Check capture resumes, audio does
   not drift, and no unrelated content enters the saved video.
5. Minimize/restore, resize and save again. Check no error, stale image, stretching
   or long stall. A new format starts a new replay epoch, so pre-resize history
   may be discarded; wait for Ready before saving.
6. Move the game between monitors if available, then save. Record display scaling,
   resolution, refresh rate, HDR state and GPU arrangement.
7. Close the game, open another ordinary app, and verify capture never adopts it.
8. Restart the game and explicitly reselect its fresh identity. Verify replay
   startup and save again; automatic stale-HWND rebinding is intentionally absent.

Record game/version, capture backend, mode, resolution, GPU/driver, Silk build
identity and actual saved-clip results. A working preview alone is insufficient.

## Expected blocked behavior

If Windows or the application denies capture, show the actionable denial and
stop without monitor fallback. A protected-game success must not be claimed from
an owned-window test. Guarded cropping, if deliberately selected separately, can
record overlays/overlapping windows; foreground shielding is not full isolation.

## Packaging

The local WGC test build is marked with a `-dirty-wgc-test` build identity. It is
not a publishable release. The normal release script requires committed source,
release preflight and the existing signing policy. No updater feed should be
published from this working tree.
