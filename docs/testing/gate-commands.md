# Testing notes — gate commands and conventions

## Gates (every task, per spec §30 rule 6)

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/desktop && npm run typecheck && npm run lint && npm run build
```

Or run everything via `scripts/check.ps1` (repository root). CI runs the
same gates on `windows-latest` (`.github/workflows/ci.yml`).

## Conventions established in S0

- Unit tests live inside each crate (`#[cfg(test)] mod tests`).
- Cross-crate behavior lives in the `integration-tests` crate
  (`tests/integration/tests/e2e.rs`) using mock backends from
  `test-support`; no native capture/encode code is exercised there.
- Failure injection: mocks expose fields like `fail_on_encode` /
  `fail_on_call` rather than hidden global toggles.
- Temp artifacts use `test_support::TempDir`, cleaned on drop.
- Determinism: scripted generators are fully deterministic; seeds for
  randomized property tests arrive with Segment S1
  (`tests/fixtures/seeds.json` reserved).

## Media validation (from S5 onward)

Per plan §15, run the repository harness from the root:

```powershell
scripts\validate-media.ps1 -ClipPath <clip.mp4>
# Use explicit paths when the approved pair is not on PATH.
scripts\validate-media.ps1 -ClipPath <clip.mp4> -FfmpegPath <ffmpeg.exe> -FfprobePath <ffprobe.exe>
scripts\verify-ffmpeg-toolchain.ps1 -FfmpegPath <ffmpeg.exe> -FfprobePath <ffprobe.exe>
```

It checks H.264/AAC streams and metadata, positive duration, per-stream DTS
ordering, and a complete `ffmpeg` decode. The harness reports a clear missing
tool error when `ffprobe` or `ffmpeg` is not installed.

The toolchain command also prints both binary hashes and records the verified
version, build configuration, and required capabilities. It does not download
or approve a third-party build; maintainer approval still requires matching
those records to the release artifact and license/source paperwork.
