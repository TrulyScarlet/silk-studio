# Upstream provenance

This crate is an attributed local fork of [`mp4` 0.14.0](https://crates.io/crates/mp4/0.14.0)
from <https://github.com/alfg/mp4-rust>.

- Upstream author: Alfred Gutierrez
- Upstream release: 0.14.0
- License: MIT; the upstream copyright and license are retained in `LICENSE`
- Local package: `silk-mp4` 0.14.0-silk.1

Silk maintains this fork so its direct, zero-reencode MP4 writer can gain
standards-compliant HEVC (`hvc1`/`hvcC`) and AV1 (`av01`/`av1C`) support while
retaining the existing AVC, AAC, sample-table, reader, and writer behavior.
Those codec extensions are tracked separately from this baseline import.
