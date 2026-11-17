# ADR 0057: Owned cross-process D3D12 shared-fence transport

- Date: 2026-11-12
- Status: Accepted for default-off explicitly launched fixture children only

Extend ADR 0056 to one exact D3D12 fixture child launched by the controller. No
arbitrary PID, injection, external HWND, elevated/protected process or game target.
Use private inherited standard pipes with fixed 128-byte packets, bounded command/
output queues and independent pipe threads. Rendering must never write/read pipes.

Offer an exact PID/process-creation-time/HWND session and five producer handle
values: three textures, produced fence, consumed fence. Parent validates retained
Child handle, HWND PID, process lifetime, QPC frequency, dimensions, handle uniqueness
and adapter LUID before duplicating/opening handles. Parent sends exact selection
approval before capture starts. The dedicated greeting uses a distinct SK12 magic;
existing frame/ACK/control packets retain their validated fixed layout. No names,
global objects, network endpoint or arbitrary attach API are introduced.

Producer emits a frame only after its produced fence completes and marks its slot
Published. Only an exact current-session/slot/sequence ACK marks Consuming; reuse
also requires actual consumed-fence completion. ACK alone is insufficient. Remote
consumer checks metadata/liveness/completion, waits/copies only to a reserved local
GPU output, signals consumed/Flushes, and queues the ACK. Dropped local outputs
still signal consumed before ACK. Never wait on a peer-promised future fence value.
The consumer is single-owner and acknowledgements are ordered by sequence.

Input failure, malformed/truncated packets, identity mismatch, stale ACK, process
exit, queue failure or deadlines are terminal: discard pending media/retire handles,
never select another application or fall back to monitor capture. Completed local
output leases may outlive the producer. Process-death observation is not crash-atomic
GPU publication; do not claim safe pixels if death interrupts pending GPU work.

Reuse the bounded existing ring/local pool contracts. The first cross-process slice
may retain unused local-import allocations in the child; count them explicitly,
not as zero-copy/minimal memory. Resize/rearm is terminal pending separate epoch
design; no unbounded pool replacement. Cleanup may kill only the exact owned child.
No new package or license. Run WARP/hardware lifecycle and decoded recorder tests
before claiming this fixture route works; no protected-game compatibility follows.
