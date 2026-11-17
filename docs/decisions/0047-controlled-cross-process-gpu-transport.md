# ADR 0047: Controlled cross-process D3D11 GPU transport

- Date: 2026-11-11
- Status: Accepted for the development-only controlled fixture

## Scope

Extend the original self-hook renderer, not an arbitrary game. A separate
development consumer launches that exact sibling fixture executable, obtains its
process handle from Child, and communicates over its private inherited stdin and
stdout pipes. Verify PID, process creation FILETIME, window ownership, protocol
version, adapter LUID, QPC frequency, and bounded texture descriptors before
accepting shared surfaces. No named objects, network listener, injector,
administrator permissions, security-product exceptions, or production opt-in UI.
Private pipe possession plus the launched process handle binds this cooperative
peer; this is not an authentication design for an injected helper or hostile child.

## Transport

Use fixed 128-byte little-endian messages with magic/version/type and zero
reserved bytes. No Rust struct memory is transmitted. Limit controls, dimensions,
slot IDs, QPC values, message types, and sequence numbers. Malformed/short input
fails closed. The parent reader uses a bounded channel and receive deadline;
cleanup kills and waits for only its own Child, then joins the pipe reader.

The existing Present callback still publishes only to its bounded in-process
queue. A separate export stage preallocates three BGRA8 NT-handle shared textures
with keyed mutexes. GPU copies and Flush are outside Present. Duplicate the
unnamed, non-inheritable NT handles from the already-open Child process into the
consumer; never treat a numeric remote handle as locally usable. Open on the
same adapter, checking the actual resource descriptor before readback.

Each exported slot is reserved until a matching session/epoch/slot/sequence ACK.
Key 0 belongs to the producer, key 1 to the consumer. AcquireSync is inspected as
an exact HRESULT: WAIT_TIMEOUT/WAIT_ABANDONED are positive values and must NOT be
mistaken for success by the generated Result wrapper. Release ownership before
acknowledging consumption. An old, duplicate, or mismatched ACK closes transport.
Full slots drop export work rather than waiting for the consumer. No rendering
thread pipe I/O is introduced by the hook; the synthetic test renderer drives
the export/control stage synchronously only as test orchestration, not as a
production threading model.

## Validation and limits

Native tests must verify cross-process pixels, retained frames under full-ring
pressure, slot reuse after ACK, child shutdown with retained imported surfaces,
and bounded cleanup after crash or stalled/malformed output. Unit tests cover
wire parsing, ACK state transitions, replay, identity, and keyed-mutex statuses.

No cross-process resize/rearm is enabled yet: replacing a ring must wait for an
explicit bounded retirement protocol, not accumulate outstanding shared handles.
No game, exclusive-fullscreen, anti-cheat, or recorder integration is implied.
No new dependency package is needed; existing official Windows bindings provide
handle duplication and D3D11 sharing APIs.
