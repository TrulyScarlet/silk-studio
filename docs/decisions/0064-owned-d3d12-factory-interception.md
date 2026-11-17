# ADR 0064: Owned D3D12 factory interception and discovered queue capture

- Date: 2026-11-12
- Status: Accepted for default-off explicitly opted-in owned child only

Add a distinct d3d12-factory-hook-fixture feature/source/CLI route. Install an
original CreateSwapChainForHwnd detour in the exact owned executable with explicit
owned/Present/factory opt-ins, before its renderer creates either swap chain.
The renderer calls DXGI directly, not the cooperative association wrapper.

Always call the original first and preserve its HRESULT/output unchanged. For
successful creations only in the exact current-process fixture window class,
try-lock an eight-slot pending-offer array and retain typed queue/chain references.
No capture/GPU submission, file/pipe I/O or wait on controller work in this callback.
QI/refcount work is permitted at creation, not placed in Present. Overflow,
contention or malformed successful observation makes capture observation terminal
but does not fail the renderer's successful creation. Unknown windows are ignored.

Only this module can construct a successful-observation proof. Controller claims
the exact canonical chain once and gives proof to ADR 0063's native owner for
DIRECT/device/HWND validation and bounded retained lifetime. It does not supply a
queue at claim time. There are at most eight pending offers plus eight adopted
associations, not an unbounded registry. Claimed offers leave pending storage.
Native owner retains Window guards; unclaimed COM offers are cleared on close.
No cooperative fallback if an expected creation was not observed or was lost.

Pin original module; publish trampoline before enable; one installation attempt
per process. Cancel observation and retire resources, but do not disable/remove
or unload executable forwarding until process exit. MinHook's transient thread
management is not a zero-suspension guarantee. Independent factory interception
does not mean independent bootstrap: fixture still installs using a known factory
and explicitly selects its chain before enabling its existing Present capture.

Use a distinct recorder source and native counters to avoid relabeling older
wrapper/Present-only evidence. Test original failed/successful creation, two queues,
single-use exact claims, bounds/contention, cancellation forwarding, selected
pixels, replay/decode and window-state/short-soak routes on WARP/hardware.
Creation variants other than CreateSwapChainForHwnd, pre-existing chains,
ResizeBuffers1 queue replacement, independent loading/bootstrap, device/resize
rearm, exclusive fullscreen and protected games remain outside this milestone.
No new package/license: existing optional MinHook 0.9.0 and windows 0.62.2 only.

Reference: [Microsoft CreateSwapChainForHwnd](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforhwnd).
