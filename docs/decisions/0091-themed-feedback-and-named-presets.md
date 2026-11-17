# ADR 0091: Theme-aware feedback and named color presets

Date: 2026-11-17

## Decision

Capture ready/buffering text and successful-save feedback use the active theme's
accent rather than fixed green. The native Direct2D/DirectComposition overlay
receives a fixed RGB palette when constructed; its saved/queued background,
text, badge, border and activity brushes follow that palette. Error cues retain
semantic error colors. Badge ink is chosen by relative luminance for contrast.
Theme changes rebuild HUD resources through the existing serialized settings
reconfiguration, outside cue dispatch; queue bounds, nonblocking event handling,
capture exclusion and media threads remain unchanged.

Custom colors add independent capture text, saved text/badge/background and
overlay accent/background/text roles. Unspecified roles follow main/background/
text overrides, then built-in defaults. Desktop and native default palettes agree.
Existing explicit overrides are retained. Previews are local to settings drafts.

Add optional/defaulted application settings: `iconTheme`, `themePresets` and
`activeThemePreset`. A preset contains a name, base theme, resolved color snapshot
and one of the four existing icon themes. Keep icons on the existing immutable
Windows theme resources/AppUserModelIDs; never generate unbounded taskbar IDs
from user preset names. Startup, settings save and late frontend icon requests
all resolve the persisted icon override, preventing stale theme effects from
replacing the chosen icon. Window background still follows the base theme.

Presets are part of the versioned configuration and atomic settings save, not
localStorage or extra files. Absent fields default for older configurations.
Names are 1–40 characters, trimmed, no controls and unique case-insensitively;
limit 20 presets and validate every nested hex color. Active names must exist.
Create/update/delete stage changes only. Save Settings persists/applies them;
discard and failed saves leave current appearance/icons/HUD unchanged. Deletion
requires confirmation and retains current applied colors. Color/icon edits clear
the active-preset marker until explicitly saving/updating a named snapshot.
Built-in theme selection clears overrides and icon override so it actually
restores that theme's palette, without deleting stored custom presets.

## Consequences and limits

No new dependencies or media/capture changes. Icon choice is Silver, Gold, Amber
or Crimson, not arbitrary uploaded artwork/RGB icon generation. User-selected
colors may have poor text contrast; only badge ink is automatically contrasted.
Existing warning/error/diagnostic semantics are not globally recolored.
Older binaries that reject unknown fields cannot read newly saved preset fields;
retain a pre-update config backup for rollback. No cloud syncing/import/export.

Deterministic config, palette and browser interaction tests plus opt-in native
brush/draw checks are recorded in the corresponding theme-presets evidence.
