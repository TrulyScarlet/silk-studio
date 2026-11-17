# ADR 0062: Bounded owned D3D12 queue association contract

- Date: 2026-11-12
- Status: Accepted as metadata-only preparation; native observation not implemented

The existing owned Present fixture supplies its queue. Before replacing that
boundary, define a deterministic registry for successful CreateSwapChainForHwnd
observations. Microsoft's API contract states that D3D12 pDevice is a DIRECT
command queue. Do not infer association from the last ExecuteCommandLists call,
foreground window, adapter coincidence or a scan of arbitrary process memory.

Use eight fixed metadata slots, nonzero opaque canonical chain/queue/device/HWND
identities, monotonically issued tokens and explicit selection. Duplicate exact
observations are idempotent. Conflicting chain/HWND or queue/device associations
are rejected without replacement; full capacity never evicts an existing entry.
Unrelated retirement frees a slot, but selected retirement closes the entire
registry. No automatic reselection or rearm. Reused addresses receive new tokens;
generation exhaustion cannot wrap. This registry has no allocation, OS calls,
COM ownership, synchronization, injection or production source registration.

A future native owner must validate successful creation, canonical COM identity,
DIRECT type/device relationship, exact owned process/window and lifetime, retain
bounded COM references, retire before release, and use nonblocking callback access.
Metadata tests cannot establish native observation or GPU behavior. Native factory
interception, independent bootstrap and production integration remain separate work.
No new dependency/license; existing owned-only restrictions remain unchanged.

Reference: [Microsoft CreateSwapChainForHwnd](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforhwnd).
