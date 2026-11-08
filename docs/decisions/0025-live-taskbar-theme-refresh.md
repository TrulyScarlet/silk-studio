# ADR 0025: Refresh the live Windows taskbar group on saved theme changes

**Superseded by ADR 0026.** The limited checks below passed, but subsequent user
switching reproduced Studio with a yellow taskbar icon despite correct native
icon handles and shortcut metadata. The timed remove/re-add strategy was not
reliable and has been removed. Its original validation is not evidence that the
intermittent defect was fixed.

## Problem

WM_GETICON confirmed the correct themed HICON while Windows 10 Explorer still
displayed the original monochrome taskbar image. Updating relaunch properties
or sending WM_SETICON alone did not invalidate that group image. Immediate
DeleteTab/AddTab also retained it. The previously installed executable was older
than the last installer build.

## Decision

Keep the stable app identifier. Set it at process startup before creating UI.
Create the main window hidden, apply the saved native icon and Shell properties,
then show it. Set relaunch metadata before initially assigning the window ID;
do not reassign that ID on every theme change.

For a visible window changing theme, remove its taskbar button before changing
Shell metadata. Re-add it after 500 ms, giving Explorer and the UI thread time
to process the removal. Do not hide/minimize the window, change focus, reset
Explorer's cache, or create theme-specific application identities.

Coalesce rapid changes into one pending refresh. A repeated saved theme does
not remove the button again. Recheck the HWND and visibility before re-adding;
closing or hiding a window must not cause it to reappear. All HWND/COM work runs
on the window thread; the delay runs on at most one helper thread. Retain the
owned large HICON until replacement. Remove the manually registered button and
clear window properties on exit and before a close that exits the application.

Update only shortcuts whose target matches the running executable. Development
builds and test examples must not rewrite the installed copy's shortcuts. A
shortcut failure is logged and must not prevent live icon updates.

## Validation and limitations

- Formatting, desktop Clippy, and 55 desktop tests cover the icon bytes, property
  ordering, matching shortcut targets, and bounded refresh scheduling.
- `cargo run -p silk --example taskbar_icons` runs the real native update code
  without capture. It uses an isolated stable application identity and a blank
  window; it cycles Studio, Classic, Ember, Vamp, Studio and exits.
- Windows 10 build 19045 taskbar screenshots confirmed one button with gray,
  yellow, orange, red, and gray icons respectively. Evidence is in the ignored
  `target/ordered-*.png` files. Tests with only native icon handles were not
  considered proof of Explorer behavior.
- Installed the rebuilt NSIS package successfully (exit 0). Used Windows UI
  Automation against the installed `AppData/Local/Silk/silk.exe` Settings UI,
  saved Legacy, Vamp, Studio, and Ember, and inspected the real taskbar colors.
  Saved configuration and the matching installed Start Menu shortcut were
  updated. Restored the original Ember setting, closed Settings, and left the
  app open with capture stopped. A final UI Automation query found exactly one
  Silk taskbar button. Focused taskbar screenshots are in
  `target/studio-taskbar-crop.png` and `target/ember-taskbar-crop.png`.
- The button can briefly disappear/reorder during a theme change. This is the
  cost of explicitly refreshing Explorer's cached group. Windows 11, other DPI
  settings, and existing pinned shortcuts need separate interactive coverage.
- No capture, encoder, or hardware behavior is claimed tested by this harness.
- No new dependency: existing windows 0.62.2 / windows-sys bindings are used.
  The taskbar example links Tauri's existing Windows resource library so its
  Common Controls v6 manifest is present on Windows 10.

## References

- https://learn.microsoft.com/en-us/windows/win32/properties/props-system-appusermodel-id
- https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-setcurrentprocessexplicitappusermodelid
- https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shgetpropertystoreforwindow
- https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itaskbarlist-addtab
- https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itaskbarlist-deletetab
