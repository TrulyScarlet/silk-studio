# Endurance test suite

The `s8-endurance` package is a short, deterministic harness for the
automatable Segment S8 slice. It uses scripted capture, injectable encoders,
and filesystem-safe mock muxers; it does not claim an eight-hour run or verify
hardware behavior.

Run it with:

```powershell
cargo test -p s8-endurance -- --test-threads 1
```

Covered scenarios:

- Repeated controller start/stop and live settings changes
- 100 sequential clip saves
- Gated concurrent save and library scan while capture continues
- Display loss, successful recovery, and bounded recovery failure
- Audio-device loss and recovery while video remains active
- Sleep/resume epoch separation
- Encoder fallback after an injected preferred-backend failure
- Stale `.part` cleanup on startup
- Bounded engine events, save work, and replay-buffer packets

The tests assert bounded packet/queue behavior, completion counts, staged-file
cleanup, recovery states, and save progress. Native resource recreation,
process-kill crash timing, eight-hour resource trends, and the hardware matrix
remain manual or platform-specific work.
