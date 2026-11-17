# ADR 0086: Explicit DXGI cropped-game mode with focus-away freeze

Date: 2026-11-17

Expose `game-guarded` as an explicit capture setting rather than silently replacing
WGC Game/Window capture. It uses DXGI monitor cropping and does not require a WGC
session/outline. UI must disclose that focused overlays/overlapping windows may be
captured, and excluded targets remain denied (ADR 0085).

Keep a single independent GPU texture containing the latest foreground crop.
While unfocused, emit copies of that snapshot with advancing timestamps, retaining
format/replay history rather than changing to minimized bounds. Until the first
valid game image, use black. On return, require a fresh foreground desktop update
before using live pixels. Clear/reset the snapshot on a genuine canvas resize.
The snapshot is independent of published encoder leases and adds one bounded
canvas texture; no CPU readback, disk work or UI media processing is introduced.

Transient background monitor changes must not restart capture. Genuine foreground
monitor/device changes and substantial resizes still use existing recovery/epoch
semantics and may discard history. Sampling foreground state is not atomic with
desktop composition; this is not an all-frame privacy guarantee.

WGC remains the isolated-window option. Its own resize/Alt-Tab history issue is
not solved by this DXGI mode. Native focus-away/resume footage must be inspected
before claiming game-specific compatibility or that the requested behavior is
hardware verified.

## Initial validation

`scripts/check.ps1` passed formatting, Clippy, workspace tests, TypeScript, ESLint
and Vite build. Configuration regression accepts the new mode and rejects an
empty explicit target. A live VALORANT DXGI replay exported/decoded successfully:
25.033s, 2560x1438, 765 GPU inputs, zero encoder readbacks. A representative frame
shows actual VALORANT lobby pixels. Evidence directory:
`C:\Users\Home\AppData\Local\Temp\opencode\valorant-guarded-shield-20261117-002351`.

The enclosing scripted focus helper exceeded its 120s tool deadline after export;
its away/return sequence is not verified. Do not claim the freeze/buffer continuity
requirement passed from that run. The installed app and installer have not yet
been rebuilt with this change. WGC's existing Alt-Tab behavior remains separate.
