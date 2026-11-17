# Owned-process resident attachment evidence

Date: 2026-11-12; implementation began 2026-11-11. Windows x64/D3D11, WARP and
local NVIDIA GeForce RTX 2080 Ti. Design: ADR 0053. Default-off, not shipped.

## Reproduction

```powershell
.\scripts\test-game-capture-prototype.ps1 -Native -Hardware -Decode
```

The script builds the helper with `resident-helper`, runs both `--resident-test`
driver variants using `resident-hooks`, and includes the new resident recorder
tests in its existing native filters. No game/PID argument is accepted.

## What changed versus cooperative loading

The parent launches the exact passive fixture child, discovers its selected HWND,
and retains its process handle. The renderer never calls LoadLibrary, exposes a
swap-chain pointer, reads helper control messages, or cooperatively pauses.
The loader verifies PID/creation time/HWND, native AMD64, and canonical fixture/
helper paths. It uses normal OS remote DLL loading and then explicitly starts
the helper worker export. Kernel export relocation uses the actual owning module
(including forwarded exports), not an assumption about identical remote addresses.
Only data is allocated remotely, never executable payloads. Windows load/access
errors are unsupported conditions; there is no bypass or privilege change.

The worker creates a private probe swap chain, installs the resident Present
detour, discovers only the selected HWND's chain through a bounded COM-lease
queue, and allocates capture textures on the worker. Global try-lock media state
replaces renderer-owner TLS in this route. The source keeps rendering throughout;
MinHook's own installation-time thread management still occurs. "No pause" here
means no renderer-assisted/cooperative pause, NOT proof of zero suspension/stall.

## Observed native results

- Before direct attachment, the selected passive renderer had already completed
  at least 10 render iterations. Each iteration presents selected red and unrelated
  blue swap chains. The parent observes test-only HWND progress metadata; this
  metadata is not used to initialize capture or hand out pointers.
- Changed process creation time was rejected before loading. A second remote
  attachment attempt after cancellation was rejected without allocating more remote
  buffers. The loader reserves only one attempt per owned child.
- Normal DLL loading and worker-side Present discovery succeeded on WARP and
  hardware. All imported selected pixels were red, not the unrelated blue chain.
- Holding all three exported slots did not stop rendering: at least 12 further
  iterations completed and the worker reported full-ring drops.
- Stop while those slots remained held closed media, returned Bye, and the remote
  worker exited successfully. The passive renderer then completed at least 20 more
  iterations. It remained alive; the detour/trampoline were NOT removed and the DLL
  was NOT unloaded. Retained completed pixels remained readable.
- Both new ResidentDll recorder tests saved actual 640x480 H.264 MP4 clips and
  approved FFmpeg decoded every frame plus a visible primary-color sample.
- The representative hardware resident replay used NVIDIA H.264 Encoder MFT,
  reported 14 native GPU-surface inputs and zero encoder readbacks, and saved a
  roughly 850 ms MP4. Capture GPU-copy counters also reported zero CPU readbacks.
- The WARP resident replay used H264 Encoder MFT software encoding and explicitly
  reported 35 encoder readbacks. It decoded successfully but is NOT GPU-only.
- Three repeated startup/replay runs on each driver also passed before the final
  full decode gate. They were MP4 validation runs, not additional decoder runs.

## Failed startup experiment and correction

The first full gate hit ERROR_BAD_LENGTH (0x80070018) during resident replay startup
while the target was still initializing/loading modules. Microsoft explicitly
documents this snapshot race. The loader now retries ONLY that read-only snapshot
condition, at most 32 times with 10 ms spacing. Other failures remain terminal, and
the remote load itself is not retried. The repeated startup tests and final gate
passed after that change.

## Resource and lifetime boundaries

One owned-child reservation permits one DLL load and one worker bootstrap. Requested
remote data buffers are bounded to 64 KiB each; the actual packet is 128 bytes.
Windows allocation granularity may exceed requested data length. The load-path
buffer is reclaimed only after loader-thread completion. Worker packet memory is
reclaimed only after worker-thread completion; uncertain completion leaves it for
owned-process exit rather than freeing under a reader. Adapter cleanup terminates
only its exact owned child; its unreclaimed bootstrap data ends with that process.

Both the helper and original Present code module are pinned for process lifetime.
Capture buffers use the existing three-slot pools and try-send/try-lock behavior.
If cancellation cannot drain a callback, state remains resident, never executable
code reclamation. This deliberately one-shot resident route rejects cooperative
rearm and does not remove/reinstall hooks. No application texture allocation,
pipe IO, encoding, disk or UI waits are introduced in Present; driver internals
and MinHook are not claimed to have hard real-time bounds.

## Final validation

- Prototype gate: 24 feature-enabled library tests, two safe adapter tests, three
  helper ABI/bootstrap tests, existing native fixture suites, both new resident
  lifecycle scenarios, and all 16 native recorder tests (11 WARP/five hardware)
  passed, including approved decode gates.
- Probe/resident-enabled Clippy and experimental-hooks-only Clippy passed.
- Workspace formatting/Clippy/tests and frontend typecheck/lint/build passed via
  `scripts/check.ps1`. Existing settings-save browser regression and strict
  dependency verification passed. No new external dependency was resolved.

## Not established

No real game, arbitrary PID target, protected process, privilege escalation,
anti-cheat/Valorant interaction, signing/antivirus acceptance or production rollout.
The fixture still supplies opt-in command-line identity, inherited private pipes,
a known title and known BGRA8 D3D11 behavior. Generic game IPC/approval and broader
swap-chain/API support are not implemented by removing renderer pointer handoff.

No resident-route resize/device-loss rearm, actual GPU removal, concurrent selected
swap-chain calls, forced drain-timeout, loader-failure matrix, other vendors,
multi-GPU, fullscreen/Alt-Tab/minimize or performance/soak benchmark was tested.
The older cooperative rearm/decode tests still pass but are separate evidence.
Process-death/interrupted-GPU publication remains non-atomic as documented in ADRs
0051/0052. Driver calls, thread suspension and joins may stall. Leaving the hook
resident is NOT evidence of safe arbitrary DLL unload or universal game stability.
No game was selected on the user's behalf; a real-game test needs an explicit
non-protected D3D11 target and a separately approved target/IPC integration.
