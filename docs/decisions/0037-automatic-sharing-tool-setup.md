# 0037: Automatic sharing-tool setup

Status: Accepted, 2026-11-08

The maintainer requires one installer with no manual dependency installation.
Silk 0.1.1 automatically prepares the already approved Gyan FFmpeg 9.0.1 pair
during NSIS setup. This also applies to one-click updates. WebView2 remains
automatically bootstrapped by Tauri. First installation requires internet.

The installer retrieves the versioned ZIP directly from the publisher, rather
than redistributing the GPL executable package in Silk's release. Archive
SHA-256 is 2E8E28AF97C2AE338CCEF92E36DA9B2A4CD21D0CAD9DDE093545606CB07F5B00.
Both executable hashes remain the approved hashes recorded in the inventory.
The publisher's LICENSE and README accompany the executables locally.

Windows PowerShell 5.1 and .NET compression, included on supported Windows,
download over HTTPS with a ten-minute deadline and 260 MB bound. Only four
fixed archive entries are extracted after archive verification. Executables
are verified before replacing any installed pair. Valid installations are
reused offline. Failed downloads/verification abort setup with an actionable
error instead of reporting a successful installation with missing features.
Staging cleanup is restricted to a generated directory under the install root.
Uninstall removes only the four managed files and empty tool directories.

The 0.1.0 unsigned publisher release decision also applies to this necessary
0.1.1 packaging correction, as authorized by the maintainer's request to make
the published app install everything automatically. Updater signatures remain
mandatory. No security settings are changed on the user's system.

Validation covers Windows PowerShell 5.1 setup in an empty directory, a real
publisher download, offline reuse, corrupt archives, repair, and export using
the resulting pair. These tests do not replace a clean Windows VM acceptance
test or guarantee availability of upstream download servers.
