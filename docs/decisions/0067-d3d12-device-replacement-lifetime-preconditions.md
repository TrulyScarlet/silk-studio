# ADR 0067: D3D12 device replacement lifetime preconditions

- Date: 2026-11-12
- Status: Accepted constraint; capture device replacement not implemented

Implementation update: ADR 0068 subsequently adds controlled healthy teardown/
recreation under these constraints. The implementation-status statements below
describe this ADR's initial precondition-only milestone.

The initial healthy-device replacement plan retained an old device interface to
compare canonical COM identities while creating its successor on the same adapter.
That plan is invalid: D3D12 devices are singletons per adapter in a process.
`D3D12CreateDevice` returns the existing healthy device while it remains alive.
A removed existing device causes creation to fail rather than returning it.

Reference: [Microsoft D3D12CreateDevice remarks](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-d3d12createdevice).

Do not use a retained old-device identity comparison as replacement proof. A
controlled same-adapter teardown/recreation must release the entire old object
graph first: worker interfaces, pending factory offers, capture media, renderer
queues/chains/backbuffers/lists/fences and incidental device interfaces. Keep only
owner-thread HWND guards and scalar session/adapter metadata. Parent GPU consumers
must retire/drain through the existing recorder delivery barrier. Failure or
uncertain GPU completion is terminal, not permission to release in-use resources.

After release, a recycled COM address is legal; neither pointer inequality nor
pointer equality proves a device lifetime transition. Evidence must describe
observed release/recreate calls, exact HWND/adapter/queue associations, bounded
epochs, fresh pixels and recorder results without claiming independently verified
runtime device destruction. Actual device removal remains a distinct milestone.

The default-off `d3d12-device-replace-fixture` feature currently enables only
lifetime-precondition tests. SK12 kind 6 remains reserved as a validated
ReplaceDevice request, but no producer accepts it as an operation. Incomplete
factory-discard and Present-retarget helpers were removed, not warning-suppressed.
No replacement source, CLI mode or production route is enabled.

Native tests cover same-adapter singleton identity while explicit devices live,
queue-retained device lifetime after dropping explicit device references, and
healthy creation on the same adapter after releasing all test-owned references.
These are precondition tests, not replacement/recovery acceptance tests.

No dependencies, versions or licenses changed. Existing windows 0.62.2
(MIT OR Apache-2.0) supplies the native APIs.
