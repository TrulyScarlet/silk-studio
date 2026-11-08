# ADR 0028: Preserve recording time across display capture recovery

## Context

A Valorant replay saved after a DXGI display loss contained 15.899 seconds of
video but only 5.269 seconds on each of three audio tracks. Capture startup resets
the source video timestamp origin; independent WASAPI workers continue running.
The replay buffer consequently selected only the small overlap between the reset
video timeline and the continuing audio timeline.

The first audio track was also silent for a separate routing reason: Silk selected
GoXLR System, while Windows reported Valorant's active audio session on GoXLR Game
and an inactive session on System. The user chose to change Silk track one to Game.

## Decision

The engine owns a recording-lifetime video timeline. Immediately after a capture
restart succeeds, it records elapsed monotonic time relative to initial capture
startup. The first recovered frame establishes a new source offset; subsequent
frames retain their source cadence. Offsets are replaced on every recovery, never
added cumulatively, and cannot move time behind the last submitted frame.

This preserves recovery downtime and excludes encoder reconfiguration and queue
delivery latency from the restart measurement. Audio workers continue on their
existing timeline. Codec/buffer epochs still clear incompatible buffered packets.
Format changes and encoder-only reconfiguration do not reset the video timeline.

No dependency or license changes. The read-only `audio-wasapi` example
`inspect_endpoints --list` reports endpoint IDs and audio-session process IDs using
Microsoft's public Core Audio APIs; without `--list` it also measures GoXLR loopback
peaks without writing audio to disk.

## Validation and limits

Deterministic tests cover repeated timestamp resets, nonzero source origins,
different time bases, recovery downtime, backward-time protection, engine recovery
wiring, and complete replay selection across three continuing audio tracks.

An opt-in native test in `apps/desktop/src-tauri/tests/native_recovery.rs` uses the
real DXGI, hardware H.264, WASAPI, AAC, and MP4 paths. It injects two SourceLost
events while all configured audio workers continue. On this Windows machine the
recovered video ended at 5.656 seconds and the three audio tracks ended at
5.546/5.568/5.568 seconds; every track began within 7 ms of the video start.
This tests real native teardown/restart, not a reproduction of an actual Valorant
fullscreen transition. It does not claim sample-accurate A/V synchronization:
startup timing between devices and first-frame arrival still needs a common
hardware-clock calibration for that guarantee.

Formatting, Clippy, 108 ordinary tests, the opt-in native recovery test, frontend
production build, NSIS build, and local upgrade passed. The installed build's first
track was enabled and switched to the active GoXLR Game endpoint; microphone and
Music routing were preserved. A quiet one-second 880 Hz tone sent specifically to
Game was present in decoded track-one AAC (peak 0.047607, signal from 7.440042 to
8.439958 seconds). The installed app saved 27.599 seconds of video with 27.477333
seconds on each audio track. Evidence clip:
`C:\Users\Home\Videos\Silk\Brave\Clip_2026-11-08_00-02-46_auto.mp4`.
This confirms the Game endpoint path; actual gameplay after the update still
requires the user's next clip.

The fix cannot reconstruct audio omitted from existing clips. Capture recovery
still discards the previous buffer epoch. Opt-in tests write clips only under
`target/dxgi-evidence`; normal tests do not access hardware.

## References

- https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessionmanager2-getsessionenumerator
- https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-getprocessid
