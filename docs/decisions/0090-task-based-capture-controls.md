# ADR 0090: Task-based capture controls

Date: 2026-11-17

## Decision

Expose three recording tasks: Game, Screen and Window. Game has a separate
selection control: Automatic (Recommended), or Choose a game. Only manual game
and window selections show the running-window picker. Selecting Game from a
different task starts in automatic mode, matching the existing new-config default.

This is a frontend presentation change, not a new persisted configuration schema
or capture backend. Continue saving `game-auto`, `game`, `display`, and `window`.
Existing `game-guarded` configurations display as Game / Choose a game; merely
opening or saving settings retains that legacy type and exact source identity.
The legacy type is no longer offered as a separate choice. Deliberately choosing
automatic/manual uses the current corresponding type. Discard retains the saved
mode and source. Existing engine routing and fullscreen recovery are unchanged.

Plain-language help explains first-focus waiting and Alt-Tab freezing. Keep the
screen-cropping/overlap limitation visible in both Game and Window. A disclosure
holds supported auto-detection titles and additional capture limitations. Do not
present Window as isolated capture. Keep existing themes, control styles, refresh,
disabled states and save model; no new dependencies.

## Validation

Browser regression covers the three task labels, Screen/Window target saves,
automatic/manual saves, failed-save retry, discard, disclosure interaction, and
legacy type/target preservation. Screenshot review covers automatic/manual at
1440, 768 and 390 pixels, plus Screen/Window at desktop width. Full development
gates are recorded with installation evidence separately.
