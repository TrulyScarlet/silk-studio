# Settings save gate, capture border consent, and theme icons

## Changes

- `apps/desktop/src/App.tsx`: active UI reads committed settings; all dialog
  dismissal paths discard drafts; failed saves preserve the active configuration.
- `crates/capture-windows/{Cargo.toml,src/session.rs}`: request Windows borderless
  access before capture, with logged fallback on unsupported/denied access.
- `apps/desktop/src-tauri/src/theme_icon.rs`: neutral Studio palette, ICO encoding,
  window Shell relaunch metadata, and pixel/ICO/embedded-asset regression tests.
- `apps/desktop/src-tauri/icons/icon{,-studio}.{ico,png}`: regenerated default assets.
- `apps/desktop/src-tauri/examples/generate_icons.rs`: repeatable ICO generator.
- Desktop manifest/lockfile and license inventory: use existing windows 0.62.2
  bindings directly for Shell property-store APIs; no new package version.
- `scripts/test-settings-save-gate.mjs`: isolated headless Edge regression coverage
  using fake IPC, not the installed recorder or real capture devices.
- ADR 0024 records the save gate, icon identity, and OS permission constraints.

## Validation

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test --workspace --quiet`: passed; opt-in native/hardware tests ignored.
- `npm run typecheck`, `npm run lint`, `npm run build`: passed.
- `node scripts/test-settings-save-gate.mjs`: passed five dismissal paths, unsaved
  theme/icon isolation, failed-save isolation, and successful-save retention.
- `cargo tauri build --bundles nsis`: passed; rebuilt
  `target/release/bundle/nsis/Silk_0.1.0_x64-setup.exe` (3.18 MiB).

## Remaining interactive checks

Install the new build, quit any older tray instance, and verify capture consent,
border removal, recording/saving, and taskbar/tray theme changes. Borderless
permission is controlled by Windows; older OS versions, denial, missing package
capabilities, or another capturing app can retain the indicator. Existing pinned
shortcuts may need unpin/re-pin. No capture hardware, Explorer cache behavior,
installer completion, or post-install UI behavior is claimed verified.

The in-app browser was disconnected; settings tests ran in a separate isolated
headless Edge instance instead. Unrelated pre-existing edits were retained.

## Follow-up: installed Windows verification

The installed app's log confirms the borderless request fails with
REGDB_E_CLASSNOTREG (0x80040154). The host is Windows 10 22H2, build 19045.6466,
below the documented 20348 introduction of borderless WGC. Border removal is
not fixed on this OS; it requires a separate capture backend or supported OS.

Inspection of tao 0.35.3 confirms set_window_icon sets ICON_SMALL only; its
separate taskbar icon uses ICON_BIG. The shell now loads the saved-theme ICO,
sets WM_SETICON/ICON_BIG on the window thread, and retains/releases owned icon
handles via managed state. This corrects the missing taskbar update path;
interactive post-install verification remains required.

The initial package still contained a purple executable icon because Cargo did
not rerun `tauri_build` when only `icons/icon.ico` changed. `build.rs` now tracks
that ICO explicitly. After rebuilding, extracting the icon resource from
`target/release/silk.exe` showed the monochrome Studio asset. A visible
Windows-10-only notice now explains why display capture has the system border.
