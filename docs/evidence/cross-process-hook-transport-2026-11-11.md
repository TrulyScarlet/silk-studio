# Cooperative cross-process hook transport: native evidence

Date: 2026-11-11. Design: ADR 0047. Supersedes the transport-not-yet-tested
limitation of the initial in-process prototype evidence, not its game limitations.

## Environment and command

- x64 Windows 10.0.19045/build 19045, unchanged from the first prototype run.
- NVIDIA GeForce RTX 2080 Ti, Windows-reported driver 32.0.16.1742.
- WARP/Microsoft Basic Render Driver tested separately.
- Debug builds; 640x480, single-sampled BGRA8; three shared export surfaces.

```powershell
.\scripts\test-game-capture-prototype.ps1 -Native -Hardware
```

The script runs deterministic tests and all-feature Clippy, then the original
in-process fixture and the new cross-process fixture on WARP and hardware.
Native runs are opt-in; no game is enumerated, opened, or injected into.

## Observed native behavior

- Both software and NVIDIA runs passed with distinct producer and consumer
  processes, communicating only through private inherited standard pipes.
- The producer's actual process creation time, HWND ownership, adapter LUID,
  QPC frequency, protocol, and shared texture descriptors were checked before use.
  Altered PID, creation time, adapter, clock, and dimensions were rejected.
- Three unnamed NT shared texture handles were duplicated from the exact Child
  process handle into the consumer and opened on its own matching D3D11 device.
- Red, blue, green, and yellow pixels were read back from shared GPU resources
  in the consumer, not sent as CPU image payloads through the control pipe.
- With all three slots retained, 32 additional Presents completed and their
  export work was dropped. The retained red frame stayed red. Releasing/ACKing
  the blue slot allowed exactly that slot to carry a new yellow frame.
- Stale ACKs terminated the producer session. Even an exact premature ACK could
  not overwrite a texture whose keyed mutex was still owned by the consumer.
- A valid-looking frame header for a never-published slot encountered a real
  WAIT_TIMEOUT (0x102), which was rejected rather than treated as success; the
  consumer then remained closed until a new session.
- A retained imported red surface stayed readable after graceful producer exit
  and after killing only the launched producer Child. Duplicate handles and COM
  resources were managed by RAII.
- EOF, malformed packets, truncated packets, an idle stalled peer, and a peer
  stalled after a partial packet were rejected within bounded receive deadlines.
  Cleanup killed/waited for only the owned Child and joined the bounded reader.

## Deterministic coverage

Ten prototype unit tests passed, including version/type/reserved-byte/length and
field validation, all-message round trips, every truncated packet length,
deterministic malformed inputs, session/epoch/process identity, exact slot ACKs,
replay rejection, bounded strong/weak leases, sequence/QPC ordering, and exact
keyed-mutex success classification.

Full validation also passed: `scripts/check.ps1` (formatting, workspace
Clippy/tests, frontend typecheck/lint/build), strict dependency verification,
and the existing isolated-browser settings source-save regression. Unrelated
hardware tests stayed ignored; only the controlled hook/transport native gates
were explicitly enabled.

## Limits

- No injected helper, arbitrary game attachment, production recorder adapter,
  cross-process resize/rearm, device-loss recovery, or helper signing/packaging.
- No exclusive-fullscreen, Alt-Tab, minimize, Valorant/anti-cheat, AMD, Intel,
  multi-GPU, D3D12, Vulkan, OpenGL, HDR, or antivirus acceptance claims.
- Peer binding relies on private pipes and the process handle of the exact
  cooperative executable launched by this harness, not a protocol suitable for
  an untrusted or injected peer. Same-user malicious-process resistance is not
  established.
- Rendering and hook lifecycle are still single-threaded and quiescent in the
  original fixture. Its control/export orchestration is synchronous outside the
  Present callback; production requires independent bounded workers.
- Full-ring checks establish functional backpressure, not a latency/throughput,
  memory-soak, or GPU-driver nonblocking guarantee. CPU readback is test-only.
- Existing source/aspect/settings changes remain in place; no installer or
  update was published and no user clips/settings were modified.
