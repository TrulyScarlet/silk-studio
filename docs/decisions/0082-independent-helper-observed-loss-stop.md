# ADR 0082: Observed-loss stop in the independent owned helper

- Date: 2026-11-14
- Status: In progress; no acceptance claim

Targeted implementation checkpoint: all-feature Clippy and 73 safe library tests
pass. The independent hardware native test passes normal helper completion after
exact parent retirement, retained original red pixels, and twelve fresh green
frames through production non-injected WGC on the same owned HWND. Broad new
regression and invalid-proof/stop/invalid-mode coverage remain pending. This
checkpoint does not establish resumed helper capture or third-party acceptance.

The subsequent tracked combined native gate passed with actual exit zero,
2026-11-14 14:52:10–15:12:47 UTC, under
`C:\Users\Home\AppData\Local\Temp\opencode\silk-helper-observed-loss-native-2dcf8bc9a6d64b9cbea81e923297d7d2`.
Afterward, additional targeted wrong-proof and stop-at-retirement tests pass.
Wrong proof produces normal helper error status one, not graph-release approval;
stop completes normally while conservatively retaining unproven removed media.
The initial two-second cancellation expectation failed because conservative drain
is not the healthy fast-stop path. The explicit cancellation test now requires
completion within six seconds; this is a known latency limitation, not a fix to
the underlying uncertain drain. The owner remains paused and is not permitted to
reconstruct after either no-proof path. Full workspace/frontend, strict dependency
and settings-save gates also pass. Invalid-mode native coverage remains pending.

Implement a distinct hardware-only early-passive fixture mode for the normally
loaded DLL helper. The owner may remove its own device through a bounded selected-
HWND scalar fixture message, then pump windows without rendering while it waits
for a bounded helper-release notification. No COM object or renderer-private
counter crosses modules. No game, WARP, late/legacy/resize mode is authorized.

The helper independently detects its actual selected device's removed reason,
closes Present admission, sends a typed loss notice and waits for exact parent
retirement. The parent crosses its delivered-frame boundary, proves an independent
D3D11 tail and drops old imported resources. Only then may the helper retire old
exports/media, all retained old worker graphs and pending factory observations.

The helper leaves all forwarding trampolines/module code resident, closes further
factory media observation and signals only the exact owned HWND that old capture
references were released. The owner retains its HWND guards, releases its removed
graph and recreates a healthy graph through its own real renderer API. Capture
stops normally; WGC on that same owned HWND must verify fresh green rendering and
retained old local red pixels after helper completion. New helper media is not
approved or resumed by this first step.

This is independent-helper **loss-stop and application continuation**, not helper
capture recovery. Before-first-ACK/uncertain retirement remains terminal. A missing
or wrong parent proof cannot authorize graph release; the owned fixture may time
out rather than claim recovery. The notification is a fixture control, not general
authenticated application IPC. No hook uninstall/reinstall/unload, security change,
arbitrary PID, process privilege change or capture-exclusion bypass. No dependency.
