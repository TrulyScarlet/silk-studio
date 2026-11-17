# ADR 0085: Reject capture-excluded targets in guarded monitor cropping

Date: 2026-11-16
Status: Accepted; supersedes ADR 0083's excluded-game suitability assumption.

## Evidence

Explicitly consented live VALORANT tests produced decoded H.264 monitor crops.
Foreground logs confirmed game PID 18912, then an owned magenta test window, then
the game again. After fixing focus-induced replay format resets, black detection
found a 6.066s black interval (7.200–13.266s) in one uninterrupted 25.033s clip.

Crucially, inspection of representative frames at 2s and 17s showed desktop/Riot
client content behind the game, not game pixels. A successful encode, nonblack
frames, correct foreground PID and a black shield interval do not prove the game
is present in Desktop Duplication. This route is unsuitable for this session.

Evidence directory:
`C:\Users\Home\AppData\Local\Temp\opencode\valorant-guarded-shield-20261116-130426`.
The diagnostic replay contains consented desktop content and is not a game-capture
success artifact. Do not publish it as one.

## Decision

Guarded cropping validates window capture affinity before starting Desktop
Duplication and during its run loop. Excluded windows return `PermissionDenied`.
There is no hook, injection, anti-cheat modification or silent desktop fallback.
The external owned-window regression covers denial in both WGC and guarded mode.

For supported, nonexcluded windows, keep the last format while unfocused. Ignore
up to two pixels of border jitter on return to avoid dropping the replay epoch.
Clear output padding and clamp the copied rectangle to the actual client extent.
Genuine larger resizes still start a new epoch. This improves continuity, not
game isolation or a guarantee of privacy on all frame transitions.

## Consequences

VALORANT recording remains unsupported by these paths in the observed session.
Other official capture integrations or vendor-approved approaches would need
separate evaluation. Do not offer capture restriction bypasses or resume hook
development implicitly. The earlier local installer predates this fail-closed fix.
