# Native HUD Gate 3 Empirical Test Report — Valorant (2026-08-31)

## Executive Summary & Scope

This document records empirical presentation telemetry collected to validate
the native DirectComposition capture confirmation HUD against the Gate 3
gameplay impact criteria.

**Scope limitation:** This report documents a single tested machine, game, and
display configuration under specific benchmark conditions. It does not
constitute universal hardware verification, driver certification, or cross-game
validation.

- **Gate 3 Verdict:** **FAIL**
- **Status:** The native HUD cannot ship enabled or as an experimental feature
  in its current form.
- **Action:** End-to-end replay save A/B testing is paused pending causal
  isolation of presentation degradation.

---

## Test Environment

- **Operating System:** Windows 11 Pro 10.0.26200 (Build 26200)
- **GPU:** AMD Radeon RX 9070 XT
- **Display Driver:** 32.0.31041.1004 (Release Date: 2026-08-17)
- **Display Resolution & Refresh:** 2560x1440 @ 240 Hz
- **Target Application / Renderer:** `VALORANT-Win64-Shipping.exe`
- **Window Presentation:** Stable foreground borderless / windowed scene
- **User Observation:** Visible stutter reported during active HUD cue
  conditions in this matrix.

---

## Measurement Protocol & Tooling

Telemetry was captured using Intel PresentMon 2.5.1 portable CLI
(`PresentMon.exe`), signed by Intel Corporation (Authenticode Valid) and
approved for local developer measurement.

- **Targeting & Metrics:** Targeted directly at `VALORANT-Win64-Shipping.exe`
  recording QPC/v2 presentation metrics via ETW.
- **Capture Structure:** Six independent 64-second capture sessions.
- **Marker Sequence:** 2 warmup cue cycles followed by 10 measured markers with
  5-second spacing per session.
- **Isolation:** Standalone release native-HUD benchmark harness executing
  isolated UI presentation; it did not trigger replay encoding, muxing, or disk
  save I/O during presentation measurements.
- **Matrix Conditions:** Initial no-HUD baseline, idle HUD, saved Full mode,
  saved Compact mode, queued-to-saved cue transition, and final no-HUD recovery.

---

## Telemetry Results Matrix

| Condition | Total Frames | HW Independent (%) | Composed (%) | Undisplayed (%) | Frame Time Avg (ms) | Frame Time p99 (ms) |
|---|---|---|---|---|---|---|
| Initial no-HUD Baseline | 15,249 | 100.000% | 0.000% | 0.000% | 4.1966 ms | 5.0011 ms |
| HUD Idle (Shown Opacity-Zero HWND, No Cues) | 13,443 | 8.711% | 91.289% | 77.126% | 4.7594 ms | 9.0922 ms |
| Saved Cue (Full Mode) | 13,541 | 8.522% | 91.478% | 77.624% | 4.7251 ms | 9.0233 ms |
| Saved Cue (Compact Mode) | 13,550 | 8.672% | 91.328% | 77.720% | 4.7228 ms | 8.9929 ms |
| Queued-to-Saved Transition | 13,469 | 3.631% | 96.369% | 81.417% | 4.7460 ms | 8.9320 ms |
| Recovery no-HUD Baseline | 15,194 | 100.000% | 0.000% | 0.000% | 4.2001 ms | 5.9307 ms |

*Note: The recovery no-HUD baseline met paired telemetry gating against the
initial baseline, confirming the target application recovered full hardware
presentation once the HUD process was terminated.*

---

## Presentation Analysis & Causal Attribution

1. **Residual Hardware Presentation:**
   The non-zero hardware-independent percentages recorded in HUD sessions
   (3.631% – 8.711%) correspond entirely to the startup and teardown phases of
   the 64-second capture window before the HUD window attached or after it
   detached.
2. **Marker Window Presentation:**
   Across all measured idle and active marker windows, the target renderer was
   **0.000% Hardware Independent** and **100.000% Composed**, with **77% – 81%**
   of presented frames classified as undisplayed (dropped or composited out of
   refresh alignment by the Desktop Window Manager).
