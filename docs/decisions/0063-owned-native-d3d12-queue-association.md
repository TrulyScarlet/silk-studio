# ADR 0063: Native ownership at the owned swap-chain creation boundary

- Date: 2026-11-12
- Status: Accepted for default-off owned fixture only

Implement ADR 0062 using a cooperative CreateSwapChainForHwnd wrapper, not a detour.
The wrapper performs successful creation itself, validates DIRECT type, compares
canonical IUnknown device identities from queue and chain, and checks GetHwnd and
current-process window liveness. Accept only the fixture's internal Window guard;
no external PID/HWND or arbitrary existing chain/queue association API.

Use eight fixed ownership slots retaining canonical COM identities, queue/device
interfaces and an Rc<Window> guard. The guard keeps the owned HWND from being
destroyed/recycled while registered. Rc also keeps the owner thread-bound. Resolve
only an explicitly selected canonical chain; interface aliases are normalized via
IUnknown. Retire metadata before releasing native references. Selected retirement
closes the owner permanently. Full admission is rejected before native creation;
failed queue validation/creation does not consume a slot. No GPU work is submitted
by this module, and no locks, channels or disk/UI I/O are added to Present.

The presentation fixture now records its two successful creations and chooses its
GPU capture device/queue through the selected association. Cancellation retires
that selection without stopping the renderer. Other renderer/media owners still
retain resources and preserve their existing fence-drain teardown contracts.

This validates cooperative native ownership, not generic factory interception,
independent bootstrap, all DXGI creation variants, multi-node queue selection,
production rearm, exclusive fullscreen or protected-game compatibility.
Native tests must cover WARP/hardware and the existing presentation/replay routes.
No dependency/version/license changes; existing windows 0.62.2 bindings only.

Reference: [Microsoft CreateSwapChainForHwnd](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforhwnd).
