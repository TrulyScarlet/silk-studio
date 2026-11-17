# Independent hook lifecycle fixture evidence

Date: 2026-11-11. Scope/design: ADR 0050. Original owned renderers only.

## Reproduction and results

`scripts/test-game-capture-prototype.ps1 -Native -Hardware -Decode` passed,
including the new `--features fixture-probes -- --allow-self-hook
--lifecycle-test` runs with WARP and with the local RTX 2080 Ti hardware path.

Both runs verified:

- Two independently paced renderer threads, each owning its HWND/device/chain,
  calling the same patched Present entry. Only the selected source was captured.
- Rendering progress despite a full capture queue and while the selected callback
  was deliberately held at a bounded test checkpoint.
- Resize from 640x480 to 800x600 invalidated capture without publishing incompatible
  frames. Explicit quiescent detach/reattach established a new capture epoch.
- Cancellation while a capture permit was held did not falsely report a drain.
  Releasing the checkpoint allowed Present to continue without new publications.
- One injected DXGI_ERROR_DEVICE_REMOVED result closed capture while the real
  original Present continued succeeding. This is simulated notification only.
- Quiescent detachment, recreation of the fixture's own device at 960x540, and
  capture of the new blue source in a fresh epoch. Retained red/green frames kept
  their pixels throughout resize, cancellation, detachment, and device recreation.

Twenty portable prototype tests passed, including three admission-gate tests;
two safe adapter tests and all seven opt-in native recorder tests also passed.
Hardware GPU replay again reported 12 GPU-surface inputs and zero readbacks;
approved FFmpeg decoded its MP4. Probe-enabled and probe-disabled Clippy passed.

Workspace formatting/lint/tests and frontend checks (`scripts/check.ps1`), settings
save regression, and strict dependency verification all passed.

## Boundaries

Capture cancellation is not authorization to free trampoline memory or unload a
DLL. Every hook removal here requires the separate fixture-wide render gate to be
closed/drained and both renderer threads to acknowledge pause. The fixture retains
the tap on uncertain drain rather than freeing executable code under a caller.
GPU driver calls and thread joins have no hard completion guarantee.

The captured swap chain still has one owner thread with TLS capture state; this
does not establish support for arbitrary concurrent calls on that chain. The
independent lifecycle scenario is in-process, not an independent cross-process
export/rearm test. No physical GPU removal/reset, game attachment, safe arbitrary
game shutdown, exclusive fullscreen, Valorant/anti-cheat, other GPU vendors, soak
performance, or production packaging was tested. Experimental code remains
default-off; nothing was released.
