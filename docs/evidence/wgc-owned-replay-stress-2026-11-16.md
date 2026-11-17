# WGC owned-window replay stress — 2026-11-16

## Scope and implementation

`apps/desktop/src-tauri/tests/native_owned_window_replay.rs` starts an animated
Win32 window in a separate owned process and selects its exact process lifetime
and HWND using `game:`. The capture wrapper requires `wgc-window` on every start.
The recorder uses hardware-only H.264, native source aspect/dimensions, a bounded
one-second replay buffer and the production MP4 muxer. No hook is used.

The test saves three independent MP4s initially and after each of five
minimize/restore/resize cycles. It decodes all eighteen with the approved FFmpeg
SHA-256 `57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A`.
Decoder passthrough/demux timebase avoids null-output timestamp quantization;
the initial decoder invocation without this setting logged duplicate output-DTS
warnings and is not counted as clean decoder evidence below.

The source is then stopped and its process exits. A second owned process starts;
the old selection must fail with `SourceUnavailable`, while fresh explicit
selection must start WGC. The second process applies its own capture exclusion;
startup must return `PermissionDenied` with no backend or GPU context. This does
not alter a game or unrelated application's capture policy.

## Bug reproduced and fixed

WGC published a `146x21` minimized/taskbar representation, including during
transition windows where `IsIconic` alone was insufficient. The NVIDIA hardware
encoder rejected this format (`0xC00D6D76`) and the recorder entered Error.

WGC now skips minimized/empty-client publications and content smaller than the
current client bounds, including stale minimized frames arriving after restore.
These inputs do not start a new encoder epoch. Restored valid content still
triggers the existing resize path. A deterministic regression covers empty,
stale-thumbnail and valid window extents. This relies on WGC capturing the full
window (including non-client bounds), not a client-only subregion.

## Results

Three consecutive stress runs saved and cleanly decoded 54 MP4s through 15
restore/resize cycles. Each reported zero encoder CPU readbacks and 262–265 GPU
inputs. Stale target rejection and fresh target startup passed in every run.

An additional run with external owned-window exclusion verification passed all
18 saves/decodes, five restore/resize cycles, stale/fresh selection, and exclusion
denial without fallback. It reported 269 GPU inputs and zero encoder readbacks.
One phase confirmed another owned window as foreground; other attempted focus
switches were denied by Windows. No forced focus/anti-cheat bypass was attempted.

Latest evidence directory:

`C:\Users\Home\AppData\Local\Temp\opencode\silk-owned-replay-19128-1794811841007383200`

Reproduce:

```powershell
$env:SILK_OWNED_REPLAY_FFMPEG = '<approved ffmpeg.exe>'
cargo test -p silk --test native_owned_window_replay owned_window_replays_survive_resize_focus_and_restart -- --ignored --nocapture
```

Dependency inventory also passed: 545 Cargo registry packages and 11 direct
frontend dependencies. No new dependency was introduced.

`scripts/check.ps1` passed after these changes: formatting, Clippy, workspace
tests, TypeScript, ESLint and production frontend build. The separate native
overlap/minimize/resize/close test also passed again after the minimized-content
filter was added. Interactive tests remain opt-in in the standard gate; their
explicit executions above are separate evidence.

## Local installer build

Tauri NSIS build passed after validation. Application:
`target/release/silk.exe`. Installer:
`target/release/bundle/nsis/Silk_0.1.4_x64-setup.exe` (4.05 MiB), SHA-256:
`315E09D529A2CB0EF60CB814767E3E4C9705E84C1DF95AB5BA3BD25E467A6318`.
An updater `.sig` was generated using the existing local signing setup. This is
not a publisher/Authenticode signing claim. Build identity is
`e861c20-dirty-wgc-test`; source remains uncommitted. This local test build was
not installed or published and is not a clean-source public release.

## Remaining limitations

- Real gameplay, anti-cheat compatibility, HDR, exclusive fullscreen, multi-GPU
  or cross-monitor transitions.
- Audio synchronization in these new video-only clips.
- Full-frame privacy of exported clips: the separate native overlap test checks
  a controlled window center pixel, not every encoded pixel.
- Automatic adoption of a restarted game: a fresh explicit identity is required.
- Long-duration endurance or every possible minimize/resize timing race.

Real-game tests remain for the user when available. Hook development stays paused.
