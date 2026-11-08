# Release preparation

Silk uses a per-user Windows x64 NSIS installer. The app identifier remains
`dev.silk.project`; settings and the clip catalog live in its app configuration
directory, independently of the installed executable. MP4s remain in the user's
chosen clip folder. WebView2 is bootstrapped if missing and may need internet access.

## Release build

Commit the reviewed source first, then configure Windows publisher signing:

- `SILK_WINDOWS_CERTIFICATE_THUMBPRINT`: code-signing certificate in the Windows store.
- `SILK_WINDOWS_TIMESTAMP_URL`: its RFC 3161 timestamp service.
- `TAURI_SIGNING_PRIVATE_KEY`: updater private-key file or content. On this developer
  machine the build script discovers `%LOCALAPPDATA%\Silk\signing\updater.key`.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: if the updater key is password protected.

Never commit private keys or certificate files. Back up the updater key securely:
existing installations trust its public half, so losing it prevents signed updates
for those installations. This key is separate from the publisher certificate.

Run from the repository root:

```powershell
node scripts/generate-notices.mjs
pwsh -NoProfile -File scripts/check.ps1
# Commit source and regenerated notices before the release build.
pwsh -NoProfile -File scripts/build-release.ps1
```

The release script requires a clean checkout, derives provenance from the commit,
finds installed NSIS/Windows SDK/media tools, and runs strict preflight and audits.
Tauri signs the app before bundling, signs the installer, then signs the completed
installer for the updater. Publisher verification follows. Signing the installer
after producing its updater signature is invalid and must not be done.

The updater test feature and insecure transport configuration are forbidden in
public release builds. Unsigned artifacts are suitable for local testing only and
must not be published as releases under the current project release policy.

## Publishing an update

For each public version, increase the matching versions in the workspace and
app Cargo manifests, package.json/package-lock.json, and tauri.conf.json. Build
and test that version, then publish GitHub release `vVERSION` on
`TrulyScarlet/silk-studio` with these assets from `target/release/bundle/nsis`:

- `Silk_VERSION_x64-setup.exe`
- `Silk_VERSION_x64-setup.exe.sig`
- `latest.json`
- `SHA256SUMS.txt`

`write-update-manifest.ps1` generates the feed from the version-bound signature.
Publish as the latest stable release only after all assets are attached. Never
publish the loopback test baseline or a manifest pointing to localhost.
Pushing source code alone does not trigger the application's update button.

Silk checks on launch and every four hours when automatic checks are enabled.
Settings → System also has a manual check. An Update & restart action downloads,
verifies, drains accepted clip saves, backs up settings/catalog, installs and
relaunches. An offline check or invalid signature leaves the current version
installed. Saved clips/settings are retained; the in-memory replay buffer resets.
Recovery favors a newer signed fix. Manual downgrade may require restoring the
matching config/catalog from `update-backup`; no silent downgrade is allowed.

## Validation and limits

The local 0.0.9 → 0.1.0 test exercised the real update controls, rejected corrupt
and mismatched-version artifacts, and installed/restarted successfully. Hashes
matched for settings, library metadata, and four saved MP4s. See ADR 0035 and
`scripts/test-app-upgrade.ps1`. That test does not establish Authenticode, public
GitHub/CDN delivery, clean-machine WebView2 bootstrap, or arbitrary GPU support.

Original drag-and-drop sharing works without FFmpeg. Compressed 10/50/100 MB share
copies require the separately installed exact FFmpeg/FFprobe pair recorded in
THIRD_PARTY_LICENSES.md. These optional tools are not distributed in the installer.

`cargo audit` reports advisory warnings for older benchmark-only dependencies and
non-Windows platform dependencies. Their paths and scope must be reviewed; a zero
vulnerability count is not a claim that every dependency is free of issues.
Production npm dependencies are audited separately from development tooling.
Native hardware recovery, clean-machine install/uninstall, and eight-hour resource
trends remain manual acceptance work.
