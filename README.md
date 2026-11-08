# Silk

A Windows-first desktop application that continuously captures a display and
configured audio tracks into a bounded replay buffer. Pressing a global hotkey
saves the previous configurable period as a standard video clip — fast,
reliable clipping without accounts, cloud services, or streaming complexity.

**Status:** preparing the first public Windows release. Silk includes DXGI desktop
capture, independently configured audio tracks, configurable hotkeys, themes,
fullscreen/theater playback, favorites, VOD review markers, size-limited share
copies, native file drag and drop, and signed one-click application updates.
Settings and clips remain on the device. Game attribution is best effort; capture
uses desktop duplication rather than graphics hooks or process injection.

Updates are announced in the title bar and in Settings → System. Clicking
**Update & restart** downloads a signed installer and restarts Silk. Saved clips
and settings stay; the unsaved replay buffer starts fresh. Automatic checks can
be disabled. A Git push alone does not publish an app update: the versioned
GitHub Release must include the installer, signature, and `latest.json` feed.

Original-file sharing works without extra tools. The 10/50/100 MB share-copy
presets require the separately installed, approved FFmpeg/FFprobe pair described
in [the dependency inventory](THIRD_PARTY_LICENSES.md). These binaries are not
bundled. Windows publisher signing and clean-machine acceptance are separate
release gates; broad hardware compatibility and eight-hour endurance are not
claimed from local tests.

## Documentation

- [Specification](spec.md)
- [Segmented implementation plan](docs/implementation-plan.md)
- [Agent instructions](AGENTS.md)
- [Third-party license inventory](THIRD_PARTY_LICENSES.md)
- [Architecture decision records](docs/decisions/)
- [Progress logs](docs/progress/)
- [Release preparation](docs/release-preparation.md)
- [MVP acceptance audit](docs/acceptance-audit.md)

## Prerequisites

- Windows 10 (1903+) or Windows 11
- Rust 1.98.0 MSVC toolchain from `rust-toolchain.toml` with Visual Studio Build Tools
- Node.js 22+ and npm 10+

## Getting started

```powershell
# Desktop frontend
cd apps/desktop
npm install
npm run build
cd ../..

# Native core
cargo build --workspace
cargo test --workspace
```

The short deterministic recovery suite can be run directly:

```powershell
cargo test -p s8-endurance -- --test-threads 1
```

## Development gates

Run all checks with `scripts/check.ps1`, or individually:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/desktop; npm run typecheck; npm run lint; npm run build
```

## Licensing

Silk retains its MIT license; see `LICENSE` and `THIRD_PARTY_LICENSES.md`.
Redistribution notices are bundled with the installer. No FFmpeg component is
linked into the application; the approved optional tool runs as a separate process.
