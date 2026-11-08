# Third-Party License Inventory

Every dependency must record name, version, source URL, license, linking mode,
distributed artifacts, required notices, and approval status (spec §26). This
inventory is maintained from Segment S0 onward. Direct dependency versions below
are the current lockfile resolutions; the full transitive closure is checked by
`scripts/verify-dependencies.ps1` and remains represented by `Cargo.lock` /
`package-lock.json`.

| Dependency | Purpose | License | Linking | Distributed artifacts | Notices | Approval |
|---|---|---|---|---|---|---|
| Inter, Google Fonts revision v20 ([source](https://fonts.google.com/specimen/Inter), downloaded 2026-11-04) | Offline desktop UI typography, weights 400/500/600 | SIL Open Font License 1.1 | Font assets | `apps/desktop/public/fonts/inter-{400,500,600}.ttf` | Bundled `fonts/Inter-LICENSE.txt` (Rasmus Andersson) | Permissive font license; no GPL dependency |
| serde 1.0.229 / serde_derive 1.0.229 ([crates.io](https://crates.io/crates/serde/1.0.229)) | Serialization framework | MIT OR Apache-2.0 | Static (linked) | Embedded in binaries | Yes (bundled notices at release) | Reviewed; bundled notices |
| serde_json 1.0.151 ([crates.io](https://crates.io/crates/serde_json/1.0.151)) | JSON configuration/events | MIT OR Apache-2.0 | Static (linked) | Embedded in binaries | Yes | Reviewed; bundled notices |
| sha2 0.10.9 ([crates.io](https://crates.io/crates/sha2/0.10.9)) | SHA-256 fingerprint verification for the approved external FFmpeg pair | MIT OR Apache-2.0 | Static (linked) | Embedded in binaries | Yes | Reviewed; bundled notices |
| thiserror 1.0.69 ([crates.io](https://crates.io/crates/thiserror/1.0.69)) | Error type derivation | MIT OR Apache-2.0 | Static (linked) | Embedded in binaries | Yes | Reviewed; bundled notices |
| chrono 0.4.45 ([crates.io](https://crates.io/crates/chrono/0.4.45)) | Timestamps for logs/metadata | MIT OR Apache-2.0 | Static (linked) | Embedded in binaries | Yes | Reviewed; bundled notices |
| windows-sys 0.59.0 ([crates.io](https://crates.io/crates/windows-sys/0.59.0)) | Win32 registry, single-instance, storage, global-hotkey, and notification-audio bindings | MIT OR Apache-2.0 | Static bindings to system libraries | System-provided OS code only | N/A (system terms) | Reviewed; bundled notices |
| tauri 2.12.5 / tauri-build 2.7.4 ([crates.io](https://crates.io/crates/tauri/2.12.5)) | Desktop shell | MIT OR Apache-2.0 | Static (linked), WebView2 system component | Bundled exe + system WebView2 runtime | Yes | Reviewed; bundled notices |
| tauri-plugin-dialog 2.7.2 ([crates.io](https://crates.io/crates/tauri-plugin-dialog/2.7.2)) | Native folder picker for the user-selected clip base directory | MIT OR Apache-2.0 | Static (linked) | Bundled dialog bridge; native picker remains OS-provided | Yes | Reviewed; bundled notices |
| tauri-plugin-notification 2.3.3 ([crates.io](https://crates.io/crates/tauri-plugin-notification/2.3.3)) | Windows and desktop save/failure notifications | MIT OR Apache-2.0 | Static (linked) | Bundled notification bridge | Yes | Reviewed; bundled notices |
| tauri-plugin-opener 2.5.4 ([crates.io](https://crates.io/crates/tauri-plugin-opener)) | Official Tauri integration for opening files and revealing indexed clips in the system explorer | MIT OR Apache-2.0 | Static (linked) | Bundled opener bridge; system default application remains OS-provided | Yes | Reviewed; bundled notices |
| @tauri-apps/api 2.12.4 ([npmjs.com](https://www.npmjs.com/package/@tauri-apps/api/v/2.12.4)) | Frontend IPC bridge | MIT OR Apache-2.0 | Bundled JS | Bundled web assets | Yes | Reviewed; bundled notices |
| react 18.3.1 / react-dom 18.3.1 ([npmjs.com](https://www.npmjs.com/package/react/v/18.3.1)) | User interface | MIT | Bundled JS | Bundled web assets | Yes | Reviewed; bundled notices |
| vite 5.4.21 ([npmjs.com](https://www.npmjs.com/package/vite/v/5.4.21)) | Frontend build tool | MIT | Dev-time only | Not distributed | Yes | Reviewed; bundled notices |
| typescript 5.9.3 ([npmjs.com](https://www.npmjs.com/package/typescript/v/5.9.3)) | Type checking | Apache-2.0 | Dev-time only | Not distributed | Yes | Reviewed; bundled notices |
| eslint 10.9.1 / @eslint/js 10.0.1 / typescript-eslint 8.68.0 / @vitejs/plugin-react 4.7.0 ([npmjs.com](https://www.npmjs.com/package/eslint/v/10.9.1)) | Linting and React build integration | MIT | Dev-time only | Not distributed | Yes | Reviewed; bundled notices |
| @types/react 18.3.31 / @types/react-dom 18.3.7 ([npmjs.com](https://www.npmjs.com/package/@types/react/v/18.3.31)) | Type declarations | MIT | Dev-time only | Not distributed | Yes | Reviewed; bundled notices |
| windows 0.62.2 ([crates.io](https://crates.io/crates/windows)) | Windows Graphics Capture, DXGI Desktop Duplication, D3D11, high-resolution waitable timers, WinRT, COM, and Media Foundation bindings | MIT OR Apache-2.0 | Static bindings; links to system APIs | Generated bindings embedded in binaries; Windows system libraries | Yes | Reviewed; bundled notices |
| windows-core 0.62.2 ([crates.io](https://crates.io/crates/windows-core)) | Core COM types and `implement` macro support for WASAPI endpoint notifications | MIT OR Apache-2.0 | Static bindings; links to system APIs | Generated bindings embedded in binaries; Windows system libraries | Yes | Reviewed; bundled notices |
| windows-numerics 0.3.1 ([crates.io](https://crates.io/crates/windows-numerics)) | Direct2D vector ABI types | MIT OR Apache-2.0 | Static bindings; links to system APIs | Generated bindings embedded in binaries; Windows system libraries | Yes | Reviewed; bundled notices |
| silk-mp4 0.14.0-silk.1, local attributed fork of mp4 0.14.0 ([upstream](https://github.com/alfg/mp4-rust), provenance in `crates/mp4-silk/UPSTREAM.md`) | Direct MP4 muxing and project-maintained codec sample entries | MIT | Static (pure Rust) | Embedded in binaries | Yes; upstream `LICENSE` retained with the fork | Reviewed; bundled notices |
| bytes 1.12.1 ([crates.io](https://crates.io/crates/bytes/1.12.1)) | Sample byte buffers | MIT OR Apache-2.0 | Static (pure Rust) | Embedded in binaries | Yes | Reviewed; bundled notices |
| Windows APIs | Platform integration | Platform terms | System-provided | System-provided | Platform terms | Required |
| FFmpeg 9.0.1 Gyan full build ([Gyan builds](https://www.gyan.dev/ffmpeg/builds/), [release archive](https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-full.7z), [source commit](https://github.com/FFmpeg/FFmpeg/commit/bf1b838f2a), [official legal considerations](https://ffmpeg.org/legal.html)) | Standalone optional export/media-validation tooling not linked into the desktop product | GPLv3 for the full build; FFmpeg core is LGPLv2.1-or-later | External static process; not linked into Silk | Approved `ffmpeg.exe`/`ffprobe.exe` pair; bundling remains a release decision | GPLv3, FFmpeg/component notices, corresponding source or source offer, exact build configuration | **Approved by maintainer instruction on 2026-08-28; exact binary hashes required** |
| PresentMon 2.5.1 ([GameTechDev/PresentMon v2.5.1](https://github.com/GameTechDev/PresentMon/releases/tag/v2.5.1)) | Local ETW/presentation measurement | MIT | Standalone external developer tool, not linked | Portable binary downloaded only to approved temp location, not distributed with Silk | Notice required only if redistributed | **Local Phase 3 measurement approved by user on 2026-08-31; distribution unapproved/not planned** |

The desktop shell also directly uses the existing `windows` 0.62.2 bindings for
Windows Shell taskbar relaunch properties and COPY-only file drag through OLE; capture uses the WinRT borderless
consent APIs. No additional dependency package or license is introduced.

## Updater dependencies

The release includes `licenses/THIRD_PARTY_NOTICES.txt`, generated from exact
Windows Cargo resolutions and production npm packages by
`scripts/generate-notices.mjs`. Missing crate-level license files are retained
under `licenses/overrides` with immutable upstream source URLs. The existing
MPL-2.0 transitive packages (CSS parsing/selectors and platform directory helpers)
remain unmodified; their license texts and versioned crate source locations are
included. No GPL library is linked into Silk. External FFmpeg is not bundled.

- `tauri-plugin-updater` 2.14.1 (MIT OR Apache-2.0): official Tauri update
  manifest parsing, version-bound Minisign verification, and Windows NSIS
  installation/restart. Statically linked. Permissive license reviewed.
- `reqwest` 0.13.4 (MIT OR Apache-2.0; exact patch in Cargo.lock): bounded native
  installer downloads over HTTPS using Windows native TLS. Statically linked.
  Permissive license reviewed. No frontend updater permissions or signing secrets.

## Approved FFmpeg artifact

- Distribution: WinGet package `Gyan.FFmpeg` version `9.0.1`, corresponding to
  Gyan's `ffmpeg-release-full.7z` archive.
- Archive SHA-256: `4b9c814cb07a1f90d05b768ef4eb2abbf89af94bbb924df5b7dbd6e64e1e2b96`.
- FFmpeg source: commit
  [`bf1b838f2a`](https://github.com/FFmpeg/FFmpeg/commit/bf1b838f2a), based on
  the official `9.0.1` release.
- Approved `ffmpeg.exe` SHA-256:
  `57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A`.
- Approved `ffprobe.exe` SHA-256:
  `AFE05347CAAABE479B3C4EAE71992B6EC1E11C57266A1D665DEB0F9FE9847208`.
- Gyan's full-build configuration and the required GPLv3/source/notice
  delivery record with the release artifact.

## Approved PresentMon measurement tool

- Official release: [GameTechDev/PresentMon v2.5.1](https://github.com/GameTechDev/PresentMon/releases/tag/v2.5.1).
- Upstream source & license: [GameTechDev/PresentMon LICENSE.txt](https://github.com/GameTechDev/PresentMon/blob/v2.5.1/LICENSE.txt) (MIT License, Copyright (c) Intel Corporation).
- Approved portable artifact: `PresentMon-2.5.1-x64.exe` (portable CLI).
- Approved artifact SHA-256: `9bec3083069f58f911e6a512f4806db51a27bd096103087bc1d05ef54c80a191`.
- Digital signature: Authenticode Valid, Signed by `Intel Corporation`.
- Tool status: Approved strictly for local developer presentation/ETW performance measurement during Phase 3 empirical validation (approved by user on 2026-08-31). The portable executable is downloaded only to approved local temporary directories (`%TEMP%`) and is not bundled, shipped, or linked into Silk. Distribution remains unapproved and not planned; notice obligations apply only if redistributed.

Rules:

1. A dependency may be added only after its row exists in this table.
2. GPL or similarly restrictive components require explicit maintainer
   approval before any linkage compiles into shipped artifacts. The exact Gyan
   FFmpeg build above is the approved exception for standalone external tooling.
3. FFmpeg build flags, codecs, and linked libraries determine obligations;
   only the recorded Gyan build and its exact binary hashes are approved.
4. Reviewed rows have identified licenses and generated redistribution notices. The exact FFmpeg approval above does not approve other builds or dependencies.
