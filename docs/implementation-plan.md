# Segmented Implementation Documentation

**Product:** Silk - Instant Replay Clipping Application
**Source specification:** [`spec.md`](../spec.md)
**Document status:** Implementation planning derived from the specification
**Implementation approach:** Independent implementation; OBS is a behavioral reference only (see spec §5)

---

## 1. Purpose and use

This document decomposes the specification into **ten ordered implementation segments (S0–S9)**, each aligned with the development milestones in spec §29. Every segment defines:

- Objective and entry criteria
- Requirements covered (traceable to spec §8 requirement IDs)
- Work breakdown
- Design notes and interfaces
- Testing and validation obligations
- Exit criteria checklist

Rules of engagement:

1. One segment is active at a time. A segment may start only when its entry criteria are met.
2. No segment may add a dependency without recording name, version, source, license, and purpose (spec §26).
3. Each segment ends with formatting, linting, unit tests, and relevant integration tests executed and reported.
4. Hardware-dependent claims must state what was actually tested (spec §30, rules 8–9).
5. Major technical decisions made during a segment are recorded as ADRs under `docs/decisions/` before implementation is finalized (spec §33).

---

## 2. Segment map

| Segment | Name | Spec milestone | Primary crates / areas | Key requirements | Status |
|---|---|---|---|---|---|
| S0 | Foundations and repository scaffold | M0 | workspace, `media-types`, `configuration`, `diagnostics`, mocks | APP-001/002/005/007 | **Exited 2026-08-26** ([evidence](progress/segment-S0.md)) |
| S1 | Synthetic replay engine | M1 | `replay-buffer`, `media-clock`, `test-support` | BUF-001…013 | **Exited 2026-08-26** ([evidence](progress/segment-S1.md)) |
| S2 | Native display capture | M2 | `capture-api`, `capture-windows` | VID-001…009, VID-011 | **In progress 2026-08-26** |
| S3 | Video encoding | M3 | `encoder-api`, `encoder-media-foundation` | ENC-001…010 | **Validated CPU/native-MF and WGC GPU-surface paths locally 2026-08-28; cross-vendor/decode checks deferred** |
| S4 | Audio capture and synchronization | M4 | `audio-api`, `audio-wasapi`, sync component | AUD-001…010 | **Deterministic ordered-track path delivered 2026-08-27; native endpoint and drift evidence open** |
| S5 | Direct MP4 muxing and replay saving | M5 | `muxer`, save-job pipeline | MUX-001…011, STO-002/003 | **Deterministic direct MP4 save path delivered 2026-08-27; media-tool validation open** |
| S6 | Global hotkeys and tray | M6 | `hotkeys`, tray integration | KEY-001…006, NOT-001…004 | **Implementation delivered 2026-08-27; interactive native verification open** |
| S7 | Desktop UI and clip library | M7 | `clip-library`, `apps/desktop` | LIB-001…011, STO-001…008, APP-003/004 | **Initial settings/library and game-aware storage slice delivered 2026-08-27; native UX evidence open** |
| S8 | Reliability and recovery | M8 | all engine crates | VID-006/007, AUD-005/010, §21 recovery | **Deterministic slice validated 2026-08-27; native/8-hour evidence open** |
| S9 | Packaging and release | M9 | installers, signing, diagnostics export | §24 privacy/security, §28 acceptance | **In progress 2026-08-27; release evidence open** |

### Dependency graph

```text
S0 ──> S1 ──> S2 ──> S3 ──> S4 ──> S5 ──> S6 ──> S7 ──> S8 ──> S9
        │             │             │
        └─────────────┴─────────────┴──> synthetic pipeline tests reused in every later segment
```

S2–S5 can be partially overlapped only if their public interfaces (defined in S0/S1) remain stable. Any interface change requires an ADR note and re-run of downstream tests.

---

## 3. Cross-cutting conventions (apply to all segments)

These obligations are inherited by every work item and are not repeated per segment.

### 3.1 Error model (spec §20)

Every error crossing a module or IPC boundary carries: stable code (from the §20 category list), human-readable message, component, severity, recoverability, suggested action, underlying platform error, timestamp, redacted context. Raw internal errors never reach the UI without a mapped user-facing explanation.

### 3.2 Observability (spec §22)

Each segment adds metrics for its own domain to the required metric set (captured FPS, encoded FPS, drops, latency, drift, buffer duration/memory, save duration, mux failures, disk space, restarts, active backend). Logging is structured JSON Lines, rotated, media-free, and path/user-redacting.

### 3.3 Concurrency rules (spec §14)

- Real-time workers (capture, audio, encode, buffer ingestion) never block on disk I/O, thumbnails, UI, or unbounded locks.
- All inter-worker channels are bounded; backpressure follows the documented degradation order (skip preview → delay thumbnails → reduce diagnostics → drop late video frames → report encoder overload → stop cleanly).
- No replay-buffer write lock is held during file output. Thread ownership of platform resources is documented per crate (`docs/architecture/threading.md`).

### 3.4 Reference-use policy (spec §5)

No OBS code, identifiers, structure, assets, comments, issue/PR code, or GPL dependencies without approval. Every crate has an original interface, project-specific naming, and tests derived from this specification.

### 3.5 Definition of done (spec §34)

