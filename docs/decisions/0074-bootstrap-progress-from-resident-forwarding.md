# ADR 0074: Bootstrap progress belongs to the resident forwarding worker

- Date: 2026-11-13
- Status: Implemented and validated in owned bootstrap

The bootstrap worker depended on a counter incremented by the renderer's Rust
module. A separately loaded DLL would have different globals, so this was not an
independent progress source suitable for the attachment proposal in ADR 0073.

Install the resident selected-chain Present trampoline in forwarding-only mode
first. It performs no media submissions while forwarding at least twelve actual
selected Presents. Fail within three seconds on no progress, cancellation or hook
failure. Confirm zero captured publications during that warmup. Only then allocate
and stage initial GPU media and arm capture once, before the existing exact parent
approval/delivery protocol. This is genuine hook-observed progress, not owner
telemetry interpreted as worker progress.

Initial arming is one-shot on the same selected chain. A sticky cancellation gate
prevents cancellation before/during arming from reopening capture. Activation
rechecks cancellation after publishing ACTIVE to close the race where cancel
occurred between admission and activation. Existing resize/device-recreation
activation remains separate and does not reinstall forwarding trampolines.

Use those same selected forwarding counts for stop/progress reports and require
twelve further actual Presents after cancellation. Remove the executable-private
present counter entirely. Fixture-only window-action mailboxes remain; this does
not implement remote loading, IPC authentication, pre-existing discovery, real
games or in-flight loss recovery. The eight-hour snapshot uses its preserved older
build and is not evidence for this new startup sequence.

Deterministic admission/cancellation regressions and owned modern/legacy
WARP/hardware lifecycle, recorder and recreation tests validate the change. No
dependencies, versions, licenses or production desktop backend changed.
