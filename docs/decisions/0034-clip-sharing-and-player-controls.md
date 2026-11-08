# ADR 0034 — Clip sharing and player controls

Status: Accepted

## Decision

Default capture and replay indicators use the neutral Studio accent; other theme
presets and explicit color overrides retain their colors. Recent activity lives
in Diagnostics. Removing its main-page row lets the library extend to the existing
12 px bottom inset. All player modes share edge navigation. Fullscreen has one exit
control at bottom right, speed controls at left, and centered rewind/play/forward.

Library thumbnails can drag the original file. A Share action in both library
views and the player opens an original Silk dialog with Original / 10 / 50 / 100 MB
choices. Size limits are decimal bytes, independent of any service's changing
account-specific upload allowance. A preset prepares a separate H.264/AAC MP4.
There is no account integration, automatic upload, or message sending.

When a file is ready, the drag area displays the clip's paused thumbnail using
the library's existing `ClipThumbnail` component. The preview preserves the full
frame and passes pointer gestures through to the file-drag area. Preparing and
failed states keep the status display; an unavailable thumbnail uses the existing
fallback without preventing file sharing.

The shell reuses the existing `clip-export` planner, bounded queue and
`encoder-ffmpeg` worker. Only one share job is accepted at a time. Encoding,
tool verification, file I/O, validation, and cancellation stay away from capture
and window threads. Stage labels describe actual work; an indeterminate progress
bar avoids pretending that coarse backend stages are precise encoding percentages.

Preset copies mix up to eight recorded audio streams to one stereo track, with a
limiter. Originals retain separate tracks. The mix uses the recording's levels;
temporary player mixer changes are not export settings. Resolution/FPS adapt to
the available bitrate, preserve aspect ratio to even-pixel rounding, and never
upscale. The first version caps share copies at 1920 px wide / 1080 px high / 60 fps.
The planner reserves 12% for overhead and encoder variance. The existing exporter
checks actual bytes, stream metadata, monotonic decode timestamps, and full decode
before atomic publication. Oversized/invalid outputs fail and are never draggable.

Shared copies use an app cache limited to 64 generated files / 2 GB. Files older
than seven days may be removed on the next preparation. Recent files remain after
closing the dialog so a receiving application can finish reading/uploading them.
Closing during preparation cancels work, including a preparation returning after
the dialog has already unmounted. A full cache reports its folder for cleanup.

Windows Shell creates the `IDataObject` from an indexed original or a completed
export owned by this process. `SHDoDragDrop` runs on the window STA with COPY only;
an arbitrary frontend file path cannot bypass library validation. The six-pixel
pointer threshold preserves ordinary thumbnail clicks; a drag suppresses opening
the player. Escape closes sharing before player shortcuts run.

## Dependencies and references

No new third-party package. Windows Shell/OLE uses the existing windows 0.62.2
bindings (MIT/Apache-2.0). Compression requires the exact external Gyan FFmpeg 9.0.1
pair approved in ADR 0017. Both binary hashes are checked before execution.
Search order: explicit environment pair, executable-adjacent tools, the local
Silk tools folder, Gyan WinGet layout, then PATH. Tools remain external; this change
does not add FFmpeg binaries to the distributable installer.

- [Microsoft SHDoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shdodragdrop)
- [Microsoft IShellItem::BindToHandler](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitem-bindtohandler)
- [FFmpeg audio filters](https://ffmpeg.org/ffmpeg-filters.html)
- [Exact approved Gyan release](https://github.com/GyanD/codexffmpeg/releases/tag/9.0.1)

## Validation

- Unit tests: mixed audio mapping/budget, invalid requests, preserved aspect ratio,
  no upscaling, bounded cache and recent-file retention; existing queue, cancellation,
  size enforcement and media-validation tests.
- Browser regression `test-player-sharing.js`: theme/log location, bottom inset,
  navigation in all modes, centered transport, one fullscreen exit, original and
  prepared drag commands, cancellation, failed preparations, Escape, and list actions.
  Edge screenshot runner `check-player-sharing.mjs`: 1280×800, 1440×900, 1920×1080.
- Existing fullscreen, hotkey/favorites, and compact-library regressions.
- Native approved-toolchain test on a real three-audio-track recording:
  10 MB → 7,689,920 bytes; 50 MB → 36,781,390 bytes; 100 MB → 67,687,877 bytes.
  All three completed full decode validation and contained the planned stereo mix.
- Installed-app preparation also produced the 7.69 MB copy using automatically
  discovered local tools. Settings file hashes matched before/after installation.
- A native Windows test read `CF_HDROP` from the actual Shell data object and
  verified the complete file path, including spaces. The automated cross-window
  mouse-drag test was blocked by Windows input injection (`SendInput` error 87;
  legacy `mouse_event` never held the left button).
- The user manually dragged the prepared 10 MB copy into Discord and confirmed
  that the attachment appeared. Sending the message was not part of this test.
- Formatting, TypeScript, ESLint, Clippy with warnings denied, and all 111 selected
  unit tests passed. The release installer built successfully and was installed;
  the installed frontend includes the final sharing/audio-pause changes.

The ready-thumbnail follow-up passed TypeScript, ESLint, formatting, and browser
checks at 1280 x 800, 1440 x 900, and 1920 x 1080. Tests verify that the thumbnail
loads paused, passes pointer gestures to the prepared-file drag handler, and does
not appear for failed exports. The rebuilt Windows app displayed a real clip's
thumbnail after installation; the settings file hash stayed unchanged. Discord's
native drop was not repeated for this frontend-only follow-up.

Actual Discord upload acceptance and playback depend on the receiving application
and account limit. These checks do not claim a Discord message was sent or that
every encoder/device combination has been tested.