3. **Cue Drawing vs. Window State:**
   Active Direct2D rendering in Full mode (p99 9.0233 ms), Compact mode (p99
   8.9929 ms), and queued-to-saved transitions (p99 8.9320 ms) showed no
   meaningful paired p99 frame time degradation over the idle HUD baseline (p99
   9.0922 ms). This isolates the root cause: presentation degradation is driven
   by the presence of the unhidden HWND, attached DirectComposition root visual,
   or window capture exclusion state (`WDA_EXCLUDEFROMCAPTURE`), rather than
   the per-cue Direct2D draw operations.

---

## Limitations & Unverified Behaviors

This benchmark identified an unresolved presentation confound:
- **Confounded Variables:** The current evidence does not separate the
  individual impacts of `WDA_EXCLUDEFROMCAPTURE`, the attached DirectComposition
  visual tree, or the top-level transparent Win32 HWND.
- **Unverified Claims:** No claims are made regarding click-through
  reliability, capture exclusion effectiveness in captured video clips, input
  focus retention, HDR presentation behavior, behavior on other GPU
  architectures or driver branches, behavior with other game titles, or
  behavior under exclusive fullscreen modes.

---

## Raw Artifact Evidence Hashes

The raw telemetry datasets generated during testing were stored at:
`C:\Users\jaepe\AppData\Local\Temp\opencode\silk-hud-evidence\valorant-matrix-20260831-015438`

The SHA-256 checksums of the raw PresentMon CSV data files are:

| Condition | Raw CSV SHA-256 Checksum |
|---|---|
| Initial no-HUD Baseline | `a9990502cf8bb319e2115957743aff0b7ce31232487e05f35a0a5b09a33b3422` |
| HUD Idle | `156ae15c76065bfad7b9399594c197d9a022ad7ef86d3a499038eefc4b2faca8` |
| Saved Cue (Full Mode) | `4d4b55f040a003e314bb4e46c65742e1e87211cd405529d0d58bf14cdf139af6` |
| Saved Cue (Compact Mode) | `8ce21edd0e08ed88c5d35f6f335a7ea37d354e71cf14c37c86394b20e97d4606` |
| Queued-to-Saved Transition | `0fdc766cc9962a4db112350dbf4681978f85cab234940354b8e811ec5394e871` |
| Recovery no-HUD Baseline | `0cdb87f5bb94ad14f1f87a466d49e12429bb998b269ca18c3d13e9ccdf54fb6c` |

---

## Phase 4A Addendum — Capture Exclusion Disabled

Phase 4A repeated the controlled Valorant measurement with
`HudConfig.exclude_from_capture = false`. Production behavior was not changed;
the switch existed only in the standalone benchmark harness. Evidence was
stored at test time under:

`C:\Users\jaepe\AppData\Local\Temp\opencode\silk-hud-evidence\valorant-phase4a-exclusion-off-20260831-022505`

| Condition | Total Frames | HW Independent (%) | Composed (%) | Undisplayed (%) | Frame Time Avg (ms) | Frame Time p99 (ms) |
|---|---:|---:|---:|---:|---:|---:|
| Initial no-HUD, exclusion off | 13,606 | 93.988% | 6.012% | 1.889% | 4.7013 | 16.7906 |
| Idle shown HWND, exclusion off | 13,391 | 8.685% | 91.315% | 77.679% | 4.7784 | 9.2265 |
| Saved Full, exclusion off | 13,437 | 8.633% | 91.367% | 77.138% | 4.7621 | 9.3901 |
| Recovery no-HUD, exclusion off | 15,165 | 100.000% | 0.000% | 0.000% | 4.2188 | 5.9787 |

The initial no-HUD session's composed frames occurred outside the measured
marker windows; all measured baseline marker windows were 100% hardware
independent. The final no-HUD recovery was also wholly independent with no
undisplayed presents. In contrast, every measured idle and active HUD marker
window remained 0% hardware independent with capture exclusion disabled.

The user reported that visible stutter was reduced compared with the original
capture-exclusion-on matrix. This limits the conclusion: capture exclusion may
affect symptom severity on this setup, but disabling it does not restore the
required presentation mode and is not a sufficient remediation. DirectComposition
root attachment and shown-HWND presence remain to be isolated.

