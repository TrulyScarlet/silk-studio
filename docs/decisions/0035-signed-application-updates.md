# ADR 0035: Signed application updates

Status: implemented and tested locally; public release still requires publisher signing.

The maintainer requested automatic update detection and one-click download,
installation, and restart before the first public release. This supersedes ADR
0014's deferral of updates. The updater uses the official Tauri plugin, with
native commands rather than exposing plugin installation permissions to the UI.

Silk checks its public GitHub release feed on launch and every four hours. System
settings can disable these checks and provide a manual check. Downloads and
restarts always require the user's Update & restart action. Checks send ordinary
HTTP request metadata to GitHub, never settings, clips, device IDs, or logs.

The release feed is HTTPS `releases/latest/download/latest.json` on
`TrulyScarlet/silk-studio`. An available version must be newer than the installed
one. The installer URL must name that repository's versioned Windows x64 release.
Downloads have a 128 MiB cap, connection/total timeouts, and a redirect limit.
There is only one update operation at a time. The native Tauri verifier checks
both the installer signature and the version in its signed trusted comment
(`requireSignedVersion`). The JSON metadata travels over HTTPS; authentication
comes from the version-bound artifact signature, rather than a separate signature
over the JSON document. Unsigned or corrupted bytes never reach installation.

The updater signing key is separate from the Windows publisher certificate. Its
private half stays outside the repository in a directory readable only by the
current Windows user and SYSTEM; only the public half is embedded in the app.
Release operators must back up this private key securely. Losing it prevents
existing installations from trusting future updates. Windows Authenticode still
requires a valid signing certificate for the public release under ADR 0014.

Capture continues during download and verification. Before installation Silk
rejects active sharing jobs, stops capture, drains queued clip saves, blocks new
capture/settings operations, and backs up config.json and library.json in the
existing app configuration directory under update-backup. The per-user NSIS
installer receives /UPDATE /P /R and relaunches Silk. The app identifier, settings
directory, library catalog, and user's clip folder remain stable. The in-memory
replay buffer starts fresh after restart; the UI explains this before updating.
An installation-launch failure keeps the current app open and resumes capture
if it was previously running.

Recovery prefers a newer signed fix. No silent downgrade is permitted. A manual
rollback uses the previous trusted installer and, if the older version cannot
read newer settings, restores the matching pre-upgrade config/catalog backup.
Clip files are not migrated or deleted by the updater. A failed OS-level installer
can require running the previous installer manually; no universal rollback or
power-loss guarantee is claimed.

The `updater-test-server` Cargo feature permits a loopback HTTP artifact for
isolated upgrade testing. Release scripts must reject this feature and insecure
updater configuration. Test builds and test feeds must never be published.

References:

- https://v2.tauri.app/plugin/updater/
- https://docs.rs/tauri-plugin-updater/2.14.1/tauri_plugin_updater/

## Validation

The opt-in Windows upgrade test installed a 0.0.9 baseline with a loopback feed,
rejected a corrupted installer without replacing or restarting the app, rejected
a validly signed installer paired with the wrong announced version, then used
the actual Update & restart control to download/install/relaunch version 0.1.0.
SHA-256 values matched for config.json, library.json, and all four existing MP4s.
Both pre-upgrade settings/catalog backups matched their originals. This used
updater signatures on local test installers; it does not claim Authenticode,
GitHub/CDN delivery, or clean-machine installation was tested.

Browser tests covered current/available states, progress, verification failure,
retry, and installing status at 1280×800, 1440×900, and 1920×1080. Native tests
cover the official URL restriction, download size bound, single-operation gate,
settings preservation, and capture/settings rejection during shutdown.

The full workspace suite passed 526 tests; 29 opt-in native/endurance tests
remained ignored. Formatting, Clippy with warnings denied, TypeScript, ESLint,
and production npm audit passed. The Rust audit reported zero vulnerabilities;
remaining informational warnings concern benchmark-only or non-Windows paths.
The Tokio dependency was patched to 1.53.3 and a yanked async-executor version
was replaced. Release preflight passes its tooling/dependency checks and stops
on the missing Windows publisher certificate and timestamp configuration.
