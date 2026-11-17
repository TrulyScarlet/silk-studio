# VALORANT non-injected WGC recheck

User explicitly approved **VALORANT WGC only** in this session. No helper was
loaded, no game/security/driver setting changed and no monitor fallback used.

Environment: Windows 10 Pro 10.0.19045, NVIDIA GeForce RTX 2080 Ti, reported driver
32.0.16.2124. Game file version was not available through the ordinary process
query; no elevation/access workaround was attempted. Graphics API and exclusive
fullscreen mode are not independently verified.

## Observed sequence

1. Fresh selection: PID 15500, HWND 1115896, process creation FILETIME
   134391146046351377. Selected window foreground and not minimized.
2. Existing production WGC probe resolved **30 frames at 2560x1440**, ending at
   966 ms. Test-only scaled preview had 416 sampled colors and 225,088 nonblack
   pixels. Visual inspection showed VALORANT's video settings page, not desktop.
   This is a settings-screen capture probe, not gameplay/replay acceptance.
3. A separate hardware-only RecorderEngine save/decode acceptance attempt against
   the same explicit identity failed at engine start with Capture(PermissionDenied).
   It produced no accepted replay. Do not count this attempt as a pass.
4. A fresh metadata/production-denial probe confirmed the same window now reports
   **display affinity 17 (capture exclusion)**. Production capture denied it,
   exposed no backend/GPU context, captured zero frames, and did not fall back.
   Further capture attempts were stopped, rather than retrying around exclusion.

The earlier successful probe does not establish stable VALORANT support. Capture
availability changed within this session; cause is unconfirmed. No anti-cheat or
account-safety conclusion is drawn. In this current excluded state, replay,
fullscreen, Alt-Tab/minimize, audio and endurance acceptance remain blocked.

Artifacts under approved temp:
- Successful initial probe/visually checked preview:
  `C:\Users\Home\AppData\Local\Temp\opencode\silk-valorant-wgc-79d50c1ff25940368b2065a1a466d2c1`.
- Failed replay attempt output directory (no accepted MP4):
  `C:\Users\Home\AppData\Local\Temp\opencode\silk-valorant-replay-997701e756f746468290774480951276`.
- Explicit blocked recheck with JSON evidence:
  `C:\Users\Home\AppData\Local\Temp\opencode\silk-valorant-recheck-b61da336dbc9496c9e5f4461483afbef`.

Probe executable SHA-256:
`600837F68F158240ACB750682135E7179EA216EED66E573CCD4FB72A5061FF89`.
The new desktop selected-game replay acceptance test requires explicit consent,
fresh identity, approved decoder hash and approved temp output. It is ignored by
default and does not load the experimental helper or start monitor capture.
