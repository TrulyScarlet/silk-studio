# ADR 0014: Installer and update boundary

**Status:** Accepted for S9 release preparation

## Context

Silk is a Windows-first local application. The installer must not require
machine-wide elevation for the initial release, and WebView2 must have an
explicit installation policy. Updates must not introduce silent network
activity or unsigned executable replacement.

## Decision

- Package the first release as a per-user NSIS installer.
- Use the WebView2 download bootstrapper when the runtime is absent. The
  installer therefore documents its network requirement for first-run runtime
  setup; the application itself remains local-first and offline after install.
- Sign the executable and installer with an Authenticode certificate and an
  RFC 3161 timestamp before publication. Missing signing inputs make a release
  preflight fail.
- Do not ship an updater in this milestone. There is no update manifest,
  update endpoint, or background update check.
- Revisit updates in a later ADR. Any future updater must use a signed manifest,
  verify the artifact before replacement, be opt-in, and document rollback.

## Alternatives Considered

- MSIX was not selected because its package identity and deployment model add
  store/package registration requirements that are not needed for the local
  MVP.
- Inno Setup was not selected because NSIS is already the configured Tauri
  target and keeps the first installer path smaller.
- A custom updater was rejected for S9 because signing, rollback, and update
  endpoint policy would be additional security-sensitive surface.

## Consequences

- Current-user installation avoids elevation and the optional startup setting
  uses only the per-user `HKCU` Run value; it does not provide a machine-wide
  install or machine-wide startup registration.
- WebView2 bootstrap failure must be reported as an installer failure with a
  clear remediation path.
- Release artifacts remain blocked until signing tools, certificate material,
  clean-machine install tests, and signed verification evidence are available.
