# ADR 0024: Saved settings and Windows theme icon identity

## Decision

Settings edits are drafts until the native configure command succeeds. The main
screen, theme, and native icons use saved settings only. Closing settings through
Close, X, Escape, backdrop, or Ctrl+, discards drafts. Closing is blocked during
an in-flight save. Theme cards retain their isolated sample previews.

Keep the existing app identifier for taskbar grouping. In addition to the Tauri
window/tray icon, set the main window's Shell property store relaunch icon,
command, name, and AppUserModelID. Store generated ICOs in the application's local
data directory with versioned filenames so Explorer does not reuse old pixels.
Do not edit unrelated shortcuts or reset Explorer's icon cache. Existing pinned
shortcuts can retain their old icon until the user unpins/re-pins Silk.

The default Studio icon uses the UI's neutral #efefed accent on charcoal, not
the previous purple palette. `cargo run -p silk --example generate_icons`
regenerates the default ICO assets. Convert these ICOs to PNG with Windows
System.Drawing.Icon.ToBitmap when updating the bundled PNG assets.

Before WGC capture begins, request Borderless access through the official WinRT
API, then disable the border only when access is Allowed. Failure or denial is
logged and recording retains the Windows indicator. No hooks or OS-security
bypass is used. Consent negotiation happens before the real-time frame pump.

## Constraints and verification

Microsoft documents that packaged apps need graphicsCaptureWithoutBorder and
that another application's border requirement overrides ours. This NSIS build
is unpackaged: whether Windows grants borderless access requires interactive
verification on the target OS. The observed Windows 10 build 19045 does not
register the API class and cannot honor this setting. Display capture on such
systems must be migrated to a DXGI Desktop Duplication backend to avoid this
border; never claim it is hidden while using WGC there.
Permission errors must never prevent normal recording.

Taskbar behavior (including existing pinned groups) also requires interactive
Windows verification. Automated checks cover icon pixels/ICO structure and
settings save/discard behavior; they do not prove capture hardware behavior.

No new dependency package: the existing MIT/Apache-2.0 windows 0.62.2 bindings
are also directly used by the desktop shell for Shell property-store APIs.

## References

- https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired
- https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-relaunchiconresource