Behavior implemented, interfaces documented, success + failure paths tested, fmt/lint/tests pass, dependencies recorded, errors handled, logging/metrics present, no unrelated regressions, hardware assumptions identified, docs updated, reference-use policy respected.

---

## 4. Segment S0 — Foundations and repository scaffold

**Milestone:** M0 · **Depends on:** nothing · **Status:** Exited 2026-08-26 — see [`progress/segment-S0.md`](progress/segment-S0.md)

### 4.1 Objective

Establish the workspace, shared types, configuration, logging, state model, mock implementations, UI shell, and governance documents — with no native capture, encoding, or hotkey code.

### 4.2 Entry criteria

None (project start). Spec §32 "Initial implementation prompt" applies verbatim.

### 4.3 Requirements covered

| ID | Coverage in this segment |
|---|---|
| APP-001 | Windows 10/11 target declared; toolchain pinned; CI runs on Windows runners |
| APP-002 | Start/stop/save actions defined as controller commands (mock-backed) |
| APP-005 | Single-instance guard implemented at application level |
| APP-007 | Recorder core crates have no dependency on Tauri/React |

### 4.4 Work breakdown

1. **Workspace scaffold** matching spec §12 exactly: `crates/*`, `apps/desktop`, `docs/{architecture,decisions,testing,licensing}`, `scripts`, `tests/{integration,endurance,fixtures}`.
2. **Governance files:** `AGENTS.md` (content from spec §31), `README.md`, `LICENSE`, `THIRD_PARTY_LICENSES.md` inventory template with the §26 dependency table pre-seeded.
3. **`media-types`:** `StreamId`, `MediaType`, `TimeBase`, `EncodedPacket` (fields exactly as spec §13.7), `VideoFrame`, `AudioFrame`, `PacketPayload`. Payloads are reference-counted/shareable and immutable after creation.
4. **`configuration`:** versioned JSON schema mirroring spec §18; load/save; migration framework keyed on a `"version"` field; validation returning structured field-level errors.
5. **`diagnostics`:** structured logger (JSONL), rotation by age/size, redaction hooks for paths/usernames, log-level configuration from settings.
6. **`recorder-engine` skeleton:** recorder states with the transition table from spec §19 as an exhaustive, tested function; invalid transitions return structured errors.
7. **Trait definitions:** `VideoCapture`, `AudioCapture`, `VideoEncoder`, `AudioEncoder` exactly as spec §13.2/13.3/13.6, plus a `Muxer` trait (§13.8 responsibilities).
8. **Mock implementations** in `test-support`: scripted capture sources, fail-injectable encoder/muxer, all deterministic and seedable.
9. **`app-controller` skeleton:** command/event enums, notification dispatcher interface, save-job manager stub, single-instance guard.
10. **Desktop shell:** Tauri + React + TypeScript app that builds, shows a placeholder main window, and round-trips one command and one event through the IPC boundary (allowlisted command table established).
11. **CI scaffolding:** `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, TypeScript typecheck/lint/build jobs on Windows.

### 4.5 Design notes

- The controller never processes media frames (spec §13.1); enforce via crate dependency direction (controller depends on engine *events*, not frame types).
- Channel bound sizes are named constants with rationale referencing the backpressure order.
- Thread ownership assumptions for each worker role listed in spec §14 live in `docs/architecture/threading.md`.

### 4.6 Testing

- Unit: configuration validation (valid/invalid/migrating configs), state transition table exhaustiveness, error-code mapping, chord normalization, redaction, rotation.
- Integration: mock end-to-end (scripted capture → mock encoder → mock muxer) produces a clip entry through the controller, including failure-injection paths.
- UI: typecheck/lint/build gates; one IPC command + event wired in the shell.

### 4.7 Exit criteria

- [x] Workspace builds clean on Windows
- [x] fmt/clippy/typecheck/tests pass
- [x] State machine rejects invalid transitions with structured errors
- [x] Config loader handles missing file, corrupt JSON, old versions
- [x] No native WGC/WASAPI/FFmpeg/hotkey code present
- [x] `AGENTS.md` and license inventory template exist

---

## 5. Segment S1 — Synthetic replay engine

**Milestone:** M1 · **Depends on:** S0

### 5.1 Objective

Implement the replay buffer, media clock, and timestamp normalization against synthetic inputs, so that the hardest correctness logic exists and is proven before any platform code is written.

### 5.2 Entry criteria

- S0 exit criteria met; `EncodedPacket`, `TimeBase`, traits frozen (or changes ADR'd).

### 5.3 Requirements covered

BUF-001 through BUF-013 (all), plus clock requirements from spec §13.4.

### 5.4 Work breakdown

1. **`media-clock`:** monotonic source (QPC-backed abstraction), conversion of arbitrary source stamps into a shared time base, initial-offset compensation, discontinuity detection threshold API, wall-clock captured only as metadata.
2. **Replay-buffer insertion (spec §15.1):** validate stream/timestamps → assign monotonic `sequence` → insert into stream-aware storage → advance newest-timeline position → evict beyond `T_b = T_n − D` → retain newest video keyframe at or before `T_b` → update memory/duration metrics.
3. **Eviction correctness (spec §15.2):** eviction never removes a keyframe still needed by the live window or an outstanding snapshot; refcounted payloads make release cheap.
4. **Snapshot creation (spec §15.3):** given save time `T_s`, compute `T_r = T_s − D`; select from latest keyframe ≤ `T_r`; include overlapping audio; copy codec parameters; take immutable references; release lock before handing to worker. Snapshot outlives subsequent live eviction (BUF-009).
5. **Timestamp normalization (spec §15.4):** rebase PTS/DTS to zero-origin preserving decode/presentation ordering; distinct DTS where codec requires.
6. **Audio alignment (BUF-011):** trim/pad audio to selected video interval at snapshot level.
7. **Duration change (BUF-012):** shrinking/growing `D` at runtime re-evaluates retention without restart.
8. **Concurrency (BUF-008):** serialized save-request queue over snapshots; concurrent-save property tests.
9. **Synthetic generators (spec §27.3):** seeded video/keyframe/audio streams, timestamp jitter, resolution-change events, device-loss events, encoder-delay injection.
10. **Pre-roll reporting (BUF-013):** snapshot records actual start time vs requested; length reported to caller.

### 5.5 Design notes

- Storage: per-stream deques of packets ordered by DTS; global timeline tracked separately. Memory bound enforced via byte accounting including payload refcounts; document overhead factor (spec §15.5).
- Locking: single short mutex (or lock-free MPSC) around insertion/eviction; snapshots clone reference handles, never payloads.

### 5.6 Testing

- Unit: insertion ordering, sequence monotonicity, eviction at exact boundaries, keyframe retention edge cases (no keyframe in window, keyframe exactly at `T_b`).
- Property (spec §27.2): buffer duration ≤ bound + allowed keyframe pre-roll; packet ordering valid under random inserts/jitter; snapshots valid after randomized eviction; normalized timestamps never precede origin; randomized concurrent saves leave buffer consistent.
- Deterministic seeds committed as fixtures (`tests/fixtures/seeds.json`) for regression.

### 5.7 Exit criteria

- [x] Randomized snapshot tests pass across ≥10⁵ generated insertion-steps (160 committed seeds × 700 steps, ~0.8 s)
- [x] Buffer memory stays within calculated bound + documented overhead (ADR 0004)
- [x] Concurrent save simulations produce non-corrupting snapshots
- [x] Duration change applies live
- [x] All S1 tests run in < 60 s (usable as fast guardrails)

---

## 6. Segment S2 — Native display capture

**Milestone:** M2 · **Depends on:** S0, S1 (traits + clock)

### 6.1 Objective

Replace mock video input with real Windows Graphics Capture (WGC) over Direct3D 11, producing timestamped GPU frames into the engine.

### 6.2 Entry criteria

- S1 complete; machine(s) with single and multi-monitor setups available; ADR drafted for minimum supported Windows build (open decision #12).

### 6.3 Requirements covered

VID-001…009 (VID-010 window capture excluded), VID-011 (no injection — verified by design review).

### 6.4 Work breakdown

1. **Display enumeration:** adapter + monitor enumeration via DXGI; stable `sourceId` scheme surviving reboots/replug where possible; expose `VideoSourceInfo`.
2. **WGC session:** capture one selected monitor using free-threaded session creation; D3D11 texture pool sized for the configured resolution; explicit OS capability check.
3. **Frame transport:** received textures wrapped as `VideoFrame` GPU payloads; zero-copy handoff to the video worker via bounded channel; timestamps stamped into the shared time base (VID-008). The encoder-side resolver/context boundary is defined in [ADR 0006](decisions/0006-gpu-handle-resolver-context.md).
4. **Resolution/format handling:** detect size/format changes (VID-006) and emit `FormatChanged`; pause submission until epoch handling responds (completed in S3/S8).
5. **Source loss:** detect monitor disconnect/session invalidation (VID-007); emit loss event; bounded reacquisition; controlled state surfaced to controller.
6. **FPS shaping:** deliver at selected 30/60 FPS; duplicate-frame suppression behind a flag pending compatibility testing (VID-009).
7. **Cursor capture policy:** configurable, default on; privacy implication documented.
8. **Resource hygiene:** explicit release of sessions/frame pools/D3D resources on stop; leak checks in soak test.

### 6.5 Design notes

- All COM/D3D objects owned by the dedicated video-capture worker thread (documented ownership); nothing crosses threads except immutable frame wrappers with proper `Send` semantics.
- Capture restarts counted as a §22 metric.

### 6.6 Testing

- Integration: enumerate real displays; capture each for 60 s; assert frame rate within tolerance and timestamps monotonic.
- Soak: 1-hour continuous capture; measure GPU-resource growth via process counters.
- Manual matrix entries: single monitor, multi-monitor, primary/unplugged secondary, 60 Hz and high-refresh panels.
- Failure injection: scripted display disable → controlled Recovering/Error state, no crash.

### 6.7 Exit criteria

- [ ] Stable 1080p60-class display capture for 1 hour with no growing GPU-resource usage
- [ ] Resolution change produces an event consumed without crash
- [ ] Display removal yields controlled state within bounded time
- [ ] No injected code into any process (design review sign-off)

---

## 7. Segment S3 — Video encoding

**Milestone:** M3 · **Depends on:** S2 (frames available; synthetic frames also sufficient)

### 7.1 Objective

Produce valid H.264 packets from captured and synthetic frames, preferring hardware encoders, with capability discovery, ranking, overload metrics, and fallback plumbing.

### 7.2 Entry criteria

- S2 capturing stably; ADR chosen for first backend: FFmpeg libavcodec vs Media Foundation (open decision #1).

### 7.3 Requirements covered

ENC-001…010 (ENC-011 HEVC/AV1 deferred).

### 7.4 Work breakdown

1. **Backend selection (ADR):** evaluate FFmpeg (NVENC/AMF/QSV wrappers) vs Media Foundation for H.264; record licensing implications (FFmpeg build-flag review per spec §26) and pick one.
2. **`encoder-api`:** freeze trait behavior — configure, encode, drain, capabilities, and the session-scoped `GpuFrameContext`; backend types never escape the crate (spec §13.6).
3. **Capability discovery (spec §16.1):** enumerate NVENC/AMF/QSV/software; validate codec+resolution+FPS support; trial-init; rank per §16.2 (HW preferred, then alternate HW, then software); persist diagnostic explanation.
4. **Configuration mapping:** quality presets (ENC-008), optional target bitrate configured via `MF_MT_AVG_BITRATE` (requiring hardware validation) (ENC-009), keyframe interval default 2 s (ENC-005/006), RC mode per backend, pixel format 8-bit 4:2:0.
5. **Epoch management:** encoder (re)init starts a new replay-buffer epoch; incompatible configurations never mix in one clip (spec §16.3).
6. **Overload telemetry (ENC-007):** dropped-frame counts, encode latency percentiles, backend-reported overload events → metrics + degraded-state signal.
7. **Fallback chain (ENC-003/004):** init failure → try next ranked backend; runtime failure → controlled reinit once → next backend; user-visible quality/performance notice.
8. **Software fallback:** CPU-bound profile validated as functional even if not meeting PERF-007.

### 7.5 Design notes

- Encoding worker owns the encoder instance; frame channel bounded (drop-late policy per §14.1); packets flow to buffer ingestion with encoder-assigned time bases converted via `media-clock`.

### 7.6 Testing

- Unit: ranking algorithm with fake capability sets; preset→backend-config mapping; epoch tagging.
- Integration: synthetic frames → H.264 elementary stream decodes fully (`ffmpeg -v error -f null -`); captured display frames → same; forced-failure injection exercises the full fallback chain.
- Hardware matrix rows exercised where machines exist (NVIDIA, AMD, Intel iGPU, multi-GPU laptop) — explicitly list what was NOT tested (spec §30 rule 8).

### 7.7 Exit criteria

- [ ] Valid H.264 produced from both synthetic and captured frames
- [ ] HW encoder used automatically when present; falls back correctly on injected failure
- [ ] Keyframe interval honored (verified via GOP inspection)
- [ ] Overload/drop metrics visible in logs/metrics

---

## 8. Segment S4 — Audio capture and synchronization

**Milestone:** M4 · **Depends on:** S3 (encoder pipeline live)

### 8.1 Objective

Add independently configured WASAPI audio sources, including desktop loopback and
optional microphone capture, with resampling, per-track gain, and shared-timeline
synchronization producing stable A/V packet streams.

### 8.2 Entry criteria

- S3 complete; ADR for drift tolerance (open decision #7) drafted.

### 8.3 Requirements covered

AUD-001…AUD-010 (AUD-011 noise suppression deferred).

### 8.4 Work breakdown

1. **Track configuration:** ordered v2 `AudioTrackSettings` entries provide stable IDs, names, source kinds, enablement, device IDs, and gain; the controller passes only enabled entries to the next session.
2. **`audio-wasapi` sources:** default-output loopback and input capture (AUD-001/002); format discovery; event-driven reads on one dedicated worker per enabled track.
3. **Device change handling:** MMDevice notifications; auto-recover after change (AUD-010); availability events surfaced.
4. **Resampling:** convert device formats to encoder-required format (48 kHz target, AUD-007); added latency documented.
5. **Gain control (AUD-009):** bounded linear multiplier applied independently to each source before encoding.
6. **Shared timeline (spec §13.4/13.5):** WASAPI timestamps converted into shared time base; initial offsets compensated; drift/discontinuities detected; sync metrics emitted; continuous audio prioritized over frame insertion.
7. **Track separation (AUD-008):** ordered, named `stream_id`s remain separate end-to-end (buffer → mux); the dependency and migration boundary are recorded in [ADR 0015](decisions/0015-configurable-audio-track-list.md).
8. **Sleep/resume groundwork:** recreate invalid audio clients post-resume; new buffer epoch (full recovery matrix in S8).

### 8.5 Design notes

- One worker per enabled configured audio track per spec §14; audio never dropped in favor of video except as last resort per backpressure order.
- Discontinuities (device swap, gap > threshold) mark affected regions; malformed packets are never inserted (spec §21.3).

### 8.6 Testing

- Unit: resampler format matrix; offset compensation math; drift detector thresholds.
- Integration: configured desktop/input tracks; combined clip with separately named streams; per-track ordering/gain; one source failing mid-capture → sibling tracks continue and the event is surfaced.
- Validation: ffprobe confirms expected streams/sample rates; manual sync check; A/V drift measured against the tolerance ADR over 30 minutes.

### 8.7 Exit criteria

- [x] Enabled ordered audio plans create independently owned workers and named streams
- [x] Per-track gain is validated and applied in deterministic synchronization tests
- [ ] Combined clip decodes with synchronized A/V within agreed tolerance
- [ ] Mic loss does not disturb desktop audio capture
- [ ] Device swap recovers automatically within bounded time
- [ ] Sync metrics exposed (drift, discontinuities)

---

## 9. Segment S5 — Direct MP4 muxing and replay saving

**Milestone:** M5 · **Depends on:** S4 (both streams flowing)

### 9.1 Objective

Turn replay snapshots into validated, atomically published standard MP4 clip files without intermediate containers, remuxing, or interrupting capture.

### 9.2 Entry criteria

- S4 exit met; [ADR 0021](decisions/0021-direct-mp4-replay-output.md) selects pure-Rust direct MP4 muxing.

### 9.3 Requirements covered

MUX-001…011; STO-002 (pre-save disk check), STO-003 (low-disk safety), STO-008 (temp cleanup); supports BUF-007/009/010 behaviors end-to-end.

### 9.4 Work breakdown

1. **Container strategy (ADR):** [ADR 0021](decisions/0021-direct-mp4-replay-output.md) selects pure-Rust direct MP4 output using the `mp4` crate. Replay snapshots are muxed directly to `.mp4` without intermediate Matroska containers or post-save re-encoding.
2. **`muxer` crate:** receive immutable snapshot; transform H.264 Annex B access units to length-prefixed AVCC samples with SPS/PPS embedded in the `avcC` track box; write separate AAC audio tracks; apply timestamp normalization; interleave streams by DTS; write sequential `ftyp -> mdat -> moov` boxes with trailing `moov` metadata; perform basic structural self-validation; return clip metadata.
3. **Temporary-file discipline (MUX-005/006):** writes go to deterministic sibling `.part` names; incomplete outputs never enter the library.
4. **Atomic finalization (MUX-007):** publish via same-volume rename after validating top-level boxes and timestamp monotonicity.
5. **Save job pipeline (spec §9.4):** request → snapshot → bounded save-worker submission → `SaveQueued` → write/finalize → validate → success/failure event. Library indexing remains S7; capture ingestion never blocks on container I/O (BUF-007, PERF-003).
6. **Disk-space gate (STO-002/003):** the save worker checks available space with `GetDiskFreeSpaceExW`, reserves a fixed safety margin plus a bounded snapshot estimate, and refuses gracefully below the threshold; recorder capture continues.
7. **File naming (MUX-009/010):** configurable `{date}`, `{time}`, `{source}`, and `{index}` rendering, Windows-invalid character/reserved-name sanitization, and collision suffixes are implemented in the controller.
8. **Failure reporting (MUX-011):** structured errors are surfaced and normal write failures remove the staged `.part`; stale crash artifacts are removed at controller startup.
9. **Automated media validation harness (spec §27.5):** `scripts/validate-media.ps1` wraps `ffprobe`/`ffmpeg` checks asserting readability, streams, duration tolerance, monotonic timestamps, and full decode.

### 9.5 Design notes

- The S5 implementation uses one bounded save worker per controller; jobs are serialized per BUF-008, save metrics are shared by short updates, and snapshot lifetime is managed by shared packet references.
- Direct MP4 generation operates entirely in pure Rust behind the project's `Muxer` trait, keeping capture and save logic independent from external processes.
- FFmpeg is standalone optional tooling used for development and CI media validation; no FFmpeg libraries are linked into Silk.
- Trailing `moov` layout supports single-pass streaming disk I/O without speculative space reservation; local media players seek directly to the trailing metadata.

### 9.6 Testing

- Unit: naming sanitizer matrix; disk-gate thresholds; normalization at mux boundary; AVCC conversion; multi-track AAC box layout; structural box validation.
- Integration (spec §27.4 subset): display+audio → MP4 file; consecutive saves; save during heavy capture; low-disk simulation; output-directory removed mid-session.
- Endurance seed: 100 sequential clips save and fully decode; capture FPS uninterrupted throughout.

### 9.7 Exit criteria

- [ ] 100 consecutive MP4 clips saved, validated, decoded
- [ ] No capture interruption during saves (metrics show continuous ingest)
- [ ] Incomplete/corrupt files never appear in library
- [ ] Temp files cleaned after induced failures and simulated crash-restart
- [ ] Separate AAC audio tracks preserved in MP4 container

---

## 10. Segment S6 — Global hotkeys and tray

**Milestone:** M6 · **Depends on:** S5 (save pipeline to trigger)

### 10.1 Objective

Make clipping operable from anywhere in Windows: registered global hotkeys, conflict detection, tray control, and save notifications.

### 10.2 Entry criteria

- S5 saving reliably; settings store exposes hotkey fields with validation (chord validator already shipped in S0's `configuration` crate).

### 10.3 Requirements covered

KEY-001…006; NOT-001…004 (NOT-005 sounds deferred); APP-003/APP-004 polish.

### 10.4 Work breakdown

1. **`hotkeys` crate:** Win32 `RegisterHotKey` on a dedicated message-loop thread; chord parsing/normalization (reuse `configuration::normalize_chord`); re-register on settings change without restart. **Core registration and replacement path delivered in [ADR 0011](decisions/0011-win32-global-hotkeys.md).**
2. **Conflict detection (KEY-002):** registration failure mapping; best-effort detection of known system/browser conflicts; clear user message with suggestion. **Registration failure mapping delivered; interactive conflict validation pending.**
3. **Global operation (KEY-003):** verified with focus on fullscreen game-class windows and unfocused desktop apps. **Implementation delivered; interactive verification pending.**
4. **Debounce/coalesce (KEY-004):** rapid repeats queue serialized save requests; stress-tested for output integrity. **Bounded event delivery and 50 ms debounce delivered; stress validation pending.**
5. **Start/stop hotkeys (KEY-005):** optional second/third bindings. **Validated binding construction, live replacement, startup loading, and atomic persistence through the `configure_hotkeys` shell command are delivered; the minimal three-field editor is S7 groundwork.**
6. **Tray:** icon states stopped/active/error (spec §9.3); menu per §9.3; close-to-tray honoring APP-003. **Tauri tray menu, state-aware controls, restore-on-click, and close-to-tray behavior delivered; native icon variants remain a polish item.**
7. **Notifications:** Windows toast with in-app fallback (NOT-001/002); Open and Reveal actions (NOT-003/004); preference gating; failures include error code + suggested action. **Native save/failure notifications delivered through `tauri-plugin-notification`; a native DirectComposition capture-confirmation HUD is integrated in place of a WebView overlay. Desktop action buttons and persisted preference gating remain pending.**

### 10.5 Design notes

- Hotkey events enter the controller as commands — identical path to UI buttons — so save logic has one owner.
- Tray lives in the Tauri app, reflects controller state via events, issues commands only.

### 10.6 Testing

- Unit: chord parse/serialize (already covered); conflict mapping.
- Integration: hotkey fires while another app focused; 20 rapid activations → only valid clips or coalesced count; toast shown with working actions.
- Manual: multi-monitor/focus games; document elevated-app limitation if encountered.

### 10.7 Exit criteria

- [ ] Hotkey works while other applications (including games) are focused
- [ ] Repeated activation produces only valid clips
- [ ] Conflicts detected and reported clearly
- [ ] Tray states/menu correct; notifications actionable

---

## 11. Segment S7 — Desktop UI and clip library

**Milestone:** M7 · **Depends on:** S6 (hotkeys/tray integrated)

### 11.1 Objective

Build the first usable desktop surface, generic audio-track settings, and the
initial filesystem-backed clip library. The current slice deliberately stops
short of playback, thumbnails, and the complete first-run flow.

### 11.2 Entry criteria

- S6 implementation review clean; native interactive verification remains a
  documented hardware/manual follow-up. The initial catalog decision is
  recorded in [ADR 0013](decisions/0013-initial-clip-library-catalog.md).

### 11.3 Requirements covered

LIB-001…010; STO-001…008 (quota/protection/auto-delete UX); APP-002/003/004 polish; §8.11 settings surface; §9 UX flows; §25 accessibility.

### 11.4 Work breakdown

1. **`clip-library` crate:** the initial slice uses a versioned JSON index
   (name, path, created_at, duration, size, missing, and protected flags) and a
   bounded library worker. It reconciles completed MP4 files, ignores staged
   artifacts, validates rename/delete/protection paths, and provides secure
   reveal/open actions. SQLite, thumbnails, filtering, and virtualization remain
   later work.
2. **Storage policies (STO-004…007):** delivered quota config, storage summary,
   and oldest-first auto-delete excluding protected or missing clips; **auto-delete
   remains disabled by default** (STO-006) with explicit disclosure at enable time.
3. **First-run flow (spec §9.1):** 10-step wizard with skip affordances, ending in encoder capability check and optional test recording (reuses S5 save path).
4. **Main window (spec §9.2):** initial status, active source, replay duration,
   encoder/audio summary, buffer counters, Save Replay, Start/Stop, recent clip
   library, warnings/activity, and settings access are delivered. Audio meters,
   performance dashboards, and the complete first-run flow remain planned.
5. **Settings UI (§8.11):** startup-loaded, validated, atomically persisted
   editing covers capture, ordered audio tracks (source, device, name, enablement,
   and gain), encoding (quality preset or custom bitrate), replay duration, output
   directory and naming, global hotkeys, storage quota, diagnostics, and
   application preferences. Config schema v4 cleans obsolete container selection,
   remuxing, and post-save export controls.
6. **Playback:** Reveal/Open-external actions and missing-file rendering are
   delivered in the initial slice; embedded playback and thumbnails remain
   deferred.
7. **IPC hardening (spec §24):** command allowlist finalized (seeded in S0); paths/names validated native-side regardless of UI input; typed event channel.
8. **Accessibility (§25):** keyboard navigation pass, focus visibility, scaling at 100–200 %, contrast audit, reduced-motion respect, screen-reader labels for status changes.
9. **Performance:** UI work off the media path (enforced since S0); virtualized lists; debounced metric updates (PERF-006).

### 11.5 Design notes

- Library DB is an index only; FS is authoritative (spec §13.9). Rows verify file existence on render.
- Thumbnails stored in an app cache dir, not the clips folder.

### 11.6 Testing

- Unit: initial reconciliation, missing-file retention, native MP4 duration
  parsing, Windows-safe rename validation, delete/catalog consistency, secure
  action paths, and quota ordering. Pagination and thumbnail queries remain
  pending.
- Integration: scripted full MVP flow: setup → capture → hotkey save → library shows playable clip → rename/delete/reveal; 1,000-row fixture dataset remains responsive.
- Accessibility: keyboard-only walkthrough; contrast report.

### 11.7 Exit criteria

- [ ] Full MVP user flow works without CLI tools (spec M7 criterion)
- [ ] Library correct at 1,000+ entries with async thumbnails
- [x] Quota enforcement deletes oldest unprotected clips only, off by default
- [ ] Missing-file detection accurate after manual FS tampering
- [ ] Keyboard-only operation possible end-to-end

The initial S7 slice is not an exit from this segment: playback, thumbnails,
quota policy, first-run setup, large-library validation, and accessibility
walkthrough evidence remain open.

---

## 12. Segment S8 — Reliability and recovery

**Milestone:** M8 · **Depends on:** S7 (product surface complete enough to exercise)

### 12.1 Objective

Harden every failure path: display/audio recovery, sleep/resume, encoder fallback under load, crash cleanup, and the endurance suite as repeatable automation.

### 12.2 Entry criteria

- S7 exit met; endurance harness scaffolding exists in `tests/endurance`.

### 12.3 Requirements covered

Consolidates VID-006/007, AUD-005/010, ENC-003/004, STO-003/008, all of spec §21; acceptance items 11–13, 15–16 (§28).

### 12.4 Work breakdown

1. **Resolution change (spec §21.1):** implement the seven-step response — pause submission → flush/close encoder epoch → reconfigure surfaces → reinit encoder → segment replay data → resume → inform UI of buffer rebuild.
2. **Display disconnection (§21.2):** recovering state, bounded reacquisition window, user prompt to pick another display, clean stop on failure.
3. **Audio device loss (§21.3):** continue video; mark the affected source unavailable; recover independently; never emit malformed packets; notify when a saved clip lacks an audio track.
4. **Sleep/resume (§21.4):** resource recreation, audio-client reinit, encoder verification, new buffer epoch, no cross-discontinuity clips.
5. **Encoder fallback under load:** inject failures during 60 FPS capture; verify chain from §16.3 including epoch separation.
6. **Crash cleanup:** kill-process during save → next launch finds staged files, cleans/quarantines, library reconciles; single-instance lock released correctly (APP-005).
7. **Endurance suite automation (spec §27.6):** scripted 8-hour capture; 100 sequential saves; start/stop cycling; settings-change loops; display/audio replug loops; slow-disk simulation; concurrent save + library scan. Sampled metrics: memory, handles, GPU resources, threads, queue depths, save latency, drops, drift — with trend assertions (no growth).
8. **Degraded-mode tuning:** verify backpressure ladder fires in-order under artificial overload; document thresholds.

### 12.5 Design notes

- Epoch ID threaded buffer→snapshot→mux so validation rejects mixed-epoch clips defensively.
- Recovery timers centralized in the controller to avoid scattered retry logic.

### 12.6 Testing

- This segment is mostly tests: automated runners plus documented manual procedures where hardware manipulation is unavoidable (display unplug, BT audio, USB mic).
- Acceptance mapping: run §28 items 11–16 scenarios and record evidence.

### 12.7 Exit criteria

- [ ] Eight-hour endurance run: no unbounded growth in memory/handles/GPU/threads/queues
- [ ] All recovery scenarios produce controlled states, never crashes
- [ ] Post-crash relaunch leaves no stray temp files and a consistent library
- [ ] Encoder fallback works mid-session without corrupting buffered clips

---

## 13. Segment S9 — Packaging and release

**Milestone:** M9 · **Depends on:** S8

### 13.1 Objective

Ship signed, reproducible installers with licensing paperwork, diagnostics export, update strategy, and release documentation.

### 13.2 Entry criteria

- S8 exit met; code-signing certificate procured; ADR for update framework (open decision #11) drafted.

### 13.3 Requirements covered

§24 security/privacy items (signing, reproducible deps, redaction); §28 items 1, 17–20; §22.3 diagnostic package; NOT/LIB polish fixes found in release testing.

### 13.4 Work breakdown

1. **Installer:** choose MSIX vs NSIS/Inno (ADR); WebView2 bootstrapping policy; per-user vs per-machine scope; clean-machine upgrade/uninstall tests.
2. **Code signing:** sign binaries and installer (spec §24); signing integrated into CI with secret handling; timestamping.
3. **Update strategy (ADR):** signed updater with manifest; opt-in checks only — no mandatory telemetry, no silent uploads.
4. **Dependency compliance:** finalize `THIRD_PARTY_LICENSES.md` per §26 (name/version/source/license/linking/artifacts/notices/approval); generate bundled notices; record FFmpeg obligations as unlinked standalone validation tooling.
5. **Diagnostic export (§22.3):** user-triggered package: versions, GPU/driver info, encoder caps, redacted settings/logs/error codes; path redaction verified; explicit-consent gating beyond the base set.
6. **Release documentation:** README (install, first-run, troubleshooting), testing summary, known-limitations page listing untested hardware matrix cells.
7. **Reproducibility:** pinned toolchains, locked dependencies, build scripts in `scripts/`; artifact hash manifest published alongside releases.
8. **Final acceptance sweep:** execute §28 checklist 1–20 as a formal sign-off record.

### 13.5 Exit criteria

- [ ] Clean-machine installation passes on Windows 10 and Windows 11
- [ ] Signed artifacts validate; hashes reproducible from build scripts
- [ ] License notices complete; FFmpeg obligations documented
- [ ] Diagnostic export contains no media, credentials, or raw paths
- [ ] All 20 MVP acceptance criteria checked with recorded evidence

### 13.6 Current evidence

- [x] User-triggered JSON diagnostic export is implemented in the desktop
  shell and covered by a redaction/no-media test.
- [x] Build metadata includes version, explicit CI provenance fields, target,
  OS, GPU/driver probes, WGC status, encoder capabilities, and FFmpeg tool
  availability.
- [x] Tauri production CSP, NSIS current-user scope, WebView2 bootstrap policy,
  license metadata, release preflight, and signed-build/hash scripts are present.
- [ ] Strict release preflight remains blocked by missing signing, packaging,
  provenance, license approval, and media-validation inputs in this environment.
- [ ] Clean-machine install, signed artifact verification, update strategy, and
  full §28 sign-off remain open.

---

## 14. Requirement traceability matrix

Requirements from spec §8 grouped by implementing segment (V = implemented, T = verified by tests in that segment).

| Group | IDs | Implemented in | Verified in |
|---|---|---|---|
| APP | 001, 002, 005, 007 | S0 | S0 |
| APP | 003, 004 | S6/S7 | S7 |
| APP | 006 | S7 (settings) | S7/S9 |
| VID | 001–009 | S2 (+S3 for 006 epoch handling, S8 recovery) | S2, S3, S8 |
| VID | 010 | Deferred (post-MVP) | — |
| VID | 011 | Enforced by design review S2 | S2 |
| AUD | 001–007, 009, 010 | S4 | S4 |
| AUD | 008 | S4 (ADR if deferred) | S4/S5 |
| AUD | 011 | Deferred (post-MVP) | — |
| ENC | 001–010 | S3 | S3, S8 |
| ENC | 011 | Deferred (post-MVP) | — |
| BUF | 001–013 | S1 (+S5 end-to-end, S8 resilience) | S1, S5, S8 |
| MUX | 001–011 | S5 | S5 |
| KEY | 001–006 | S6 | S6 |
| NOT | 001–004 | S6 | S6 |
| NOT | 005 | MAY — deferred | — |
| LIB | 001–010 | S7 | S7 |
| STO | 001, 004–008 | S7 | S7 |
| STO | 002, 003 | S5 | S5, S8 |
| PERF | 001–008 | Cross-cutting | S8 (targets finalized post-S3 prototype) |
| §24 Privacy/security | all | S0 (IPC allowlist), S5 (redaction), S7, S9 | S9 |

---

## 15. Verification command reference

Standard gates run at every segment close (exact invocations recorded in task reports per spec §30 rule 7):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/desktop && npm ci && npm run typecheck && npm run lint && npm run build
```

