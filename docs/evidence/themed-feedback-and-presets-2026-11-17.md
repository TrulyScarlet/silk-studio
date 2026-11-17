# Themed Feedback and Named Color Presets Verification Evidence

Date: 2026-11-17
Platform: Windows 11 (26100), D3D11 / Direct2D / DirectComposition, headless Chrome test runner
Repository: Silk (`C:\Users\Home\Documents\Proj\silk`)
Decision: ADR 0091 (`docs/decisions/0091-themed-feedback-and-named-presets.md`)

## Scope

1. **Buffering Gameplay Text, Clip Saved Bubble, and Native Clip Saved Overlay Matching Theme Color Palette**
   - Eliminate hard-coded green from desktop status and clip feedback components.
   - Wire the in-app buffering badge, in-app "Clip saved" banner, and native D2D/DirectComposition HUD overlay to the active theme palette.
   - Contrast native checkmark/ink dynamically based on relative luminance.

2. **Custom Color Controls for All Roles, Named Presets, and Independent App Icon Selection**
   - Add custom color fields for:
     - Buffering and ready text
     - Clip saved text and badge
     - Clip saved background
     - Overlay accent
     - Overlay background
     - Overlay text
   - Allow saving custom schemes as named presets (1–40 characters, up to 20 presets).
   - Allow choosing the taskbar and tray app icon color independently per preset or custom selection (Silver, Gold, Amber, Crimson).
   - Ensure changes remain isolated to settings modal draft until explicitly saved with **Save Settings**.
   - Built-in theme selection cleanly clears custom overrides while preserving stored custom presets.

## Verification

### 1. Configuration Validation and Serialization

- Deterministic roundtrip tests in `crates/configuration/src/validate.rs`:
  - `named_theme_presets_roundtrip_colors_and_independent_icons`: Verifies valid presets serialize to JSON, deserialize identically, and pass atomic disk save/load cycles without losing any fields.
  - `theme_presets_validate_names_bounds_references_and_all_colors`: Verifies name trimming, length bounds (1–40 chars), character safety (rejects control chars), unique case-insensitive names, 20-preset capacity limit, hex color validation for every nested role, and dangling active preset reference checks.

### 2. Native HUD Palette Integration

- Unit tests in `crates/native-hud/src/windows.rs`:
  - `palette_badges_keep_contrasting_ink`: Verifies white badges receive black text/ink and dark badges receive white text/ink.
  - `themed_brushes_render_in_full_and_compact_modes`: Verifies Direct2D brush initialization with theme accents (Studio, Classic, Ember, Vamp, custom hex) in both Full (320x56 DIP) and Compact (180x38 DIP) modes.

### 3. Desktop Application & Save Gate Tests

- Executed `scripts/test-settings-save-gate.mjs` against live Vite preview:
  - Staging custom colors and icon (`vamp`) does not mutate live CSS variables or invoke backend icon change before saving.
  - Discarding (Escape) restores prior values cleanly.
  - Simulated save failure keeps modal open, displays error banner, and leaves live styles untouched.
  - Submitting successfully persists preset (`Ocean`), updates CSS variables (`--capture-color: #87cce2`, `--feedback-saved-bg: #15161a`, `--clip-saved-color: #e7a26e`), and calls `update_theme_icon('vamp')`.
  - Dispatched controller events (`status_changed: Buffering`, `clip_saved`) verify real rendered DOM colors:
    - Buffering state pill: `rgb(135, 204, 226)` (matches custom capture color, not green).
    - Feedback badge: `rgb(231, 162, 110)` (matches custom clip-saved color, not green).
    - Feedback container: `rgb(21, 22, 26)` (matches custom background).
  - Page reload proves configuration reloads with preset intact.
  - Selecting built-in theme (e.g. Ember) resets overrides and renders buffering & feedback in Ember's amber palette (`rgb(234, 177, 124)`).
  - Re-selecting the named preset restores saved custom colors and icon.
  - Deleting preset is staged and confirmed only on save.

### 4. Automatic Game Target Attribution Verification

- Unit tests in `crates/capture-windows/src/enumerate.rs`:
  - `recognized_games_matches_known_titles_and_rejects_desktop_shells`: Verifies authoritative list of recognized games and rejects desktop shell helpers (`explorer`, `TextInputHost`, `chrome`, etc.).
  - `automatic_game_target_never_resolves_to_desktop_shell`: Verifies `capture_target_window("game:auto")` resolves running games (foreground first, then active) and never defaults to explorer or desktop.
- Integration tests in `apps/desktop/src-tauri/src/shell_tests.rs`:
  - `recorder_status_in_auto_game_mode_never_reports_desktop_target`: Verifies `recorder_status` returns clean game title or auto-detect label without mentioning Desktop.
- Live system verification with running VALORANT (`pid:15900`):
  - `capture_target_window("game:auto")` resolved `WindowSourceInfo { title: "VALORANT", app_name: "VALORANT-Win64-Shipping" }, fullscreen: true`.
  - Main page Target card in Silk UI displayed `Valorant · Fullscreen` instead of `Desktop`.
  - Confirmed via screen capture of the live running application (`silk-window-shot.png`).

### 5. Full Workspace Checks

- Executed `./scripts/check.ps1`:
  - Rust formatting (`cargo fmt --check`): PASS
  - Workspace tests (`cargo test --workspace`): PASS (390+ unit and integration tests)
  - TypeScript typecheck (`npm run typecheck`): PASS
  - ESLint (`npm run lint`): PASS
  - Vite production bundle build (`npm run build`): PASS

## Installation

Built and installed updated candidate `e861c20-dirty-target-attribution`.
- Release binary: `target/release/silk.exe` installed to `C:\Users\Home\AppData\Local\Silk\silk.exe`
- Installer: `target/release/bundle/nsis/Silk_0.1.4_x64-setup.exe`
- Configuration backup: `C:\Users\Home\AppData\Local\Temp\opencode\silk-target-attribution-20261117-034521\config.json`
- Configuration verified byte-for-byte preserved upon restart.
- Running executable verified at `C:\Users\Home\AppData\Local\Silk\silk.exe` (PID 18688).
- UI verified displaying `Target: Valorant · Fullscreen` while buffering gameplay.
