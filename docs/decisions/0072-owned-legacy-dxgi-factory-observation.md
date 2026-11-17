# ADR 0072: Observe owned legacy DXGI swap-chain creation

- Date: 2026-11-13
- Status: Accepted for default-off owned fixtures only

The independent D3D12 worker observed only CreateSwapChainForHwnd. Loading that
worker into a process would not establish queue ownership for applications using
the older CreateSwapChain API. Add a second resident detour for that API, sharing
the existing bounded observation and exact owned-window/process admission rules.

Forward the original method and preserve its HRESULT/output before observing.
Only successful creations are inspected. Query the supplied object as a D3D12
command queue, retain bounded COM references and claim only the selected owned
HWND. Do no GPU work or blocking controller operations in either detour. Pin both
original code modules; keep both forwarding trampolines resident until process
exit. Partial installation closes observation and never unhooks live forwarding.

The explicit --legacy-factory fixture launch chooses legacy DXGI creation for
both owned rendering windows. The bootstrap worker still receives metadata only,
not cooperative queue/chain pointers. Existing factory/Present flags remain
mandatory. Recorder source validation rejects legacy mode without bootstrap or
combined with resize/replacement/fault controls not yet tested for that mode.

Native WARP/hardware lifecycle tests must verify two observations, one selected
claim, bounded pressure, fresh window-action pixels, stop and continued forwarding.
Saved/decode replay tests exercise software and hardware-only encoding separately.
This is an attachment prerequisite, not remote loading, pre-existing-chain
discovery, arbitrary-process support, exclusive-fullscreen acceptance or a shipping
backend. CreateSwapChainForComposition and CoreWindow are not implemented.

No dependency/version/license changes: existing windows 0.62.2 and MinHook 0.9.0
remain development-only under their recorded licenses.

References:
- [IDXGIFactory::CreateSwapChain](https://learn.microsoft.com/en-us/windows/win32/api/dxgi/nf-dxgi-idxgifactory-createswapchain): D3D12 supplies a direct queue, not a device.
- [IDXGIFactory2::CreateSwapChainForHwnd](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforhwnd).

No OBS implementation used.
