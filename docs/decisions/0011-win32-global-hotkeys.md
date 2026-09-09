# ADR 0011 - Dedicated Win32 global hotkey loop

**Status:** Accepted for S6
**Date:** 2026-08-27
**Segment:** S6

## Context

The save shortcut must work while the Silk window is unfocused, without
polling keyboard state or injecting code into another process. Registration
and message handling must not run on the UI or media workers.

## Decision

1. Use the documented Win32 `RegisterHotKey`/`UnregisterHotKey` APIs with a
   null window handle, so `WM_HOTKEY` messages are delivered to a dedicated
   thread queue.
2. Keep registration, replacement, unregistration, and message pumping on
   `silk-hotkey-loop`.
3. Communicate with that thread through an eight-entry bounded control queue
   and a 64-entry bounded event queue.
4. Use `MOD_NOREPEAT` and a 50 ms per-binding debounce window; dropped event
   bursts are reported rather than blocking the loop.
5. Translate `HotkeyEvent::Activated` into the existing controller
   `Command` variants. Hotkey actions never get a separate media or save path.
6. Treat registration conflicts as non-fatal to recorder startup and retain a
   clear structured diagnostic error.
7. Make replacement requests return only after the dedicated thread confirms
   registration. Unchanged bindings remain registered during replacement;
   changed bindings are restored before a conflict is returned to the caller.

## Consequences

- The default `Ctrl+Shift+F10` binding can operate without window focus.
- Binding changes can restore the previous registration if a replacement
  conflicts with another application, and the settings path will not persist a
  rejected replacement.
- Physical registration, focus behavior, and conflict handling remain
  hardware/desktop-dependent validation items.
- No keyboard hook, process injection, or new third-party media dependency is
  introduced.

## Verification

- Chord normalization, duplicate IDs, virtual-key boundaries, and controller
  command routing have deterministic tests.
- Replacement planning and rollback have deterministic tests; the caller
  receives the registration result instead of an optimistic queue-accepted
  result.
- Native `RegisterHotKey` behavior has not been exercised in an interactive
  desktop test yet.
