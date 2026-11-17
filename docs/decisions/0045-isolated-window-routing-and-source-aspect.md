# ADR 0045: Isolated window routing and source-aspect output

- Date: 2026-11-11
- Status: Accepted; game-specific native acceptance testing pending

## Context

The shell previously discarded capture mode when building controller settings.
Game/Auto therefore captured a monitor. DisplayCapture attempted Desktop
Duplication for every source, and WGC startup validated even HWND sources through
display enumeration, rejecting its otherwise implemented CreateForWindow path.

## Decision

Carry `game:` and `window:` prefixes through the controller boundary. Resolve
these modes only to the selected application window using WGC, never to a monitor.
Game mode is explicitly labeled window-based until experimental hooks exist.
Legacy Game/Auto monitor-following is not retained as a silent privacy fallback:
the user must select a running application. Monitor Auto remains a display mode.

Enumerated window IDs carry a PID and HWND. Validate both before starting capture
and again when creating the capture item. List non-foreground, minimized, and
cloaked application windows, without promising minimized-frame availability.
Exclude Silk's own windows. Legacy HWND-only IDs remain supported. This is not
durable process identity across PID reuse or application restarts; users may need
to refresh and reselect after a game restarts. A future hook backend must bind
process creation time as well.

Enumerate window dimensions for engine startup, then publish actual WGC content
dimensions before frames. Native output follows the selected source. For isolated
window/game modes, a fixed output resolution is a bounding box: fit the source
aspect ratio within it instead of stretching or adding monitor-shaped bars.
Round output dimensions down to even pixels for NV12. This can introduce a
sub-two-pixel aspect approximation; it does not detect or remove black bars
rendered inside a game's own surface, nor guarantee client-area-only capture.
Resize continues through the existing encoder/replay epoch reset path.

Capture-source select handlers snapshot the selected string before submitting a
React state updater, rather than reading a mutable DOM selection during a later
update. The browser regression gate covers second-monitor selection, window/game
selection, successful persistence, and retry after failure without reopening.
This hardens the reported settings path; the exact friend's native repro remains
to be confirmed.

## Consequences

There is no new hook, injection helper, dependency, or release in this change.
WGC can still fail or stall for minimized or exclusive-fullscreen games. Valorant
and anti-cheat compatibility are unverified. Native startup failures can still be
asynchronous in the existing WGC session worker. Do not claim this implements the
experimental hook milestone or resolves those compatibility limitations.
