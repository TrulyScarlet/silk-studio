# Configurable shortcuts and Favorites

## Decision

Separate Capture and Hotkeys in Settings. Move System before Diagnostics and put
Open clips in fullscreen first in one shared settings panel.

Store application and player shortcut overrides in `hotkeys.shortcuts`. Missing
entries inherit defaults; `null` disables an action. The configuration crate and
React UI use the same `shortcuts.json` catalog of IDs, labels, and defaults.
Existing configuration documents remain readable without a version migration.
Existing global recorder bindings take precedence over new inherited local
defaults; explicit duplicate bindings are rejected before saving.

Recorder shortcuts still run through the independent Windows hotkey manager.
Application and playback shortcuts run only in the focused Silk window. Local
actions use exact modifier matching, ignore composition and ordinary text input,
and preserve native button activation. Bookmark submission works in its input
when its assigned chord uses Enter. Standard text editing, Tab navigation, and
dialog Escape retain platform behavior. The settings shortcut closes an open
player before showing Settings so the dialogs cannot overlap fullscreen.

The recorder supports punctuation, editing/navigation keys, function keys, and
numeric keypad chords. Clear and per-action reset controls apply to local
bindings; recording bindings retain their existing required/optional policy.
Player help and tooltips show saved bindings. Clip navigation can be assigned
shortcuts and follows the library's current filtered order.

Favorites reuses `ClipEntry.protected`, which already backs the star and protects
clips against automatic quota deletion. No second favorite flag or migration is
introduced. The collection filter combines with game, search, sort, and grid/list
view. Removing a favorite immediately removes it from the filtered collection.
Collection and game counts are derived from the current collection.

## Validation

- TypeScript type checking, ESLint, Rust formatting and Clippy pass.
- 112 ordinary tests pass across configuration, hotkeys, and shell packages;
  two tests requiring an interactive hardware environment remain ignored.
- `scripts/test-hotkeys-favorites.js` exercises category order, the joined System
  panel, all 31 shortcut editors, discard/save/reopen, conflicts, Escape recording,
  disabling aliases, remapped player actions, and Favorites combinations in both
  library views using an isolated IPC fixture and local MP4 playback.
- Existing fullscreen and System/theme browser regression scripts pass.
- The NSIS release was built and installed. Native screenshots verified the
  joined System panel and Hotkeys layout; Windows UI Automation verified the
  Favorites filter reduces this library from four clips to its two starred
  clips. Existing Ctrl+1 and all three GoXLR track settings were preserved.
  Browser screenshots timed out twice, so layout inspection used the native app.

## Limits

Windows-reserved or already registered global combinations can still fail native
registration; the existing settings transaction reports that error and retains
the previous configuration. Punctuation uses physical browser codes and Win32
OEM virtual keys; non-US keyboard layout behavior is not hardware-tested. Mouse
buttons and modifier-only chords are not keyboard bindings. Frame stepping retains
the existing average-frame-rate approximation for variable-frame-rate clips.

No dependencies or media pipeline changes were introduced.
