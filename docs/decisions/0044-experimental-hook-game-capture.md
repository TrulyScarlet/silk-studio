# ADR 0044: Opt-in experimental hook-based game capture

- Date: 2026-11-11
- Status: Accepted scope amendment; implementation and compatibility unverified

## Context

The product owner approved revising the no-hooks MVP scope to pursue capture of
games using exclusive fullscreen. Current monitor-based behavior can expose the
desktop on Alt-Tab and must not be presented as window-isolated game capture.
Window-targeted Windows Graphics Capture remains useful but is not assumed to
work for every exclusive-fullscreen game.

## Decision

Allow an original, opt-in experimental user-mode graphics-hook capture backend
and the minimum targeted process injection required for that backend. This is a
separate milestone, not permission to enable hooks in production immediately.
The exception supersedes the former blanket MVP prohibition only within this
scope. All OBS reference-use and dependency-license restrictions remain intact.

Before implementation, document the first graphics API, supported process
architectures, target-selection rules, IPC and shared-surface ownership, helper
lifecycle, and failure recovery in a technical design. Do not assume D3D11,
D3D12, Vulkan, or other API support is interchangeable.

### Boundaries

- User explicitly enables experimental capture and selects or approves the game
  target. Verify process identity and lifetime; never target unrelated processes.
- No anti-cheat bypass, evasion, stealth, kernel drivers, privilege escalation,
  protected-process injection, or security-product exclusions.
- If anti-cheat or Windows blocks capture, stop and explain the unsupported
  condition. Do not promise account safety or universal compatibility.
- No silent fallback from game/window capture to desktop/monitor capture.
- Keep capture behind the capture interface, separate from encoding, replay
  buffering, muxing, and UI. Use bounded transport and buffers; rendering threads
  must not wait for encoding, disk, or UI work.
- Define safe behavior for Alt-Tab, minimize, resize, device loss, game exit,
  helper failure, and target recreation. Never substitute another app's frames.
- Match output to the captured game surface; do not assume its aspect ratio
  matches the monitor. Capture cannot automatically remove bars rendered by the
  game itself without an additional explicit crop policy.

## Delivery gates

1. Fix the settings-save regression and validate source switching separately.
2. Implement honest window-targeted capture/detection where supported; distinguish
   monitor, window, and experimental hook modes in configuration and UI.
3. Review the technical design and dependency licenses before hook implementation.
4. Prototype one graphics API with a controlled local test application first.
5. Add deterministic tests for target lifecycle, bounded transport, frame sizing,
   timestamps, failure recovery, and prevention of desktop fallback.
6. Record actual hardware, game, Windows, graphics API, and anti-cheat test results;
   test fullscreen, Alt-Tab, minimize, resize, crashes, and sustained capture.
7. Review signing, installer, updater, and antivirus implications before shipping
   any helper binary. Signing does not guarantee antivirus acceptance.

## Consequences and limitations

This adds substantial crash, compatibility, security, and maintenance risk.
Valorant and every other game remain unverified until tested; security controls
may make some games unsupported. No new dependency, hook code, injection code,
or production capability is introduced by this scope amendment.
