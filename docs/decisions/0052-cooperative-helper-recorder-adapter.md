# ADR 0052: Cooperative helper recorder adapter and epoch delivery

- Date: 2026-11-11
- Status: Accepted for the default-off owned fixture

Extend ControlledCapture with an explicit cooperative-DLL producer mode. Keep the
command-driven fixture route as a comparison. Launch only the allowlisted owned
fixture path; approve the offer against its retained process handle before loading
the helper. Reuse ConsumerRing, LocalGpuBridge, QPC conversion, VideoCapture and
the existing recorder/encoder/replay/muxer interfaces. Do not request frames from
the independently paced helper renderer. Shape delivery to the requested FPS,
but consume/release/ACK skipped frames so backpressure remains bounded.

Use a one-entry nonblocking rearm mailbox. Rearm closes the old delivery epoch,
retires its resolver, sends Stop, drains/discards old frame headers until Bye,
and requests explicit owned resize/device recreation in the next epoch. Validate
the new offer against the SAME target identity/generation and expected dimensions.
Publish FormatChanged with the replacement GPU bridge before delivering new frames.
The capture object switches gpu_frame_context at that event; RecorderEngine's
existing format path drains/reconfigures the encoder and resets the replay epoch.
Never mix previous-source packets into the new clip. Keep QPC origin across rearm
to preserve elapsed time, and test conversion plus deterministic epoch rejection.

Keep queues and pools bounded: three capture events, three imported transport
slots, three local GPU slots, existing six-local-surface retained-context budget,
and the helper's existing three-attachment lifetime quota. Reliable format events
may retry on a full capture queue while the owned renderer continues independently;
no pipe, encoding, disk or UI work is placed in Present. Refuse budget exhaustion
rather than allocate unbounded replacement pools.

Retain a duplicate of the exact child process handle for liveness checks in worker
and next_event. Recheck after receiving/copying and before delivering. On observed
exit or transport failure close delivery, invalidate the GPU resolver, and discard
queued events. No reattachment/desktop fallback. This does not close the race of a
process dying immediately AFTER a liveness check or guarantee completion of GPU
driver calls. ADR 0051's interrupted-WARP-work limitation remains; do not claim
crash-atomic pixels. Previously encoded valid replay packets are a separate policy
from pending capture frames; target death does not erase already saved clips.

Native acceptance: actual DLL -> shared GPU transport -> bounded local bridge ->
Media Foundation -> replay -> MP4; approved FFmpeg decode; source-aspect dimensions,
explicit resize/new-device epochs, stop/restart, dead-source pending-frame rejection,
and hardware GPU inputs with zero reported CPU readbacks. No real-game injection,
fullscreen/anti-cheat compatibility, physical GPU removal or production shipment.
