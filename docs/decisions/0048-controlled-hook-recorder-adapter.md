# ADR 0048: Controlled hook source through the recorder interface

- Date: 2026-11-11
- Status: Accepted for a development-only adapter

ADR 0049 adds an explicit GPU-frame bridge; this record describes the original
CPU-input milestone, which remains available as a comparison mode.

## Decision

Implement `VideoCapture` for the explicitly launched cooperative D3D11 fixture,
under the default-off experimental-hooks feature. Only the reserved
`fixture:d3d11` source is accepted; no desktop/game/window source IDs, user
settings entry, arbitrary process attachment, or injection is introduced.

A capture worker owns the Child, pipe reader, imported D3D11 device, and keyed
mutex leases. A bounded three-event channel separates it from the encoder worker.
Startup reports success only after the identity/adapter/texture handshake passes.
Capture controls and frame pacing run outside Present. Shutdown interrupts
deadline waits, closes channels, kills/waits for only the owned Child, and joins
its reader. Peer failures fail closed, without monitor fallback or automatic
attachment to another process. Driver calls remain an unbounded platform risk.

For this first recorder integration, read shared textures back to immutable BGRA
CPU frames **on the capture worker**, not the UI or Present callback. Release
GPU ownership and ACK only after that readback. This intentionally validates the
existing CPU-input encoder path before introducing cross-thread GPU ownership.
It is not a zero-copy or production-performance implementation. A future GPU
adapter must copy into a bounded local lease ring before acknowledging imported
slots, so encoder work cannot race cross-process slot reuse.

Convert QPC deltas to milliseconds with checked wide arithmetic and a
session-local origin. Reject backward/overflowing timestamps. Source dimensions
come from the validated fixture surface, not the monitor. Native acceptance
uses the existing RecorderEngine, Media Foundation encoder, replay buffer, and
MP4 muxer, writing only generated test media to isolated temporary storage.

## Gates and limitations

Add deterministic timestamp tests and native start/stop/failure tests, then save
a replay from the real hook/transport/encoder pipeline. Verify dimensions,
duration, keyframe/codec metadata, and container structure; decode with the
already-approved FFmpeg toolchain if available. No new third-party package is
required: all encoder/recorder dependencies already exist in the workspace.

Actual game attachment, concurrent hook lifecycle, cross-process resize/rearm,
audio synchronization with games, fullscreen/anti-cheat compatibility, and
production packaging remain separate milestones.
