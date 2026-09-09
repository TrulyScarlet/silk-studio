# Segment S9 Progress Log - Packaging and release preparation

**Status:** Release preparation implemented; signed/clean-machine evidence open

**Plan reference:** [implementation-plan.md](../implementation-plan.md) section 13

## Delivered

- [x] User-triggered diagnostic export through the Tauri shell.
- [x] Bounded recent JSONL records and redacted settings in diagnostic packages.
- [x] Build metadata for version, explicit provenance, target, OS, GPU/driver,
  WGC, Media Foundation encoder capabilities, and external media tools.
- [x] Per-user Windows startup registration through the HKCU Run key.
- [x] Production CSP, NSIS current-user bundle metadata, WebView2 bootstrap
  policy, and license metadata.
- [x] Pinned Rust toolchain, dependency/license checks, strict release preflight,
  signed build wrapper, signature verification, and SHA-256 manifest generation.
- [x] ADR 0014 records the installer choice and defers an updater until a
  signed-manifest and rollback design is approved.

## Verification

```text
pwsh -NoProfile -File .\scripts\check.ps1 -> passed
pwsh -NoProfile -File .\scripts\verify-dependencies.ps1 -> passed
pwsh -NoProfile -File .\scripts\release-preflight.ps1 -RunAudits -> passed with 6 warnings (fresh process PATH lacks local makensis/signtool)
cargo tauri build --config apps\desktop\src-tauri\tauri.conf.json --ci -> unsigned NSIS bundle built
pwsh -NoProfile -File .\scripts\smoke-desktop.ps1 -> passed
```

The Rust audit found zero vulnerabilities and reported 17 allowed maintenance
or unsoundness advisories.

The strict release preflight is expected to fail in this environment because
the certificate, timestamp, provenance inputs, and license approvals are not
configured, while the local signing tools, packager, and audit tool are now
installed.

## Remaining Limitations

- Signed release build and Authenticode verification require certificate and
  timestamp material; `signtool`, `cargo-tauri`, and NSIS are installed locally.
- License approval and generated third-party notices require maintainer review.
- Clean-machine Windows 10/11 install, WebView2 bootstrap, upgrade/uninstall,
  startup login, native recovery, and eight-hour endurance require manual
  hardware testing.
- Direct MP4/remux settings and playback remain deferred; the separate
  post-save MP4 export setting, target presets, progress, and cancellation are
  wired and pass their controller/worker tests plus the approved-pair real-media
  integration test. Release bundling, cross-vendor WGC GPU-surface encoding,
  and interactive application evidence remain open.
