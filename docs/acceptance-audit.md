# MVP Acceptance Audit

This is the current evidence ledger for the final S8/S9 sweep. `Pass` means a
deterministic test or command was actually run. `Manual` and `Blocked` are not
passes.

| Area | Status | Evidence or remaining work |
|---|---|---|
| Repeated start/stop and rejected-start retry | Pass | `tests/endurance/tests/s8.rs` |
| One hundred sequential saves | Pass | `one_hundred_sequential_saves_complete_without_staged_files` |
| Save while capture/library work is active | Pass | `saving_and_library_scan_overlap_an_active_capture`; save worker uses bounded Windows background processing mode |
| Bounded engine event delivery | Pass | Capacity-64 overflow test |
| Scripted display/audio loss recovery | Pass | S8 scripted recovery tests |
| Configurable ordered audio tracks and named streams | Pass | Configuration migration/validation, controller plan, audio-sync gain, and separate multi-track AAC tests |
| User-selected base directory and managed `Silk` game folders | Partial | Path resolution, bounded hierarchy, sanitization, and attributed-save tests pass; native picker and interactive Windows path change remain |
| Library game metadata and game filter | Pass | Catalog v2 metadata, one-level scan, nested-path safety, frontend filter, and attributed-save coverage |
| Capture confirmation HUD | Partial | Native DirectComposition HUD with Full/Compact layouts and save-event wiring is integrated; deterministic and native lifecycle tests pass, while interactive game frame pacing, click-through, placement, and capture-exclusion tests remain |
| Native display/audio/GPU recovery | Partial | Native WGC display capture, same-device WGC GPU-surface-to-MF encoding, deterministic format-change handling, and scripted display-loss encoder reconfiguration passed; physical loss/reconnect, resize on hardware, other GPU vendors, and long-run drift remain |
| Sleep/resume epoch separation | Pass | Deterministic resume test; OS sleep/resume still manual |
| Encoder fallback | Pass | Injected encoder failure test |
| Process kill during native save | Blocked | Startup stale `.part` cleanup is tested; kill timing is not reproduced |
| Eight-hour memory/handle/GPU/thread trend | Blocked | Long-duration run not performed |
| User-triggered redacted diagnostics | Pass | Native JSON export and redaction test; full gate passed |
| Build/version/capability information | Partial | Metadata and probes compile; runtime GPU/driver/MF values require the desktop on supported Windows hardware |
| Tauri CSP and bundle metadata | Pass | Production CSP and NSIS/WebView2 policy configured and parsed |
| Dependency/license approval | Partial | The exact Gyan FFmpeg 9.0.1 GPL build is approved as standalone validation tooling; other inventory rows and release notices remain provisional |
| Reproducible release inputs | Partial | Locked dependencies and pinned Rust toolchain exist; CI provenance/build hash flow remains |
| Signed installer and executable | Blocked | Certificate/timestamp material is unavailable; local signing tools and verification scripts are installed |
| Clean-machine install and uninstall | Blocked | Manual Windows test required |
| Direct MP4 output and media validation | Partial | Generated H.264/AAC MP4 fixtures passed `scripts\validate-media.ps1`; hardware player matrix and third-party decoder compatibility remain |
| Direct single-pass MP4 replay output | Pass | Pure-Rust direct MP4 muxer writes H.264/AAC snapshots with trailing moov box and separate audio tracks without intermediate containers or re-encoding |
| Startup registration | Manual | `startWithWindows` writes the per-user Run value; clean-login behavior still requires Windows testing |

## Commands Run

```text
cargo fmt --all -- --check                              Pass
cargo clippy --workspace --all-targets -- -D warnings   Pass
cargo test --workspace -- --test-threads=1              Pass
cd apps/desktop && npm run typecheck                    Pass
cd apps/desktop && npm run lint                         Pass
cd apps/desktop && npm run build                        Pass
pwsh -NoProfile -File .\scripts\check.ps1             Pass
pwsh -NoProfile -File .\scripts\smoke-desktop.ps1     Pass (process alive for 6 s)
pwsh -NoProfile -File .\scripts\release-preflight.ps1 Pass with expected warnings
pwsh -NoProfile -File .\scripts\verify-dependencies.ps1 Pass (498 Cargo packages, 11 frontend dependencies)
cargo test -p encoder-media-foundation -- --ignored --test-threads=1 Pass (native MF tests, including WGC GPU input)
cargo test -p audio-wasapi -- --ignored --test-threads=1 Pass with rendered tone (native loopback)
cargo test -p capture-windows --test native_display -- --ignored --test-threads=1 Pass (7 native WGC tests, including D3D11 conversion)
SILK_REAL_CAPTURE=1 cargo test -p silk shell_tests::command_handlers_drive_full_lifecycle_and_emit_events -- --exact --test-threads=1
                                                        Pass (native WGC capture lifecycle with mock encoder)
cargo test -p clip-export -- --test-threads=1       Pass (21 planner/queue tests)
cargo test -p encoder-ffmpeg -- --test-threads=1    Pass (24 exporter/process/toolchain/worker tests)
scripts\validate-media.ps1 ...                     Pass (temporary generated MP4 fixtures)
scripts\verify-ffmpeg-toolchain.ps1 ...            Pass (approved Gyan 9.0.1 pair and exact hashes)
```

The complete `scripts/check.ps1` gate and desktop smoke test passed after the
direct MP4, ordered audio-track, and capture-overlay integration. Native WGC
capture, same-device D3D11 BGRA-to-NV12 conversion, Media Foundation GPU-surface
encoding, pure-Rust direct MP4 muxing with trailing moov, and the tone-backed
WASAPI test passed on the interactive Windows development machine. No CPU
readback or intermediate containers were introduced. These results do not
verify the interactive Tauri window, NVIDIA/Intel hardware encoder bitrate
enforcement (`MF_MT_AVG_BITRATE`), monitor topologies, device-swap recovery,
or release packaging/source delivery for third-party tools.

Strict dependency verification remains blocked by the exact failure
`License inventory still contains provisional approvals.`
