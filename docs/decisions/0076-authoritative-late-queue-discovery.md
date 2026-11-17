# ADR 0076: Authoritative queue evidence for late D3D12 attachment

- Date: 2026-11-13
- Status: Implemented and validated for conditional owned late attachment

An already-created D3D12 swap chain does not expose its original presentation
queue through GetDevice. Executed-command order, the most recently seen queue,
and matching adapter/device identity are not sufficient to infer that queue.

Use a subsequent successful **ResizeBuffers1** call as a narrower authoritative
observation: Microsoft documents its explicit per-buffer presentation queues.
This does not support a chain that never calls ResizeBuffers1 after attachment.
Do not describe this planned route as unconditional late game discovery.

The first supported layout is two through sixteen buffers on a single-node
device, creation node mask one for every buffer, with the same retained canonical
DIRECT queue and matching canonical device for every buffer. Reject distinct
same-device queues, non-DIRECT queues, other device identities, multi-node layouts,
missing arrays and preserved-count zero. In particular, BufferCount zero does not
give the observer a safe array length or proof of a supplied current queue.

The metadata validator has deterministic tests, but does not inspect or retain COM
objects, install a hook or prove an API call occurred. The native layer must first
forward the original call unchanged, check success, bound the input lengths,
retain and validate actual COM identities, verify the exact pinned owned HWND,
and publish through a nonblocking bounded offer. It must keep forwarding resident
and fail closed on contention/ambiguity; never ask the renderer to disclose a COM
pointer as a substitute for observation.

Implemented integration creates/renders the owned target **before** loading the
helper, uses independently created invisible STATIC-class probe windows solely
to obtain the method entry, observes a real later ResizeBuffers1 on the same target
chain, and saves/decodes capture through the resulting queue. The target completes
at least twelve actual Presents before publishing its initial identity; calling
step twelve times is insufficient on a busy WARP device. The probe graph is
released before discovery and never supplies the target queue or media.

Both native paths verify zero new factory creations, selected pixels, increasing
QPC, retained local pixels and continued rendering after worker stop. Actual
distinct-queue resize and absent-resize cases return terminal worker status with
no admitted media. The distinct-queue negative fixture pauses rendering rather
than submitting GPU work through an unimplemented rotating-queue application graph.

This is conditional late discovery after a later explicit queue-bearing API call,
not arbitrary already-running-game support or automatic subsequent resize recovery.
Targeted native tests and software/hardware replay/decode passed. A subsequent
tracked combined run passed 61 recorder tests, fourteen attachment tests and all
previous owned native suites with actual exit zero. Modern HWND chains on the
tested WARP/hardware paths are covered; late legacy mode, arbitrary targets and
subsequent automatic resize recovery are not established.

No dependency/version/license changes. Existing windows 0.62.2 and development
MinHook only if/when the native observer is added. No protected-game targeting,
process/security changes or monitor fallback authorized by this policy.

Reference: [Microsoft ResizeBuffers1](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiswapchain3-resizebuffers1).
