# ADR 0068: Owned healthy D3D12 device-graph teardown/recreation

- Date: 2026-11-12
- Status: Accepted for the default-off owned fixture only
- Prerequisites: ADRs 0066 and 0067

## Scope

Implement explicit healthy teardown/recreation inside the exact owned bootstrap
child, keeping PID, creation time, selected HWND, adapter LUID and QPC frequency.
`d3d12-device-replace-fixture` enables `--replace-device` over `--bootstrap --rearm`
and the distinct source `owned-d3d12-device-recreation`. Production routing, remote
loading, arbitrary pre-existing chains and protected games are not enabled.

This is an operation over our known device graph, not independently observed
runtime destruction and not a driver reset/device-removal recovery claim. Follow
ADR 0067: release all old producer COM references before calling D3D12CreateDevice;
do not retain an old device or use recycled pointers as device-lifetime proof.

## Ownership transition

1. Validated kind-6 ReplaceDevice carries exact current session and bounded even
   dimensions. One in-flight transition plus one queued local request maximum.
   Existing Resize remains distinct and may be interleaved on this route.
2. Cancel Present capture admission; emit Retire(next exact session). Parent
   advances delivery epoch and crosses next_event's delivered-frame resolver gap,
   retires/drains old remote media, and returns exact Retired(next session).
   Old ACKs during retirement must match published slot/sequence snapshots exactly.
3. Drain/release old bridge sources/media. Drop worker chain/queue/device resources
   and clear pending factory references while leaving the observer resident.
   Initial discovery locals must not remain live across the transport loop.
4. The owner-thread mailbox drains its healthy render-only fixture and clones only
   the two Rc<Window> guards. Drop the entire old renderer, including its one-live-
   fixture reservation, before creating a new device/queue/chains on those HWNDs.
   Keep guards through worker join even if construction fails. Uncertain drains
   remain terminal and retain uncertain objects; do not pretend fences completed.
5. Worker claims the newly intercepted exact HWND, validates DIRECT type, canonical
   new queue/chain device association, actual HWND/format/dimensions and healthy
   GetDeviceRemovedReason. Fresh hello must preserve adapter/QPC/target/generation
   and contain exactly next epoch and requested dimensions.
6. Stage fresh bounded media only while inactive, empty and unfailed. The new chain
   must use the installed Present entry and exact selected HWND. Retarget the
   resident forwarder, without reinstall/unhook/unload. Parent imports, queues new
   context with FormatChanged, then approves exact session. Only then admit media.

Six-local-surface retirement budgets remain shared across epochs, on both producer
and consumer sides. Encoder-held resolved leases may survive retirement. Selected
renderer progress is accumulated monotonically rather than resetting with renderer
statistics. QPC origin/cadence remain continuous. Factory offers and control/media
channels stay bounded; no disk/UI work or blocking control I/O is added to Present.

## Failure and validation contract

Wrong/stale/missing retirement, approval, ACK or session is terminal. Injected
after-release/before-hello failures and malformed replacement hello must not
deliver new frames or silently retry/fall back. Explicit stop/start launches a
new child/session; it is not live device-loss recovery. Cancellation at the parent
delivery barrier must remain bounded. A replacement request without the distinct
opt-in is rejected even if live resize is enabled.

Tests cover native recreated pixels/QPC and retained old surfaces; repeated epochs
and timed soak; resolver-gap safety and retained-context budget; mixed same-size
resize/recreation; fault cleanup and explicit restart; actual saved/decode replay
epochs on WARP/software and hardware-only encoding. Timed soaks are bounded test
runs, not hours-long endurance, leak-free proof, performance benchmarks or game/
exclusive-fullscreen acceptance.

No dependency/version/license changes. Existing windows 0.62.2 (MIT OR Apache-2.0),
minhook 0.9.0 (existing MIT/BSD-2-Clause notices) and internal interfaces are reused.

References:
- [D3D12CreateDevice singleton contract](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-d3d12createdevice)
- [CreateSwapChainForHwnd: one flip-model chain per HWND](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforhwnd)
