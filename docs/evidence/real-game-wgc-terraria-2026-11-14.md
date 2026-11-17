# Real-Game WGC Capture & Replay: Terraria

Date: 2026-11-14
Target: Terraria (`Terraria.exe`)

## Environment

- Operating System: Microsoft Windows NT 10.0.19045 (Windows 10 22H2)
- GPU: NVIDIA GeForce RTX 2080 Ti
- Driver Version: 32.0.16.2124 (NVIDIA 621.24)
- Display: 1920x1080 @ 60Hz
- Target Application: Terraria (`Terraria.exe`, PID 19764, HWND 11601208, creation timestamp `134391504885452527`)
- Window Title: "Terraria: Digger T' Blocks"
- Capture Method: Non-injected OS Windows Graphics Capture (`WgcDisplayCapture` / `DisplayCapture`)
- Injection: None. No graphics hooks, DLL helper, or process injection used or attempted.
- Security / Anti-Cheat: No bypasses, elevation, or OS modifications.

## Preflight & Capture Affinity

- `GetWindowDisplayAffinity(11601208)` returned `WDA_NONE` (0). Terraria does not set `WDA_EXCLUDEFROMCAPTURE` or `WDA_MONITOR`.
- Initial mode: Windowed (1152x864).
- Top-level foreground window verified.

## 1. Windowed Mode Capture Probe (60 FPS)

- Command: `cargo run -p capture-windows --example capture_probe -- --source "pid:19764:created:134391504885452527:hwnd:11601208" --seconds 5 --fps 60`
- Format observed: `1152x864`
- Delivered frames: 299 over 5.0 seconds (target 60 fps)
- Monotonic violations: 0
- Queue drops: 0
- Resizes: 0
- Source lost: false

## 2. Hardware Replay Buffer & MP4 Decode

- Pipeline: `DisplayCapture` (`wgc-window`) -> `RecorderEngine` -> Hardware Media Foundation H.264 Encoder -> `Mp4Muxer`
- Configuration: `follow_source_dimensions: true`, `target_fps: 60`, `retention_ms: 1000`
- Replay clip saved: `real-game-replay.mp4` (1749ms duration, 1152x864 resolution)
- Encoder metrics:
  - `gpu_surface_inputs`: 183
  - `cpu_readback_inputs`: 0 (zero CPU readbacks; pure GPU texture surface path)
- FFmpeg validation: Approved FFmpeg build (`57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A`) decoded all video frames with exit code 0 and zero decode/bitstream errors.
- Visual inspection: Decoded preview confirmed genuine Terraria gameplay pixels (player inventory, crafting interface, character sprite, landscape) with no desktop or background substitution.

## 3. Borderless Fullscreen (2560x1440) Capture & Replay

- Mode switched in-game to Borderless Fullscreen. Window bounds: `(0, 0, 2560, 1440)`.
- Format observed by WGC: `2560x1440`.
- 60 FPS Capture Probe: Delivered 299 frames across 5.0 seconds (target 60 fps), zero monotonic timestamp violations, zero queue drops, zero unexpected resizes, `source_lost=false`.
- Hardware Replay Buffer: `RecorderEngine` followed source dimensions to 2560x1440, buffered at 60 fps, and saved a 1716ms MP4 clip.
- Encoder metrics:
  - `gpu_surface_inputs`: 209
  - `cpu_readback_inputs`: 0
- FFmpeg validation: Full raw video decode passed cleanly with zero decode or muxer errors.

## 4. 8-Hour Real-Game Endurance Capture (28,800s @ 60 FPS)

- Execution: Detached interactive task under Windows Task Scheduler with power-management sleep prevention (`ES_CONTINUOUS | ES_SYSTEM_REQUIRED`).
- Immutable Snapshot: `C:\Users\Home\AppData\Local\Temp\opencode\silk-realgame-8hr-terraria`
- Target: Terraria (`PID 19764`, `HWND 11601208`, Borderless Fullscreen `2560x1440`).
- Start Time: `2026-11-14T21:39:39Z` (4:39:39 PM local)
- Finish Time: `2026-11-15T05:39:40Z` (12:39:40 AM local)
- Result: **Passed** (`exitCode: 0`)
- Exact Metrics:
  - Duration: `28,800.6` seconds (8.00 hours continuous)
  - Frames Delivered: `1,671,652`
  - Monotonic Timestamp Violations: `0`
  - Format Changes: `1` (initial 2560x1440 lock)
  - Unexpected Resizes: `0`
  - Source Lost: `false`
  - Compositor Copies: `6,371,860`
  - Queue Dropped: `254` (0.0039% drop rate under background/multitasking load)
  - Memory: Working set remained bounded at ~15.9 MB with 365 handles (zero leak).
  - Exit Code: `0`

## Remaining Gates

- Release packaging and signing validation.
