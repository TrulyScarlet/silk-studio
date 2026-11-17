# Controlled D3D11 Present hook: native evidence

Date: 2026-11-11. Design: ADR 0046. Development prototype only.

Later cooperative cross-process results are recorded separately in
`cross-process-hook-transport-2026-11-11.md`; the results here are the original
in-process gate, not a statement that transport is still absent.

## Environment actually tested

- x64 Windows, OS version 10.0.19045, build 19045.
- Hardware adapter reported by D3D11: NVIDIA GeForce RTX 2080 Ti.
- Windows-reported graphics driver version: 32.0.16.1742.
- Software adapter: Microsoft Basic Render Driver, requested explicitly with WARP.
- Debug build, D3D11, BGRA8, single-sampled windowed swap chains.
- Source sizes: 640x480, then 800x600 after resize.

## Reproduction

```powershell
.\scripts\test-game-capture-prototype.ps1 -Native -Hardware
```

This opens only the original fixture's own windows and hooks its own process.
It does not enumerate games, load a DLL into another process, or use installed
Silk. Without `-Native`, the script runs deterministic tests/lint only. Without
`--allow-self-hook`, the fixture refuses to run.

## Observed results

- Five deterministic tests passed: strict identity/epoch/session checks,
  sequence/QPC ordering, metadata bounds, disconnect gate, fixed-capacity leases,
  and rejection of weak-reference resurrection during reuse.
- Clippy with all prototype features and targets passed with warnings denied.
- Full `scripts/check.ps1` gate passed: workspace formatting, Clippy, tests,
  frontend typecheck/lint/build. Unrelated hardware tests remain ignored.
- Strict dependency verification, `git diff --check`, and the existing settings
  source-save browser regression passed after adding the prototype.
- Both the WARP and hardware fixture runs passed.
- Actual calls through the swap chain's Present method reached the MinHook
  trampoline/detour; original Present continued returning successfully.
- Captured red, blue, and green GPU frames read back to the expected BGRA pixels
  across the full source area (one-code-value tolerance).
- A second, unrelated swap chain presented yellow without publishing a selected
  capture frame. A mismatched process creation time and concurrent tap were
  rejected. DXGI_PRESENT_TEST did not publish a frame.
- Retaining a red frame while filling the other two slots did not overwrite its
  pixels. Additional Presents were dropped by capture rather than waiting for
  the consumer. Retained pixels were also valid after detachment.
- Resize invalidated the old tap. Explicit reattachment created a new epoch with
  800x600 source dimensions and correct green pixels.
- Receiver disconnect invalidated capture; detachment was idempotent and the
  swap chain continued presenting after detachment.

## What this does not establish

- No game, exclusive fullscreen, Alt-Tab, minimize, anti-cheat, AMD, Intel,
  multi-GPU, D3D12, Vulkan, OpenGL, or antivirus acceptance was tested.
- This original gate did not test DLL injection, cross-process shared texture transport, authenticated IPC,
  recorder-engine hook backend, settings opt-in, or shipping helper exists yet.
- The fixture guarantees one-thread rendering and quiescent hook lifecycle;
  arbitrary game threads do not provide those guarantees. Production shutdown
  and in-flight callback handling require a different, reviewed lifecycle.
- Backpressure checks show functional dropping, not a latency, FPS, memory-soak,
  or performance benchmark. GPU drivers can internally stall command submission.
- Metadata structs are not a serialized ABI or a substitute for authenticated
  cross-process communication. QPC frequency negotiation is future IPC work.
- No installer/update was published. Default production dependencies exclude the
  hook code (`cargo tree -p silk --edges normal` has neither prototype nor MinHook).