Gate 4A Oracle review attempt 1 of 3 passed this diagnostic isolation phase.
The review refuted capture exclusion as a sufficient cause or remediation,
authorized Phase 4B root/window isolation, and kept end-to-end replay-save A/B
paused. This is not product-release clearance.

Phase 4A raw PresentMon CSV SHA-256 checksums:

| Condition | Raw CSV SHA-256 Checksum |
|---|---|
| Initial no-HUD, exclusion off | `8d1bd0dd23bff4a97475a62db91963b2a868e64812769480658a5199179ae3dd` |
| Idle shown HWND, exclusion off | `f24a468188a7d2e4ee6a88f9b6fa327c5daa455872af3994e9dc57c3291b7a90` |
| Saved Full, exclusion off | `519b6d5e9377ba2be6125d7c7ebf45c5568eaa720c935137ad8211dc8ff3aeba` |
| Recovery no-HUD, exclusion off | `cdcbb9b0bcb6c599588941324f1de0cca1477e8d9888005da3ca9e0be9b7d863` |

---

## Phase 4B Addendum — Diagnostic Lifecycle Isolation (`detached-shown`)

Phase 4B isolated the DirectComposition root attachment lifecycle while keeping
the Win32 HWND shown continuously (`detached-shown` policy). Production code
and default behavior remained unchanged; the lifecycle policy was compiled only
in the benchmark harness under the `diagnostic-lifecycle` Cargo feature.

Capture exclusion (`exclude_from_capture = false`) remained disabled across all
Phase 4B runs to maintain single-variable isolation.

### Evidence Root & Invalidated Run Handling

- **Valid Clean Evidence Root:**
  `C:\Users\jaepe\AppData\Local\Temp\opencode\silk-hud-evidence\valorant-phase4b-restart-20260831-031134`
- **Process & Swapchain Target:** Target renderer PID `4908`, swapchain
  `0x2B09A96B1E0`.
- **Invalidated Session Exclusion:** Earlier test attempts under
  `valorant-phase4b-20260831-025745` were invalidated due to user-confirmed
  in-game scene transitions and an uncoordinated renderer restart. Those runs
  are explicitly excluded from the evidence dataset.

### Phase 4B Whole-Session Presentation Telemetry

| Condition | Total Frames | HW Independent (%) | Composed (%) | Undisplayed Presents (%) | Frame Time Avg (ms) | Frame Time p99 (ms) | Raw CSV SHA-256 Checksum |
|---|---:|---:|---:|---:|---:|---:|---|
| Initial no-HUD Baseline | 15,296 | 100.000% | 0.000% | 0.000% | 4.1836 ms | 4.7344 ms | `ffca7686d5252d77e07f8d82c79f9ad0ce90255b362c5f8d3182b39010a14234` |
| Idle `detached-shown` | 15,277 | 99.967% | 0.033% (5 frames) | 0.026% | 4.1887 ms | 4.6286 ms | `f9261e59a5f663801efa4a35ff083434027a564128e366264e6dd507b8630aeb` |
| Recovery no-HUD (after idle) | 15,257 | 99.993% | 0.007% (1 frame) | 0.000% | 4.1909 ms | 4.6603 ms | `acd6c775e858384e0e83aff88d31885c994d40d5fbfdbc538bb68873562b0cbe` |
| Saved Full `detached-shown` | 13,175 | 0.000% | 100.000% | 87.696% | 4.8545 ms | 9.0473 ms | `a25a7e766472e0e204fabac2c89e17f30ba627545e2f926f9ad5a16884de70e3` |

*Note on terminology: "Undisplayed Presents" counts PresentMon rows where
`DisplayedTime = NA`. It does not mean a literal application-side frame
generation failure or establish a one-to-one user-visible dropped frame.*

### Marker Window Telemetry & Lifecycle Findings

1. **Initial no-HUD Baseline:**
   All 10 measured marker windows exhibited 100.000% Hardware Independent Flip
   with 0% undisplayed presents. Paired control criteria were fully satisfied.
