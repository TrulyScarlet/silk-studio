# ADR 0093: Correlate native capture timestamps, not delivery times

Date: 2026-11-17

## Decision

DXGI display and guarded-window capture stamp frames against one process-wide
monotonic epoch. The capture interface exposes that epoch; the recorder converts
those timestamps directly to the recording origin. Queue latency, encoder setup,
first-focus waiting, format changes and recovery do not redefine the clock.

WASAPI exposes each frame's host capture instant by correlating its documented
100ns QPC timestamp with a current QPC/Instant pair. After audio synchronization
and format conversion, native audio output timestamps use that capture instant
relative to the same recording origin. Device downtime is retained rather than
collapsed into contiguous audio timestamps. Pre-session timestamps clamp to zero.

Legacy and synthetic backends without host clock correlation keep their existing
timestamp behavior. The guarantee applies to product DXGI/WASAPI routing, not
experimental hooks or WGC. No hardware timing precision is claimed from unit tests.

Automatic game attribution now flows from the attached capture object's source
identity through the recorder and controller. Waiting/recovery exposes no attached
identity. Shell lookup formats only that identity; it does not select another game.
Retained frozen frames keep their original session identity.

## Release exception

The user explicitly authorized continuing the temporary Windows publisher-signing
exception on 2026-11-17. Extend its bounded allowlist to 0.1.5 only. Updater artifact
signatures, official endpoint restrictions and provenance checks remain required.

## Validation and limitations

Deterministic correlated-clock tests cover joint video/audio timestamps at startup,
30-second waiting and 150-second recovery, independent of packet delivery time.
Run workspace formatting, lint and tests and settings browser regression tests.
Live decoded audiovisual-stimulus acceptance remains required before claiming
measured synchronization precision or publishing the release as validated.

This supersedes ADR 0092's unverified claims of exact millisecond alignment and
eliminated long-term drift. QPC correlation does not eliminate device/driver latency
or prove end-to-end audiovisual synchronization.
