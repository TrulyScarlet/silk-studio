# Silk

A Windows-first desktop application that continuously captures a display and
configured audio tracks into a bounded replay buffer. Pressing a global hotkey
saves the previous configurable period as a standard video clip — fast,
reliable clipping without accounts, cloud services, or streaming complexity.

**Status:** S9 preparation in progress — native capture/audio/encoding/muxing
foundations, replay buffering, independently configured audio tracks, global
hotkeys, tray controls, settings, capture confirmation, filesystem-backed clip
management, deterministic recovery tests, and redacted diagnostic export are
present. Native hardware recovery, eight-hour endurance, playback/media
validation, post-save compression, signed packaging, and final acceptance
remain open. Clips can be organized below a user-selected base directory in
Silk-managed game folders; foreground attribution remains best-effort until
interactive Windows verification is complete.

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
- Node.js 18+ and npm 10+

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

Licensing of this project itself is not finalized; see `LICENSE` and
`THIRD_PARTY_LICENSES.md`. No GPL dependency may be introduced without
explicit approval, and no FFmpeg component is linked before a documented
build-flag review (see spec §26).
