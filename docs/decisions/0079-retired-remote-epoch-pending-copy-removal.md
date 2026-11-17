# ADR 0079: Pending producer-copy removal after exact remote retirement

- Date: 2026-11-14
- Status: Implementation in progress; not unexpected-loss acceptance

The current new probe is **hardware-only**. WARP attempts hit intermittent native
access violations during removed-graph teardown, even after an initial remote
ACK. WARP is now rejected before graphics startup or RemoveDevice. Earlier WARP
successes do not establish support for this route. Local WARP probes in ADR 0078
remain separate. Before-first-ACK hardware requests are also rejected normally,
without removal. The supported probe requires a primed remote epoch.

Extend the owned integrated worker's controlled device-replacement route with
`replace-remove-pending`. Stop Present media admission without closing the private
producer ring, pause only the owned renderer through a bounded scalar action, and
cross the parent's existing delivered-frame/independent GPU retirement barrier.
Process exact old ACKs until its matching Retired message. The parent must drop
its old imported resources before returning that proof; retained local encoder
publications are independent textures and remain valid.

Only after that exact proof, permanently prohibit further old remote exports and
ACKs. Submit an actual unpublished producer copy behind an unsatisfied owned queue
wait, verify the producer fence is below its sequence, then immediately invoke
RemoveDevice. Reject UINT64_MAX as completion. Release confirmed-invalid producer
resources without calling Reset on removed allocators/lists. Keep the selected
HWND guards, release every old worker/renderer graph, create the replacement via
the application's real factory API, and require fresh exact parent approval.

No renderer COM handoff, hook removal/reinstallation, guessed queue, TDR, security
change or arbitrary target. This is a **cooperatively retired remote epoch with
an in-flight unpublished producer copy**, not unexpected removal racing parent
copies, independent DLL-worker recovery or automatic game graph discovery. Those
remain separate gates and must not inherit this result. Wrong/missing retirement,
wrong approval and stop must remain fail-closed and bounded. Tests must cover
retained old local pixels and saved/decode replay across replacement epochs.

No new dependency. Existing windows 0.62.2 and internal transport/retirement
interfaces only. Preserve the normal desktop graph's exclusion of prototype,
helper and MinHook until the independent production integration gate.

Retirement closes original exported handles before producer COM resources are
released. Resource fields retain both devices/queues until their lists, imported
wrappers and shared objects are destroyed. Neither this ordering nor a prior
queue-tail fence fixed the observed WARP native fault; they must not be described
as its root-cause fix. An explicit opt-in owned-fixture exception diagnostic uses
standard user-mode Windows exception/symbol APIs; it never installs in games,
suppresses exceptions, enables a kernel driver or changes OS crash settings.
