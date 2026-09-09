# Segment S0 Progress Log — Foundations and repository scaffold

**Status:** Exited 2026-08-26
**Started:** 2026-08-26
**Plan reference:** [implementation-plan.md](../implementation-plan.md) §4

## Toolchain verification (2026-08-26)

| Tool | Version |
|---|---|
| cargo | 1.98.0 (797e8a9bc 2026-08-05) |
| rustc | 1.98.0 (88d9e12ae 2026-08-18) |
| node | v22.23.2 |
| npm | 10.9.8 |

## Task checklist (plan §4.4)

1. [x] Workspace scaffold matching spec §12
2. [x] Governance files: `AGENTS.md`, `README.md`, `LICENSE`, `THIRD_PARTY_LICENSES.md`
3. [x] `media-types`: StreamId, MediaType, TimeBase, EncodedPacket, VideoFrame, AudioFrame, PacketPayload
4. [x] `configuration`: versioned JSON schema, load/save, migration framework, validation
5. [x] `diagnostics`: structured JSONL logging, rotation, redaction
6. [x] `recorder-engine` skeleton: state machine per spec §19 + engine lifecycle
7. [x] Trait definitions: VideoCapture / AudioCapture / VideoEncoder / AudioEncoder / Muxer
8. [x] Mock implementations in `test-support` (scripted captures, fail-injectable encoder/muxer)
9. [x] `app-controller` skeleton: commands/events, notification dispatcher, save-job stub, single-instance guard
10. [x] Desktop shell: Tauri 2 + React 18 + TypeScript; `ping` command round-trip and `controller-event` listener wired end-to-end
11. [x] CI scaffolding: `.github/workflows/ci.yml` (windows-latest: fmt/clippy/test + typecheck/lint/build) and `scripts/check.ps1`

## Exit criteria (plan §4.7)

- [x] Workspace builds clean on Windows (17 crates + Tauri shell)
- [x] fmt / clippy -D warnings / typecheck / lint / tests pass — final run via `scripts/check.ps1`: "All gates passed."
- [x] State machine rejects invalid transitions with structured errors (`InvalidTransition`, exhaustive table tests, 6/6 pass)
- [x] Config loader handles missing file (defaults+warning), corrupt JSON (error), old versions (v0→v1 merge migration), newer versions (rejected), unknown keys (rejected)
- [x] No native WGC / WASAPI / FFmpeg / hotkey code present (stub crates are doc-only; verified by inspection)
- [x] `AGENTS.md` and license inventory template exist

## Command log

```text
cargo --version / rustc --version / node --version / npm --version
  → cargo 1.98.0, rustc 1.98.0, node v22.23.2, npm 10.9.8
rustup component add rustfmt clippy          → installed
cargo check --workspace --all-targets        → clean after fixes below
cargo clippy --workspace --all-targets -- -D warnings
  → Finished dev profile, 0 warnings
cargo fmt --all -- --check                   → exit 0
npm install                                  → added 173 packages
npm run typecheck / lint / build             → all exit 0 (vite build 145 KiB JS)
cargo check -p silk                           → compiles after adding icons/icon.ico
scripts/check.ps1 (final gate run)           → "All gates passed."
cargo test totals: app-controller 4, configuration 10, diagnostics 7,
  media-types 7, recorder-engine 6, test-support 2+unit, integration/e2e 4,
  plus single_instance/jobs/temp_dir suites — 0 failures.
```

### Issues found and fixed during verification

1. serde needed the `rc` feature for `Arc<EncodedPacket>` snapshots.
2. Redaction originally ran on serialized JSON; backslash escaping let Windows
   paths slip past substring matching. Moved to pre-serialization redaction of
   raw message/field strings (`Redactor::redact_value`); regression test added.
3. The engine never configured the encoder or called `capture.start(...)`;
   mocks correctly refused to run. `RecorderEngine::start` now configures both
   halves before any state transition and restores parts on failure.
4. `windows-sys` needed the `Win32_Security` feature for `CreateMutexW`.
5. tauri-build requires `icons/icon.ico`; generated a minimal valid 16×16
   32-bpp icon programmatically.
6. State-machine test bug caught by the final gate script: the abort-edge test
   incorrectly included `Stopping -> Stopping`. Test fixed (table itself was
   correct). Lesson recorded: rely on the full gate script, not grep-filtered
   output, for pass/fail conclusions.

## Deviations and decisions

- **EngineConfig grew** `video_source_id` + `video_encoder` params so start()
  configures capture+encoder explicitly; documented in code docs.
- **State machine additions** (Degraded edges, abort paths) recorded in ADR 0003.
- **Configuration policy** (defaults-on-missing, strict unknown keys,
  atomic saves, additive migrations) recorded in ADR 0001.
- **Concurrency baseline** (std threads + bounded `sync_channel`, single
  video worker in S0, bounds table) recorded in ADR 0002 +
  docs/architecture/threading.md.
- The engine's replay store is a temporary count-bounded packet ring;
  the real duration-based buffer lands in Segment S1 per plan.
- Project LICENSE is an MIT placeholder pending the formal §26 review.

## Untested / known limitations (spec §30 rules 8–9)

### Risk closure pass (2026-08-26, pre-S1)

| Original risk | Action taken | Result |
|---|---|---|
| Tauri window compiled, not launched | Refactored IPC handlers into tested `shell` module (`shell.rs`, `shell_tests.rs`: ping round-trip + full lifecycle through real handler code); added `scripts/smoke-desktop.ps1` | Shell logic 2/2 unit tests pass; **real exe launched, stayed alive 6 s, killed cleanly** |
| Single-instance only tested in-process | Added `instance-probe` helper binary + `tests/single_instance_cross_process.rs` | Cross-process test passes: second process rejected while holder alive; slot frees after exit |
| eslint 9 deprecated | Upgraded to eslint 10.9.1 + typescript-eslint 8.68 | lint green, no deprecation warning |
| Elevated-process mutex visibility | Cannot elevate in this environment | **Still untested** — named-mutex namespace is session-global by OS design; revisit in S9 hardening matrix |
| LICENSE placeholder | Requires maintainer decision | Open item for S9 release review |

Gates re-run after fixes: `scripts/check.ps1` → "All gates passed."

### Residual notes

- The smoke test proves startup liveness, not visual rendering or click-through IPC (needs interactive session; covered by S6/S7 manual QA).