2. **Idle `detached-shown`:**
   All 10 measured marker windows exhibited 100.000% Hardware Independent Flip
   with 0% undisplayed presents. The generic analyzer reported a single-window
   p99 spike (8.219 ms; max paired delta +3.7064 ms, mean paired delta +0.3762
   ms). Similar isolated spikes occurred in the clean no-HUD recovery control (+3.7589
   ms max delta, +0.4382 ms mean delta), indicating normal system jitter rather
   than a lifecycle flaw. The 5 composed frames and 4 undisplayed presents
   occurred entirely during startup/teardown outside the marker intervals.
   **Idle detached-root successfully restored Hardware Independent Flip.**
3. **Active Saved Full (`detached-shown`) — Categorical Mode Latch:**
   Across all 10 measured active cycles, every pre-cue and post-cue marker
   window dropped to **0.000% Hardware Independent Flip** (100.000% Composed).
   The nominal cue lifecycle is 180 ms entrance + 2200 ms hold + 200 ms exit =
   2580 ms total duration, with a 5000 ms trigger period. The pre-marker
   sampling window (evaluating roughly 1.42 s to 2.42 s after expected root
   detachment) remained entirely composed. Attaching the root visual for an active
   cue triggered a **session-long mode latch into Composed Flip** that failed
   to recover between cues after cue completion and root detachment.
4. **User Experience:**
   The user reported that all cues rendered visibly, but visible stutter
   persisted throughout the active session.

### Current Status & Paused State

- **Gate 4B Status:** Incomplete. The user paused benchmark execution prior to
  the required no-HUD recovery control following the active condition and prior
  to evaluating the `attached-hidden` diagnostic policy.
- **Oracle Decision:** No Gate 4B Oracle verdict has been issued.
- **Save A/B Status:** End-to-end replay save A/B testing remains strictly paused.

---

## Phase 4B Reanalysis & Continuation Status (2026-08-31)

### Environment & Process Continuation State

- **Process Cleanliness:** Revalidation confirmed no lingering `hud_benchmark`
  or `PresentMon.exe` processes running.
- **Target Application:** `VALORANT-Win64-Shipping.exe` is not running. The
  original test PID `4908` and its DXGI swapchain `0x2B09A96B1E0` have terminated.
- **Pairing Invalidation for Future Runs:** Because the original renderer session
  is terminated, the missing post-active `detached-shown` no-HUD recovery cannot
  be captured retroactively. Any future benchmark capture under a new process PID
  must be treated as an independent session and cannot be paired with the
  historical `detached-shown` active session.
- **Tooling Verification:** Intel PresentMon 2.5.1 executable is verified at the
  approved path with Authenticode Valid status (Signer: `Intel Corporation`) and
  SHA-256 `9bec3083069f58f911e6a512f4806db51a27bd096103087bc1d05ef54c80a191`.
- **Live Measurement Blockers:** Live execution of the `attached-hidden` sandwich
  matrix and optional SteelSeries reference capture is blocked because Valorant
  is not in a stable foreground scene, the user is unavailable for required
  elevation/interactive validation, and no safe pre-configured SteelSeries cue
  trigger exists. No game input, memory modification, or anti-cheat state
  automation was attempted.

### Diagnostic Tooling Enhancements (v2)

To rigorously evaluate cue recovery within 200 ms without altering production
defaults, visual styling, or ADR 0022:
1. **Benchmark Marker Metadata:** Benchmark markers now emit `nominal_duration_ms`
   and `interval_ms` calculated dynamically from actual system reduced-motion
   capabilities, followed by a 5-second post-loop steady-state hold.
2. **Half-Open Windows & Recovery Metrics:** The generic analyzer evaluates
   conservative half-open windows:
   - Nominal cue active window: `[T_marker, T_marker + nominal_duration_ms)`
   - Post-cue recovery search: scans up to 200 ms after nominal completion
   - Post-recovery verification: `[nominal completion + 200 ms,
     nominal completion + 700 ms)`
   - Steady-state verification: `[nominal completion + 700 ms,
     T_marker + interval_ms)`