Media validation (S5 onward):

```bash
ffprobe -v error -show_streams -show_format <clip.mp4>
ffmpeg -v error -i <clip.mp4> -f null -
```

Endurance (S8 onward): long-running tagged suite in `tests/endurance`.

---

## 16. Risk register and open-decision checkpoints

| # | Risk / open decision (spec §33) | Owner segment | Mitigation |
|---|---|---|---|
| 1 | FFmpeg vs Media Foundation first encoder | S3 | Prototype both minimal paths; decide via ADR before S3 start |
| 2 | Direct MP4 container authoring | S5 | Direct pure-Rust MP4 single-pass muxing with trailing moov (ADR 0021) |
| 3 | In-memory vs disk-assisted buffering | S1 | Start in-memory with §15.5 bounds; revisit only if 120 s @ max bitrate breaches budget |
| 4 | Single process vs recorder service | S0 | MVP: single process with crash-isolated workers; service split deferred |
| 5 | HW encoder ranking specifics | S3 | Capability-probe results drive order; logged diagnostically |
| 6 | Max supported replay duration | S1 | Bound by memory formula + UI pre-apply warning |
| 7 | A/V drift tolerance | S4 | Define measurable tolerance in ADR before S4 exit |
| 8 | Default quality preset/bitrate | S3 | Benchmarks on tier hardware; defaults conservative |
| 9 | Separate audio tracks in first MVP | S4/S5 | If container/toolchain friction, ship mixed stereo + ADR |
| 10 | Thumbnail/playback implementation | S7 | Prefer external-player-safe approach; embedded player optional |
| 11 | Update framework | S9 | ADR 0014 records that updates are deferred; future updater requires a new signed-updater ADR |
| 12 | Minimum Windows build | S2 | WGC feature availability survey; document floor early |

Additional standing risks:

- **Hardware variance:** every segment's hardware-matrix coverage is explicitly reported as tested/untested (spec §30 rules 8–9).
- **Hotkey reach into elevated apps:** documented limitation if encountered in S6.
- **FFmpeg licensing:** blocking checkpoint before any FFmpeg linkage compiles into shipped artifacts; GPL components require explicit approval (spec §30 rule 14).

---

## 17. Document maintenance

This plan is updated when:

- A milestone exits (mark segment status),
- An ADR changes a segment's scope,
- Acceptance criteria or requirements change upstream in `spec.md`.

Status column convention in §2: `Not started / In progress / Exited (date)` with an evidence link to the segment progress log.
