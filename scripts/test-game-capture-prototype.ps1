# Development-only hook gate. Native runs open only the fixture's own windows.
# No games, running user applications, or installed Silk instance are targeted.
param(
    [switch]$Native,
    [switch]$Hardware,
    [switch]$Decode
)

$ErrorActionPreference = 'Stop'

cargo test -p capture-hook-prototype --all-features
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo clippy -p capture-hook-prototype --all-features --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy -p capture-hook-prototype --features d3d12-fixture --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy -p capture-hook-prototype --features experimental-hooks --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy -p capture-hook-prototype --features d3d12-bootstrap-fixture --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy -p capture-hook-prototype --features d3d12-rearm-fixture --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo test -p capture-hook-helper --all-features
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo clippy -p capture-hook-helper --all-features --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if ($Hardware -and -not $Native) {
    throw '-Hardware requires -Native so opening native fixture windows is explicitly approved.'
}
if ($Decode -and -not $Native) { throw '-Decode requires -Native.' }

if ($Native) {
    cargo build -p capture-hook-helper --all-features
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $previousFfmpeg = $env:SILK_HOOK_TEST_FFMPEG
    try {
        if ($Decode) {
            $ffmpeg = if ($previousFfmpeg) { $previousFfmpeg } else { Join-Path $env:LOCALAPPDATA 'Silk\tools\ffmpeg\bin\ffmpeg.exe' }
            $hash = (Get-FileHash -LiteralPath $ffmpeg -Algorithm SHA256).Hash
            if ($hash -ne '57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A') {
                throw 'Decoder is not the exact approved FFmpeg artifact; no executable will be run.'
            }
            $env:SILK_HOOK_TEST_FFMPEG = $ffmpeg
        }
        cargo run -p capture-hook-prototype --features experimental-hooks --bin silk-d3d11-hook-fixture -- --allow-self-hook --warp
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo run -p capture-hook-prototype --features experimental-hooks --bin silk-d3d11-hook-fixture -- --allow-self-hook --transport-test --warp
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo run -p capture-hook-prototype --features fixture-probes --bin silk-d3d11-hook-fixture -- --allow-self-hook --lifecycle-test --warp
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo run -p capture-hook-prototype --features experimental-hooks --bin silk-d3d11-hook-fixture -- --allow-self-hook --helper-test --warp
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo run -p capture-hook-prototype --features resident-hooks --bin silk-d3d11-hook-fixture -- --allow-self-hook --resident-test --warp
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --features d3d12-fixture --test d3d12_native _warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --features d3d12-fixture --lib queue_owner_warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --features d3d12-gpu-fixture --test d3d12_gpu_native _warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --features d3d12-cross-process --test d3d12_cross_native _warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --features d3d12-factory-hook-fixture --test d3d12_hook_native _warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --features d3d12-bootstrap-fixture --test d3d12_bootstrap_native _warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --features d3d12-bootstrap-fixture --test wgc_owned_native -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --features d3d12-rearm-fixture --test d3d12_resize_native _warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --features d3d12-device-replace-fixture --test d3d12_device_replace_native _warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --all-features --test d3d12_attachment_native _warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p capture-hook-prototype --all-features --test controlled_recorder _warp -- --ignored --test-threads=1 --nocapture
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        if ($Hardware) {
            cargo test -p capture-hook-prototype --all-features --test d3d12_attachment_native _hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p gpu-windows native_conversion_preserves_visible_pixels_at_native_resolution -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo run -p capture-hook-prototype --features experimental-hooks --bin silk-d3d11-hook-fixture -- --allow-self-hook
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo run -p capture-hook-prototype --features experimental-hooks --bin silk-d3d11-hook-fixture -- --allow-self-hook --transport-test
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo run -p capture-hook-prototype --features fixture-probes --bin silk-d3d11-hook-fixture -- --allow-self-hook --lifecycle-test
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo run -p capture-hook-prototype --features experimental-hooks --bin silk-d3d11-hook-fixture -- --allow-self-hook --helper-test
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo run -p capture-hook-prototype --features resident-hooks --bin silk-d3d11-hook-fixture -- --allow-self-hook --resident-test
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p capture-hook-prototype --features d3d12-fixture --test d3d12_native _hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p capture-hook-prototype --features d3d12-fixture --lib queue_owner_hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p capture-hook-prototype --features d3d12-gpu-fixture --test d3d12_gpu_native _hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p capture-hook-prototype --features d3d12-cross-process --test d3d12_cross_native _hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p capture-hook-prototype --features d3d12-factory-hook-fixture --test d3d12_hook_native _hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p capture-hook-prototype --features d3d12-bootstrap-fixture --test d3d12_bootstrap_native _hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p capture-hook-prototype --features d3d12-rearm-fixture --test d3d12_resize_native _hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p capture-hook-prototype --features d3d12-device-replace-fixture --test d3d12_device_replace_native _hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo test -p capture-hook-prototype --all-features --test controlled_recorder _hardware -- --ignored --test-threads=1 --nocapture
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        }
    } finally { $env:SILK_HOOK_TEST_FFMPEG = $previousFfmpeg }
}
