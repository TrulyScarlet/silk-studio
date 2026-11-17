# D3D12 queue association contract — metadata tests only

ADR 0062 and `crates/capture-hook-prototype/src/d3d12_queue_registry.rs` define
the bounded contract for a future owned CreateSwapChainForHwnd observer.
No native hooks, COM ownership or capture routing changed in this slice.

Eight deterministic tests passed:
- explicit selection ignores unrelated associations;
- duplicate observation/selection is idempotent;
- zero identities consume no capacity;
- conflicts preserve existing associations, while shared queue/device is allowed;
- eight-slot pressure never evicts, and unrelated retirement restores capacity;
- retired tokens cannot select/retire reused addresses;
- selected retirement/close is terminal;
- generation overflow never wraps.

Validation on 2026-11-12:
- `cargo test -p capture-hook-prototype --all-features --lib`: 41 passed;
- all-feature/all-target prototype Clippy with warnings denied: passed;
- `scripts/check.ps1`: formatting, workspace Clippy/tests, frontend typecheck,
  ESLint and Vite build passed;
- settings-save regression: passed;
- strict dependency verification: passed, 545 registry packages/11 direct frontend
  dependencies; no new dependency/license.

Registry tokens are local to their owner, not credentials. Native validation must
still check DIRECT queue type, canonical COM identity, device relationship, exact
owned HWND/process and lifetime, and retain bounded references before using these
IDs. Native interception/bootstrap and GPU acceptance remain unimplemented and
untested for this contract. No game or security settings were touched.
