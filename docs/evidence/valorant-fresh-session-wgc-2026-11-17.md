# Fresh-session VALORANT WGC evidence — 2026-11-17

The user supplied `C:\Users\Home\Videos\NVIDIA\Valorant\Valorant 2026.11.16 - 23.59.39.03.DVR.mp4`.
Approved FFmpeg decoded the 35.12s H.264 2560x1440/60fps + AAC stereo clip.
Two-second visual samples showed genuine practice-range gameplay, not the desktop.
No black segment of at least 0.15s was detected; near-static intervals were at most
approximately 0.52s with the selected freeze detector. The user reported Alt-Tab,
but there is no synchronized focus trace for the supplied clip, so its exact
focus-away interval and continued-background-versus-freeze behavior are unproven.

## New session, changed affinity

Read-only window audit found PID 17376, creation 134393650471241843, HWND 262286.
GetWindowDisplayAffinity succeeded and returned **0**, unlike the earlier process
18912 reporting 17. No exclusion was cleared by Silk. The cause of the changed
affinity is unknown; do not attribute it to NVIDIA or a particular setting.

The explicit selected-window WGC hardware replay probe passed on the new identity:

- Route: `wgc-window`, no injection or monitor fallback.
- Format: 2560x1440; H.264 NVIDIA hardware MFT.
- 115 encoder GPU inputs, zero encoder CPU readbacks.
- Saved/decode clip: 1730ms, 2,236,268 bytes.
- Representative frame at 0.5s visibly shows VALORANT's in-game store/menu UI.
  This is actual selected game-window content, not a claim of live match gameplay.

Evidence: `C:\Users\Home\AppData\Local\Temp\opencode\valorant-fresh-wgc-20261117-000551`.
The probe's machine JSON deliberately still says game pixels unverified; the
subsequent visual inspection above is separate evidence, not an edited test result.

The prior unsupported finding remains valid for the earlier excluded session,
but must not be generalized to this nonexcluded one. WGC works for the observed
new window. Audio synchronization, Alt-Tab recovery/privacy, fullscreen mode and
long-duration real-game WGC acceptance remain untested in this run.