3. **Fail-Closed Presentation & Outlier Rules:** Evaluates PresentMon dropped
   frames/undisplayed ratios fail-closed; reports max outlier p99 warnings and
   marks unrecovered runs as explicitly censored (indicating presentation failure
   persisted throughout the entire inter-cue spacing).

*Timing Limitation Note:* Reanalysis of historical Phase 4B datasets uses a
legacy-derived standard 2580 ms Saved Full duration (180 ms entrance + 2200 ms
hold + 200 ms exit) because historical marker files predate inline metadata.
Nominal completion represents conservative scheduling evidence and does not
constitute a direct OS `ShowWindow` or DComp visual commit event callback.

### Enhanced Reanalysis Results (`analysis_recovery_v2`)

All datasets under valid evidence root
`C:\Users\jaepe\AppData\Local\Temp\opencode\silk-hud-evidence\valorant-phase4b-restart-20260831-031134`
were re-evaluated under the v2 recovery analyzer:

| Dataset / Condition | Outcome | Pre-Cue HW Indep (%) | Post-Recovery HW Indep (%) | Recovery Observed? | Warnings / Notes |
|---|---|---:|---:|---|---|
| Initial no-HUD Baseline | **PASSED** | 100.000% | 100.000% | N/A (Control) | 0 warnings |
| Idle `detached-shown` | **PASSED** | 100.000% | 100.000% | N/A (Idle) | 0 undisplayed in marker windows; 1 warning: paired p99 max delta +3.7064 ms (mean delta +0.3762 ms) |
| Recovery no-HUD (after idle) | **PASSED** | 100.000% | 100.000% | N/A (Control) | 0 undisplayed in marker windows; 1 warning: paired p99 max delta +3.7589 ms (mean delta +0.4382 ms) |
| Saved Full `detached-shown` | **FAILED** | 0.000% | 0.000% | **NO (Censored)** | 10 of 10 markers failed; 0% Independent pre- and post-cue; no clean recovery observed for at least 2420 ms after nominal completion; PostRecovery500 and SteadyState remained 0% Independent; ~86-90% undisplayed pre-ratios with severe frame drops; user visible stutter confirmed |

### Artifact SHA-256 Hashes (`analysis_recovery_v2`)

| Condition | `analysis_recovery_v2.json` SHA-256 | `analysis_recovery_v2.md` SHA-256 |
|---|---|---|
| Initial no-HUD Baseline | `55f57f57170d32ecfb0b4aba7715fe677b4f21de22c0a624b6b72a15f7d4645c` | `b7f6fc51a6cfc9f5a293aaf4e742c91fd993be05a206fed57b0e114bfd1c20ed` |
| Idle `detached-shown` | `5fe7edcb488ec800cc373ce9a17df1480e55dd97ef01bc129dbb330ce65cb501` | `8900ed0e8eb0884a2a34f3053ed77c5b1706f0610fe334b7d9ba6c95c47fab6d` |
| Recovery no-HUD (after idle) | `36d48a2564ac96f7736ed0bbd602890eea7125933f0d74a99d15dae87f760d45` | `713958cde59710b2c4c5f152e87a5c1804a491f305502efacff27fa8b78db27c` |
| Saved Full `detached-shown` | `acad567acac60f35b50c5c75e01e18c86b4beb4645b506683590a296d60a8b61` | `fbe1a590e90fd6a02eddede7b4fa4c99fb4bad7f178bc6f8986dca5b9165a1a3` |

### Synthesis & Current Project Posture

- **Gate 4B Status:** Incomplete. No Gate 4B Oracle verdict is possible without
  empirical measurement of the `attached-hidden` candidate lifecycle.
- **Phase 4C Status:** Not authorized.
- **Product Posture:** The native DirectComposition HUD remains blocked from
  production enablement due to the failed Gate 3 evaluation and the persistent
  presentation mode latch under `detached-shown`. Production code defaults and
  ADR 0022 remain unmodified.
- **Validation Note:** Deterministic focused unit/analyzer tests have passed;
  full test command logging and final gate verifications are owned by the parent
  orchestrator.
