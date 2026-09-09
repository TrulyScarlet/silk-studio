# Silk - Instant Replay Clipping Application  
## Product and Technical Specification

**Document status:** Initial specification  
**Target release:** MVP  
**Initial platform:** Windows 10 and Windows 11  
**Implementation approach:** Independent implementation  
**Reference products:** OBS Studio Replay Buffer, NVIDIA ShadowPlay, AMD ReLive, Xbox Game Bar, Medal, and SteelSeries Moments

---

## 1. Purpose

This document specifies a Windows-first desktop application that continuously captures video and audio into a bounded replay buffer. When the user activates a global hotkey, the application saves the previous configurable period as a video clip.

The application will focus specifically on fast, reliable clipping rather than streaming, scene composition, or full video production.

OBS Studio may be studied as a behavioral and performance reference. The application must not copy, translate, adapt, or link against OBS source code.

---

## 2. Product vision

The product should provide the reliability and capture quality users expect from established recording software while remaining:

- Easier to configure
- Faster to launch
- Less resource-intensive
- Focused entirely on instant replay and clip management
- Reliable during long gaming or desktop sessions
- Functional without an online account
- Privacy-conscious and local-first

The core user interaction is:

> Press a hotkey and save what just happened.

---

## 3. Goals

### 3.1 MVP goals

The MVP must:

1. Capture a selected Windows display.
2. Capture desktop audio.
3. Optionally capture a microphone.
4. Encode video using hardware acceleration when available.
5. Continuously retain the latest configurable period.
6. Save the buffered period without interrupting capture.
7. Support a configurable global hotkey.
8. Produce standard, playable video files.
9. Notify the user after a clip is saved.
10. Provide a local library of saved clips.
11. Maintain bounded memory and disk usage.
12. Recover from common capture and device failures.
13. Operate without an account or internet connection.

### 3.2 Quality goals

The product should:

- Start capture within five seconds under normal conditions.
- Save a replay without visible interruption.
- Maintain synchronized audio and video.
- Avoid unbounded memory growth.
- Prefer GPU encoding to minimize CPU usage.
- Clearly communicate capture or encoder failures.
- Preserve clips if the UI crashes while the recorder remains operational.
- Produce output that is playable in common media players.

### 3.3 Long-term goals

Potential later releases may support:

- Window capture
- Game-specific profiles
- Automatic game detection
- Multiple audio tracks
- Clip trimming without re-encoding
- Vertical-video export
- HDR
- Automatic highlights
- OCR and speech transcription
- Optional cloud upload
- macOS and Linux
- Hardware-specific game capture

These are not MVP requirements unless explicitly promoted through a future specification.

---

## 4. Non-goals

The MVP will not provide:

- Live streaming
- Scene composition
- Browser sources
- Video transitions
- Plugin compatibility with OBS
- Advanced video editing
- Cloud accounts
- Mandatory telemetry
- Injected graphics hooks
- Anti-cheat-sensitive process injection
- Full OBS feature parity
- Mobile clients
- Webcam overlays
- Automated highlight detection
- HDR recording
- Multi-display compositing

The application is not intended to replace OBS as a streaming or production tool.

---

## 5. Reference-use policy

### 5.1 Permitted reference use

OBS Studio and other products may be used to evaluate:

- Expected replay-buffer behavior
- User-facing settings
- Capture reliability
- Hotkey behavior
- Save latency
- Recovery behavior
- Performance characteristics
- Error reporting
- Output compatibility
- General usability

Official operating-system, hardware-vendor, codec, and library documentation should be the primary technical references.

### 5.2 Prohibited use

Contributors and AI agents must not:

- Copy or translate OBS source code
- Adapt OBS classes or functions line-by-line
- Link against `libobs`
- Import OBS modules
- Copy OBS’s internal identifiers or module structure
- Copy OBS assets, icons, themes, branding, or UI layouts
- Copy comments or documentation from OBS
- Incorporate code from OBS issues or pull requests
- Introduce GPL dependencies without explicit approval
- Claim compatibility or affiliation with OBS

### 5.3 Originality requirement

Every component must have:

- An independently documented purpose
- An original interface
- Project-specific naming
- Tests based on this specification
- Dependency and licensing records

The repository should contain an `AGENTS.md` file that states these restrictions.

---

## 6. Target users

### 6.1 Primary users

- PC gamers who want to save recent gameplay
- Creators who need quick source clips
- Users who find OBS excessive for simple clipping
- Users who prefer local recording over cloud-managed platforms

### 6.2 Secondary users

- Software testers recording recent bugs
- Educators capturing recent demonstrations
- Desktop users who need retroactive screen recording
- Developers recording reproduction steps

---

## 7. Core user stories

### 7.1 Initial setup

As a new user, I want to choose a display, audio source, clip duration, quality, and hotkey so that I can begin clipping quickly.

### 7.2 Start replay capture

As a user, I want replay capture to start from the main window or system tray so that recent activity begins being retained.

### 7.3 Save a clip

As a user, I want to press a global hotkey and save the previous \(N\) seconds without interrupting ongoing capture.

### 7.4 Receive confirmation

As a user, I want a notification containing the clip name and save status so that I know the action succeeded.

### 7.5 Browse clips

As a user, I want to browse, play, rename, delete, and reveal clips in File Explorer.

### 7.6 Change clip duration

As a user, I want to configure replay duration based on available memory and desired context.

### 7.7 Configure audio

As a user, I want to configure multiple independent audio sources, including
desktop sound and an optional microphone, with stable names and levels.

### 7.8 Handle failures

As a user, I want clear information when a display, audio device, or encoder becomes unavailable.

### 7.9 Control storage

As a user, I want automatic storage limits so that old clips do not fill my disk.

### 7.10 Preserve privacy

As a user, I want capture to remain local and to know when recording is active.

---

## 8. MVP functional requirements

Requirements use these priorities:

- **MUST:** Required for MVP
- **SHOULD:** Expected unless technically blocked
- **MAY:** Optional enhancement

### 8.1 Application lifecycle

- **APP-001 — MUST:** The application must run on supported Windows 10 and Windows 11 versions.
- **APP-002 — MUST:** The application must expose start, stop, and save-replay actions.
- **APP-003 — MUST:** Closing the main window must optionally minimize the application to the system tray.
- **APP-004 — MUST:** The tray menu must show current capture status.
- **APP-005 — MUST:** The application must prevent accidental duplicate recorder instances.
- **APP-006 — SHOULD:** The application should optionally start with Windows.
- **APP-007 — SHOULD:** The recorder core should remain independent of the desktop UI.

### 8.2 Video capture

- **VID-001 — MUST:** The application must capture one selected display.
- **VID-002 — MUST:** Capture must use an official Windows capture API.
- **VID-003 — MUST:** The application must support 1920×1080 at 60 FPS when the system is capable.
- **VID-004 — MUST:** The user must be able to select 30 or 60 FPS.
- **VID-005 — MUST:** The user must be able to select native, 1080p, or 720p output.
- **VID-006 — MUST:** The recorder must detect resolution changes.
- **VID-007 — MUST:** Capture must recover or stop cleanly when a display is disconnected.
- **VID-008 — MUST:** The pipeline must record frame timestamps.
- **VID-009 — SHOULD:** Duplicate frames should be avoided when the source is unchanged, if doing so does not cause compatibility issues.
- **VID-010 — MAY:** Window capture may be introduced after display capture is stable.
- **VID-011 — MUST NOT:** The MVP must not inject code into games or other processes.

### 8.3 Audio capture

- **AUD-001 — MUST:** The application must capture desktop audio through WASAPI loopback.
- **AUD-002 — MUST:** The application must optionally capture a microphone.
- **AUD-003 — MUST:** Each configured audio track must be independently enabled.
- **AUD-004 — MUST:** The user must be able to select the device for each configured source.
- **AUD-005 — MUST:** The recorder must handle an unavailable microphone without losing desktop audio capture.
- **AUD-006 — MUST:** Audio timestamps must be synchronized to the recorder timeline.
- **AUD-007 — MUST:** Audio must be resampled when required by the selected encoder.
- **AUD-008 — SHOULD:** Configured audio sources should be stored in separate, ordered tracks.
- **AUD-009 — SHOULD:** Basic bounded linear gain control should be supported for each configured audio track.
- **AUD-010 — SHOULD:** The recorder should recover after an audio-device change.
- **AUD-011 — MAY:** Noise suppression may be added after MVP.

### 8.4 Encoding

- **ENC-001 — MUST:** The application must support H.264 output.
- **ENC-002 — MUST:** Hardware encoding must be preferred when available.
- **ENC-003 — MUST:** The application must detect encoder initialization failure.
- **ENC-004 — MUST:** A fallback path must be available when the preferred encoder fails.
- **ENC-005 — MUST:** The encoder must use a configurable or internally selected keyframe interval.
- **ENC-006 — SHOULD:** The default keyframe interval should be approximately two seconds.
- **ENC-007 — MUST:** The encoder must expose dropped-frame and overload information.
- **ENC-008 — MUST:** The user must be able to choose a quality preset or configure a custom target bitrate.
- **ENC-009 — SHOULD:** Video bitrate is calculated automatically based on resolution and frame rate presets or specified directly by the user (configured via `MF_MT_AVG_BITRATE` on supported encoders, requiring hardware validation).
- **ENC-010 — SHOULD:** The application should support NVIDIA, AMD, and Intel hardware where available.
- **ENC-011 — MAY:** HEVC and AV1 may be added later.

### 8.5 Replay buffer

- **BUF-001 — MUST:** The application must continuously retain recently encoded packets.
- **BUF-002 — MUST:** The replay duration must be configurable.
- **BUF-003 — MUST:** The supported MVP range must include 15–120 seconds.
- **BUF-004 — MUST:** The buffer must have a strict memory or storage bound.
- **BUF-005 — MUST:** Old packets must be evicted based on timestamps.
- **BUF-006 — MUST:** Saved video must begin at or before the requested start time on a valid keyframe.
- **BUF-007 — MUST:** Saving a clip must not stop packet ingestion.
- **BUF-008 — MUST:** Multiple save requests must be serialized or safely processed concurrently.
- **BUF-009 — MUST:** Snapshot packets must remain valid while the live buffer continues evicting data.
- **BUF-010 — MUST:** Saved packet timestamps must be normalized.
- **BUF-011 — MUST:** Audio must be trimmed or aligned to the selected video interval.
- **BUF-012 — MUST:** Changing replay duration must not require an application restart.
- **BUF-013 — SHOULD:** The application should support pre-roll variance caused by keyframe alignment and report the actual clip length.

### 8.6 Clip saving and muxing

- **MUX-001 — MUST:** Clips must be saved in a standard ISO Base Media File Format (MP4) container.
- **MUX-002 — MUST:** Direct MP4 muxing must write H.264 video and AAC audio directly without intermediate container files or secondary re-encoding.
- **MUX-003 — MUST:** The MP4 muxer must write top-level boxes sequentially (`ftyp -> mdat -> moov`) with a trailing `moov` metadata box for single-pass disk I/O, supporting O(1) local playback seeking without network buffering.
- **MUX-004 — MUST:** Multi-track audio (e.g. desktop audio and microphone) must be written as separate, independent AAC audio tracks in the MP4 container.
- **MUX-005 — MUST:** Incomplete output files must not appear as successfully saved clips.
- **MUX-006 — MUST:** Temporary files must use a distinct extension or temporary directory.
- **MUX-007 — MUST:** The application must use an atomic finalization strategy when feasible.
- **MUX-008 — MUST:** The output directory must be configurable.
- **MUX-009 — MUST:** File names must avoid invalid Windows characters and collisions.
- **MUX-010 — SHOULD:** The default file name should include date, time, and source name.
- **MUX-011 — MUST:** The application must report save failures with actionable messages.
- **MUX-012 — MUST:** The user-selectable output base must contain a managed
  `Silk` directory; new game-attributed clips must be saved below a sanitized
  game subdirectory without scanning outside that base.

Example naming pattern:

```text
Clip_2026-08-26_01-00-05_GameName.mp4
```

### 8.7 Global hotkeys

- **KEY-001 — MUST:** The user must be able to configure a global save-replay hotkey.
- **KEY-002 — MUST:** Hotkey conflicts must be detected when possible.
- **KEY-003 — MUST:** Hotkeys must function while the main window is unfocused.
- **KEY-004 — MUST:** Repeated hotkey activation must not corrupt output.
- **KEY-005 — SHOULD:** Separate hotkeys should exist for start/stop capture.
- **KEY-006 — MUST:** The UI must clearly display the active hotkey.

### 8.8 Notifications

- **NOT-001 — MUST:** Successful saves must produce a desktop notification or in-app notification.
- **NOT-002 — MUST:** Save failures must produce an error notification.
- **NOT-003 — SHOULD:** Notifications should include an action to open the clip.
- **NOT-004 — SHOULD:** Notifications should include an action to reveal the clip in File Explorer.
- **NOT-005 — MAY:** Notification sounds may be configurable.

### 8.9 Clip library

- **LIB-001 — MUST:** The application must display locally saved clips.
- **LIB-002 — MUST:** Each clip entry must show name, creation time, duration, and file size.
- **LIB-003 — MUST:** The user must be able to play a clip.
- **LIB-004 — MUST:** The user must be able to rename a clip.
- **LIB-005 — MUST:** The user must be able to delete a clip with confirmation.
- **LIB-006 — MUST:** The user must be able to reveal a clip in File Explorer.
- **LIB-007 — SHOULD:** The library should generate thumbnails asynchronously.
- **LIB-008 — SHOULD:** The user should be able to sort by date, duration, size, or name.
- **LIB-009 — SHOULD:** The library should remain responsive with at least 1,000 clip records.
- **LIB-010 — MUST:** Missing files must be detected and represented correctly.
- **LIB-011 — MUST:** The library must expose the game attribution associated
  with a clip when available and allow filtering by game, with an
  `Uncategorized` fallback.

### 8.10 Storage management

- **STO-001 — MUST:** The application must display the current output directory.
- **STO-002 — MUST:** The application must check available disk space before saving.
- **STO-003 — MUST:** Low-disk-space failures must not crash the recorder.
- **STO-004 — SHOULD:** The user should be able to configure a storage quota.
- **STO-005 — SHOULD:** Automatic deletion should delete the oldest eligible clips first.
- **STO-006 — MUST:** Automatic deletion must be disabled by default unless clearly disclosed during setup.
- **STO-007 — SHOULD:** Users should be able to protect clips from automatic deletion.
- **STO-008 — MUST:** Temporary files must be cleaned up after failure or on the next launch.

### 8.11 Settings

The MVP settings interface must include:

- Capture display
- Frame rate
- Output resolution
- Quality preset or bitrate
- Replay duration
- Desktop audio enablement
- Microphone enablement and device
- Encoder selection or automatic mode
- Output container
- Output directory
- Global hotkey
- Startup behavior
- Notification preferences
- Storage quota settings
- Diagnostic logging level

Settings must be validated before being applied.

---

## 9. User experience specification

### 9.1 First-run flow

On first launch:

1. Display a brief explanation of the replay buffer.
2. Request or validate necessary Windows permissions.
3. Select a display.
4. Select desktop audio and optional microphone.
5. Select clip duration.
6. Configure the save hotkey.
7. Choose an output directory.
8. Run an encoder capability check.
9. Offer a short test recording.
10. Present the ready state.

The user should be able to skip optional steps.

### 9.2 Main window

The main window should contain:

- Capture state
- Active source
- Replay duration
- Encoder
- Audio-level indicators
- Buffer readiness
- Save Replay button
- Start/Stop button
- Recent clips
- Performance warnings
- Settings access

Suggested recorder states:

```text
Stopped
Starting
Buffering
Ready
Saving
Degraded
Recovering
Error
Stopping
```

“Buffering” means capture is active but a full replay-duration history is not yet available.

### 9.3 Tray behavior

The tray menu should provide:

- Current status
- Save Replay
- Start/Stop Capture
- Open Application
- Open Clip Folder
- Settings
- Exit

The tray icon should visually distinguish stopped, active, and error states.

### 9.4 Clip save behavior

When the hotkey is pressed:

1. The application acknowledges the request immediately.
2. The recorder creates a replay snapshot.
3. The live buffer continues receiving packets.
4. A worker writes and finalizes the output.
5. The clip is validated.
6. The library is updated.
7. The user receives a success notification.

If saving fails, the application must retain diagnostic details and continue capture when possible.

---

## 10. High-level architecture

The application will use a native recorder core and a separate desktop interface.

```text
┌────────────────────────────────────────────┐
│               Desktop UI                   │
│          Tauri + React/TypeScript           │
└───────────────────┬────────────────────────┘
                    │ Commands and events
┌───────────────────▼────────────────────────┐
│          Application Controller             │
│ State, settings, validation, notifications  │
└───────────────────┬────────────────────────┘
                    │
┌───────────────────▼────────────────────────┐
│             Recorder Engine                 │
│ Capture, sync, encode, buffer, save          │
└───────┬───────────┬───────────┬────────────┘
        │           │           │
┌───────▼──────┐ ┌──▼────────┐ ┌▼─────────────┐
│ Video Capture│ │Audio Capture│ │Encoder Layer │
│ WGC + D3D11  │ │   WASAPI    │ │HW + fallback│
└───────┬──────┘ └────┬────────┘ └──────┬──────┘
        └──────────────┴─────────────────┘
                       │
               ┌────────▼──────────┐
               │ Replay Buffer      │
               │ Encoded packets    │
               └────────┬──────────┘
                        │ Snapshot
               ┌────────▼──────────┐
               │ MP4 Mux Workers   │
               └────────┬──────────┘
                        │
               ┌────────▼──────────┐
               │ Clip Library       │
               │ Files + metadata   │
               └───────────────────┘
```

---

## 11. Technology choices

### 11.1 Native core

Preferred language: **Rust**

Reasons:

- Memory safety
- Strong concurrency primitives
- Explicit ownership suitable for packet lifetimes
- Good native interoperability
- Suitable for a long-running recorder process

C++ may be used only where required by an SDK or where the Rust integration is impractical. Such use must be isolated behind a safe interface.

### 11.2 Desktop UI

Preferred stack:

- Tauri
- React
- TypeScript
- A small, locally bundled UI component system

The UI must not own the real-time media pipeline.

### 11.3 Windows APIs

Preferred APIs:

- Windows Graphics Capture for display capture
- Direct3D 11 for GPU textures
- WASAPI loopback for desktop audio
- WASAPI capture for microphone input
- Win32 hotkey APIs
- Native Windows notifications where practical

### 11.4 Media libraries

Direct MP4 container authoring is implemented in pure Rust using the `mp4` crate. Video and audio encoding utilize native Windows Media Foundation APIs.

FFmpeg tooling (such as `ffmpeg` and `ffprobe`) is standalone optional tooling used for development-time and CI media validation and optional external export workflows; it is not linked or bundled into the desktop product.

All third-party components and build options must undergo license review.

### 11.5 Configuration and metadata

- Human-readable configuration: JSON
- Clip metadata and indexing: SQLite
- Structured diagnostics: JSON Lines or structured text logs

Secrets are not expected in the MVP. If introduced later, they must not be stored in plain-text configuration.

---

## 12. Proposed repository structure

```text
/
├─ AGENTS.md
├─ README.md
├─ LICENSE
├─ THIRD_PARTY_LICENSES.md
├─ Cargo.toml
├─ apps/
│  └─ desktop/
│     ├─ src/
│     ├─ src-tauri/
│     └─ package.json
├─ crates/
│  ├─ app-controller/
│  ├─ recorder-engine/
│  ├─ capture-api/
│  ├─ capture-windows/
│  ├─ audio-api/
│  ├─ audio-wasapi/
│  ├─ encoder-api/
│  ├─ encoder-ffmpeg/
│  ├─ media-types/
│  ├─ media-clock/
│  ├─ replay-buffer/
│  ├─ muxer/
│  ├─ hotkeys/
│  ├─ clip-library/
│  ├─ configuration/
│  ├─ diagnostics/
│  └─ test-support/
├─ docs/
│  ├─ architecture/
│  ├─ decisions/
│  ├─ testing/
│  └─ licensing/
├─ scripts/
└─ tests/
   ├─ integration/
   ├─ endurance/
   └─ fixtures/
```

---

## 13. Component responsibilities

### 13.1 Application controller

Responsibilities:

- Own application state
- Validate settings
- Start and stop the recorder
- Process UI commands
- Forward recorder events
- Coordinate notifications
- Manage save jobs
- Prevent invalid state transitions

The controller must not process media frames directly.

### 13.2 Video-capture component

Responsibilities:

- Enumerate displays
- Start and stop capture
- Produce GPU-backed video frames
- Assign source timestamps
- Report resolution and format changes
- Detect source loss
- Reinitialize capture when safe

Conceptual interface:

```rust
pub trait VideoCapture: Send {
    fn enumerate_sources(&self) -> Result<Vec<VideoSourceInfo>>;
    fn start(&mut self, config: VideoCaptureConfig) -> Result<()>;
    fn next_event(&mut self) -> Result<VideoCaptureEvent>;
    fn stop(&mut self) -> Result<()>;
}
```

### 13.3 Audio-capture component

Responsibilities:

- Enumerate audio devices
- Capture desktop loopback
- Capture optional microphone input
- Provide timestamps
- Detect device changes
- Expose audio format information

```rust
pub trait AudioCapture: Send {
    fn enumerate_devices(&self) -> Result<Vec<AudioDeviceInfo>>;
    fn start(&mut self, config: AudioCaptureConfig) -> Result<()>;
    fn next_event(&mut self) -> Result<AudioCaptureEvent>;
    fn stop(&mut self) -> Result<()>;
}
```

### 13.4 Media clock

The media clock provides the shared timeline for:

- Video frame timestamps
- Desktop audio
- Microphone audio
- Encoder packets
- Replay-buffer eviction
- Clip boundaries

Requirements:

- Use a monotonic clock.
- Never use wall-clock time for packet ordering.
- Store wall-clock time only as clip metadata.
- Convert each source timestamp into a shared time base.
- Detect large timestamp discontinuities.

### 13.5 Synchronization component

Responsibilities:

- Convert source timestamps
- Maintain monotonic ordering
- Compensate for initial source offsets
- Detect excessive drift
- Resample audio when necessary
- Emit synchronization metrics

The MVP should favor continuous audio and stable timestamps over aggressive frame insertion.

### 13.6 Encoder abstraction

```rust
pub trait VideoEncoder: Send {
    fn capabilities(&self) -> EncoderCapabilities;
    fn configure(&mut self, config: VideoEncoderConfig) -> Result<()>;
    fn encode(&mut self, frame: VideoFrame) -> Result<Vec<EncodedPacket>>;
    fn drain(&mut self) -> Result<Vec<EncodedPacket>>;
}

pub trait AudioEncoder: Send {
    fn configure(&mut self, config: AudioEncoderConfig) -> Result<()>;
    fn encode(&mut self, frame: AudioFrame) -> Result<Vec<EncodedPacket>>;
    fn drain(&mut self) -> Result<Vec<EncodedPacket>>;
}
```

The abstraction must not expose backend-specific types outside its crate.

### 13.7 Replay buffer

The replay buffer stores encoded packets, not raw video frames.

A conceptual packet model:

```rust
pub struct EncodedPacket {
    pub stream_id: StreamId,
    pub media_type: MediaType,
    pub pts: i64,
    pub dts: i64,
    pub duration: i64,
    pub time_base: TimeBase,
    pub is_keyframe: bool,
    pub sequence: u64,
    pub payload: PacketPayload,
}
```

The implementation should use reference-counted or otherwise shareable immutable payloads so snapshots do not require unnecessary deep copies.

### 13.8 Muxer

Responsibilities:

- Receive an immutable replay snapshot
- Create a temporary output file (`.part`)
- Transform H.264 Annex B streams to AVCC format and embed SPS/PPS parameter sets in track header (`avcC`)
- Write independent AAC audio tracks for desktop and microphone streams
- Write sequential `ftyp -> mdat -> moov` boxes placing `moov` metadata at file end
- Normalize timestamps to a zero origin
- Interleave video and audio packets by DTS
- Validate structural integrity before publication
- Atomically publish the final MP4 file via same-volume rename
- Return clip metadata

### 13.9 Clip library

Responsibilities:

- Index completed files
- Store clip metadata
- Detect missing files
- Generate thumbnails
- Support rename and deletion
- Enforce optional storage policies

The database is an index, not the authoritative media store. The file system remains the source of truth for clip existence.

---

## 14. Threading and concurrency model

Real-time capture must not wait for disk I/O, thumbnail generation, or UI work.

Recommended logical workers:

1. Video-capture worker
2. Desktop-audio worker
3. Microphone worker
4. Synchronization/dispatch worker
5. Video-encoding worker
6. Audio-encoding worker
7. Replay-buffer coordinator
8. Clip-save worker pool
9. Library-index worker
10. UI event bridge

Communication should use bounded channels.

### 14.1 Backpressure policy

When a downstream component cannot keep up:

- The system must emit overload metrics.
- Video frames may be dropped according to a documented policy.
- Audio should be preserved whenever possible.
- Capture threads must not block indefinitely.
- The replay buffer must remain bounded.
- Disk save workers must not block encoding.
- The system should reduce workload or enter a degraded state before crashing.

Suggested overload order:

1. Skip preview updates.
2. Delay thumbnail generation.
3. Reduce nonessential diagnostics.
4. Drop late video frames.
5. Report encoder overload.
6. Stop capture cleanly if safe operation is no longer possible.

### 14.2 Locking rules

- Do not hold a replay-buffer write lock during file output.
- Do not call UI code while holding media locks.
- Avoid shared mutable packet payloads.
- Keep critical sections short.
- Document thread ownership for platform resources.
- Use lock-order documentation if multiple locks are unavoidable.

---

## 15. Replay-buffer algorithm

### 15.1 Packet insertion

For every encoded packet:

1. Validate its stream and timestamps.
2. Assign a monotonically increasing sequence number.
3. Insert it into stream-aware storage.
4. Update the newest timeline position.
5. Evict data older than the configured retention boundary.
6. Preserve any required keyframe preceding that boundary.
7. Update memory and duration metrics.

### 15.2 Eviction

If the newest video timestamp is \(T_n\) and configured replay duration is
\(D\), the target retention boundary is:

$$
T_b = T_n - D
$$

The buffer must retain the most recent video keyframe at or before \(T_b\).
Packets that are no longer needed by either the live buffer or an active
snapshot may be released.

### 15.3 Snapshot creation

When a save is requested at timeline time \(T_s\):

1. Compute requested start \(T_r = T_s - D\).
2. Find the latest video keyframe with timestamp at or before \(T_r\).
3. Select packets from that keyframe through the snapshot end.
4. Include audio packets that overlap the selected video interval.
5. Capture codec parameters and stream metadata.
6. Create immutable references to selected packets.
7. Release the live-buffer lock.
8. Submit the snapshot to a mux worker.

### 15.4 Timestamp normalization

The saved clip’s timestamps should begin at zero or another container-valid
origin.

For a packet timestamp \(T_p\) and selected origin \(T_o\):

$$
T'_p = T_p - T_o
$$

The muxer must preserve valid decoding and presentation order, including
distinct DTS and PTS values where codecs require them.

### 15.5 Memory estimation

At a total encoded bitrate of \(B\) bits per second and duration \(D\)
seconds, approximate payload memory is:

$$
M \approx \frac{B \times D}{8}
$$

At 20 Mbps for 60 seconds:

$$
M \approx \frac{20{,}000{,}000 \times 60}{8}
= 150{,}000{,}000 \text{ bytes}
$$

Actual memory is higher because of audio, packet metadata, allocator
overhead, and snapshot references.

The UI should estimate memory use before applying extreme quality or
duration combinations.

---

## 16. Encoder selection strategy

### 16.1 Capability discovery

At startup or when settings change:

1. Enumerate supported encoder backends.
2. Validate codec and resolution support.
3. Test encoder initialization.
4. Rank viable options.
5. Store a diagnostic explanation of the selected backend.

### 16.2 Automatic ranking

The initial ranking should be:

1. Compatible hardware encoder
2. Alternate compatible hardware encoder
3. Software H.264 fallback

The exact ordering among hardware backends should account for the installed
GPU and tested compatibility.

### 16.3 Fallback behavior

If the active encoder fails:

- Stop accepting new frames temporarily.
- Attempt a controlled reinitialization.
- If reinitialization fails, attempt the next compatible encoder.
- If codec parameters change, start a new replay-buffer epoch.
- Do not combine incompatible packet configurations in one clip.
- Notify the user if capture quality or performance changes.

---

## 17. Format and container strategy

### 17.1 Video

MVP codec:

- H.264/AVC

Default pixel format:

- 8-bit 4:2:0 where supported

Video access units are converted from Annex B to AVCC sample format. Video bitrate is configured through encoder properties (such as `MF_MT_AVG_BITRATE` in Media Foundation) and requires hardware validation across target GPU architectures.

### 17.2 Audio

Preferred MVP codec:

- AAC

Preferred sample rate:

- 48 kHz

Configured audio sources (e.g. desktop loopback and microphone) are stored as separate, ordered AAC tracks in the MP4 container.

### 17.3 Container

Standard MP4 (ISO/IEC 14496-14):

- Pure-Rust single-pass direct MP4 muxer.
- Sequential top-level box layout: `ftyp -> mdat -> moov` with trailing `moov` movie metadata box for single-pass disk I/O.
- Direct output eliminates intermediate files, background remuxing passes, and post-save transcoding quality loss.
- Local media players seek directly to the trailing `moov` box.

Workflow:

```text
Replay snapshot
    -> temporary staged MP4 (.part)
    -> container & timestamp validation
    -> atomic publication (.mp4)
```

---

## 18. Configuration model

Example versioned configuration:

```json
{
  "version": 4,
  "settings": {
    "capture": {
      "sourceType": "display",
      "sourceId": "display-1",
      "frameRate": 60,
      "outputResolution": "1920x1080"
    },
    "audio": {
      "tracks": [
        {
          "id": "desktop",
          "name": "Desktop",
          "enabled": true,
          "sourceKind": "output_loopback",
          "deviceId": null,
          "gain": 1.0
        },
        {
          "id": "microphone",
          "name": "Microphone",
          "enabled": false,
          "sourceKind": "input",
          "deviceId": null,
          "gain": 1.0
        }
      ]
    },
    "encoding": {
      "videoCodec": "h264",
      "encoder": "auto",
      "qualityPreset": "high",
      "videoBitrateKbps": null,
      "keyframeIntervalSeconds": 2.0,
      "audioCodec": "aac",
      "audioBitrateKbps": 192
    },
    "replay": {
      "durationSeconds": 60
    },
    "output": {
      "directory": "C:\\Users\\User\\Videos",
      "container": "mp4",
      "fileNamePattern": "Clip_{date}_{time}_{source}"
    },
    "hotkeys": {
      "saveReplay": "Ctrl+Shift+F10",
      "startCapture": null,
      "stopCapture": null
    },
    "storage": {
      "quotaEnabled": false,
      "quotaGigabytes": 100,
      "automaticDeletionEnabled": false
    },
    "application": {
      "startWithWindows": false,
      "minimizeToTray": true,
      "notificationsEnabled": true
    },
    "diagnostics": {
      "loggingLevel": "info"
    }
  }
}
```
    },
    "hotkeys": {
      "saveReplay": "Ctrl+Shift+F10",
      "startCapture": null,
      "stopCapture": null
    },
    "storage": {
      "quotaEnabled": false,
      "quotaGigabytes": 100,
      "automaticDeletionEnabled": false
    },
    "application": {
      "startWithWindows": false,
      "minimizeToTray": true,
      "notificationsEnabled": true
    },
    "diagnostics": {
      "loggingLevel": "info"
    }
  }
}
```

Configuration migrations must be versioned.

---

## 19. State model

The recorder must use explicit transitions.

```text
Stopped -> Starting -> Buffering -> Ready
Ready -> Saving -> Ready
Ready -> Recovering -> Ready
Starting -> Error
Buffering -> Error
Ready -> Error
Error -> Starting
Any active state -> Stopping -> Stopped
```

Saving should preferably be represented as a concurrent job rather than a
state that prevents capture. The UI may display “Saving” while the recorder
internally remains ready.

Invalid transitions must return structured errors.

---

## 20. Error handling

Errors must contain:

- Stable error code
- Human-readable message
- Component
- Severity
- Recoverability
- Suggested action
- Underlying platform or library error
- Timestamp
- Relevant configuration context with private data removed

Example error categories:

```text
CAPTURE_SOURCE_UNAVAILABLE
CAPTURE_PERMISSION_DENIED
CAPTURE_DEVICE_LOST
AUDIO_DEVICE_UNAVAILABLE
AUDIO_FORMAT_UNSUPPORTED
ENCODER_NOT_AVAILABLE
ENCODER_INITIALIZATION_FAILED
ENCODER_OVERLOADED
BUFFER_NOT_READY
OUTPUT_DIRECTORY_UNAVAILABLE
INSUFFICIENT_DISK_SPACE
MUXER_INITIALIZATION_FAILED
MUXER_WRITE_FAILED
OUTPUT_VALIDATION_FAILED
HOTKEY_REGISTRATION_FAILED
```

The UI must not display raw internal errors without a user-friendly
explanation.

---

## 21. Recovery behavior

### 21.1 Display changes

When resolution changes:

1. Pause submission of incompatible frames.
2. Flush or close the current encoder epoch.
3. Reconfigure capture surfaces.
4. Reinitialize the encoder if required.
5. Clear or segment incompatible replay data.
6. Resume buffering.
7. Inform the UI that the buffer is rebuilding.

### 21.2 Display disconnection

The application should:

- Enter a recovering state.
- Attempt to reacquire the source for a bounded period.
- Allow the user to choose another display.
- Stop cleanly if recovery fails.

### 21.3 Audio-device loss

The application should:

- Continue video capture.
- Mark the affected audio source unavailable.
- Attempt recovery.
- Avoid inserting malformed audio packets.
- Notify the user if a saved clip may lack an audio track.

### 21.4 Sleep and resume

After system resume:

- Recreate invalid capture resources.
- Reinitialize audio clients.
- Verify encoder state.
- Start a new buffer epoch.
- Avoid combining pre-sleep and post-resume discontinuities in one clip.

---

## 22. Diagnostics and observability

### 22.1 Required metrics

The application must track:

- Captured video FPS
- Encoded video FPS
- Dropped capture frames
- Dropped encoder frames
- Encoder latency
- Audio discontinuities
- A/V drift
- Replay-buffer duration
- Replay-buffer memory
- Save-job duration
- Mux failures
- Available disk space
- Capture restarts
- Active encoder backend

### 22.2 Logging

Logs must:

- Be structured
- Include timestamps and component names
- Rotate by age or size
- Avoid recording media content
- Avoid exposing usernames or file paths unnecessarily
- Support export as a diagnostic package

### 22.3 Diagnostic package

The user should be able to export:

- Application version
- Operating-system version
- GPU and driver information
- Encoder capabilities
- Relevant settings
- Recent redacted logs
- Recent error codes

No clip files, audio samples, screenshots, or credentials may be included
without explicit user consent.

---

## 23. Performance requirements

Test hardware tiers must be defined before release.

Initial targets on supported mid-range hardware:

- **PERF-001:** 1080p60 capture with hardware encoding
- **PERF-002:** Median save-request acknowledgment under 100 ms
- **PERF-003:** No capture interruption during replay save
- **PERF-004:** Stable memory use during an eight-hour session
- **PERF-005:** No sustained unbounded queue growth
- **PERF-006:** Main UI remains responsive during save operations
- **PERF-007:** CPU utilization remains substantially below software encoding under equivalent settings
- **PERF-008:** Replay-buffer memory stays within its calculated bound plus documented overhead

Exact CPU and GPU thresholds should be established after the first native
prototype.

---

## 24. Privacy and security requirements

- Capture must be local by default.
- No account may be required.
- No media may be uploaded without explicit user action.
- Telemetry must be opt-in if introduced.
- Capture status must be visible.
- Logs must not contain media payloads.
- File paths should be redacted from exported diagnostics when practical.
- Third-party update mechanisms must use signed packages.
- The application and installer must be code-signed for production.
- All dependency binaries must come from controlled, reproducible sources.
- Process injection is prohibited in the MVP.
- The UI must validate all paths and file names crossing the IPC boundary.
- Native commands exposed to the UI must use an allowlist.

---

## 25. Accessibility requirements

The UI should:

- Be fully keyboard navigable
- Provide visible focus states
- Support Windows display scaling
- Avoid conveying state through color alone
- Provide labels for icons and controls
- Respect reduced-motion preferences where feasible
- Maintain readable contrast
- Expose status changes to assistive technology where supported

---

## 26. Licensing requirements

Every dependency must be recorded with:

- Name
- Version
- Source URL
- License
- Static or dynamic linking mode
- Distributed artifacts
- Required notices
- Approval status

A dependency table should be maintained:

| Dependency | Purpose | License | Distribution | Approval |
|---|---|---:|---|---|
| Rust crates (including `mp4`, `bytes`) | Native implementation & MP4 muxing | MIT / Apache-2.0 | Linked | Pending review |
| Tauri | Desktop shell | MIT/Apache-2.0 | Bundled | Pending review |
| React | User interface | MIT | Bundled | Pending review |
| Windows APIs / Media Foundation | Platform integration & encoding | Platform terms | System-provided | Required |
| FFmpeg (standalone tooling) | Development-time & CI media validation | GPLv3 (approved Gyan build) | Standalone external tooling | Approved for validation tooling; not linked |

The project must not assume that every FFmpeg build is LGPL-compatible.
Build flags, codecs, and linked libraries determine obligations. FFmpeg
tooling remains strictly external and unlinked.

---

## 27. Testing strategy

### 27.1 Unit tests

Required unit coverage includes:

- Timestamp conversion
- Buffer insertion
- Duration-based eviction
- Keyframe retention
- Snapshot selection
- Timestamp normalization
- Stream interleaving
- Configuration validation
- File-name sanitization
- State transitions
- Storage-quota ordering
- Error conversion
- Encoder selection

### 27.2 Property tests

Property tests should verify:

- Buffer duration never exceeds bounds beyond allowed keyframe pre-roll.
- Packet ordering remains valid.
- Snapshot references remain valid after live eviction.
- Normalized timestamps do not precede allowed origins.
- Randomized save requests do not corrupt buffer state.

### 27.3 Synthetic pipeline tests

Before native capture, implement deterministic generators for:

- Video frames
- Keyframes
- Configured desktop and microphone audio sources
- Multiple independently configured audio tracks
- Timestamp jitter
- Resolution changes
- Device loss
- Encoder delay

The synthetic pipeline must support repeatable seeds.

### 27.4 Integration tests

Test:

- Display capture to output file
- Desktop audio to output file
- Combined A/V capture
- Multiple configured audio tracks with stable ordering and names
- Per-track device and gain settings
- Hotkey-triggered save
- Multiple consecutive saves
- Encoder fallback
- Output-directory failure
- Low-disk-space behavior
- Resolution changes
- Display removal
- Audio disconnection
- Sleep and resume

### 27.5 Media validation

Development and CI fixtures must be validated with tools equivalent to:

```bash
ffprobe -v error -show_streams -show_format clip.mp4
```

Decode validation:

```bash
ffmpeg -v error -i clip.mp4 -f null -
```

Validation should check:

- File readability
- Expected video stream
- Expected audio streams
- Duration tolerance
- Monotonic timestamps
- Successful full decode
- Resolution and frame rate
- Codec metadata

### 27.6 Endurance tests

Required scenarios:

- Eight-hour continuous capture
- 100 sequential clip saves
- Repeated start and stop
- Repeated setting changes
- Repeated display reconnection
- Repeated audio-device reconnection
- Simulated slow disk
- Concurrent clip save and library scan

Measure:

- Memory growth
- Handle growth
- GPU-resource growth
- Thread growth
- Queue depths
- Save latency
- Dropped frames
- A/V drift

### 27.7 Hardware matrix

At minimum, test:

- NVIDIA GPU with current driver
- AMD GPU with current driver
- Intel integrated GPU
- Multi-GPU laptop
- Single-monitor desktop
- Multiple-monitor desktop
- 60 Hz and high-refresh-rate displays
- USB microphone
- Bluetooth audio output
- Default and explicitly selected audio devices

---

## 28. MVP acceptance criteria

The MVP is acceptable when all of the following are true:

1. A user can install and launch the application.
2. A supported display can be selected and captured.
3. Desktop audio is captured.
4. A microphone can be optionally captured.
5. A 30–120-second replay duration can be configured.
6. The global hotkey saves a valid clip.
7. Saving does not stop active capture.
8. The resulting clip begins at a valid decodable point.
9. Audio and video remain synchronized within an agreed test tolerance.
10. Output fully decodes in the validation pipeline.
11. The application survives an eight-hour endurance test without unbounded resource growth.
12. Encoder failure produces a fallback or actionable error.
13. Display and audio-device loss do not crash the application.
14. Clips can be browsed, played, renamed, revealed, and deleted.
15. Low disk space is handled safely.
16. Temporary files are recovered or cleaned on restart.
17. Dependency licenses are documented and approved.
18. No OBS source code or assets are included.
19. Capture works without an internet connection.
20. Production installers and binaries are signed.

---

## 29. Development milestones

### Milestone 0: Architecture and repository

Deliverables:

- Repository scaffold
- `AGENTS.md`
- Architecture document
- Dependency proposal
- License review
- Core data types
- Structured logging
- Configuration loader
- Mock implementations

Exit criteria:

- Workspace builds
- Tests and linting pass
- No native capture implementation yet

### Milestone 1: Synthetic replay engine

Deliverables:

- Synthetic video/audio generators
- Encoded-packet model
- Replay-buffer implementation
- Keyframe-aware snapshots
- Timestamp normalization
- Deterministic tests

Exit criteria:

- Randomized snapshot tests pass
- Buffer remains bounded
- Concurrent save simulations pass

### Milestone 2: Native display capture

Deliverables:

- Display enumeration
- Windows Graphics Capture integration
- D3D11 frame transport
- Resolution-change events
- Device-loss reporting

Exit criteria:

- Stable display capture for one hour
- No growing GPU-resource usage
- Source loss produces a controlled state

### Milestone 3: Video encoding

Deliverables:

- Encoder abstraction
- One functional H.264 backend
- Hardware capability discovery
- Keyframe configuration
- Overload metrics

Exit criteria:

- Synthetic and captured frames produce valid H.264
- Encoder initialization and failure paths are tested

### Milestone 4: Desktop audio and microphone

Deliverables:

- WASAPI loopback
- Microphone capture
- Resampling
- Shared timeline synchronization
- Separate-track support where feasible

Exit criteria:

- Combined clip decodes successfully
- Sync remains within the defined tolerance

### Milestone 5: Direct MP4 muxing and replay saving

Deliverables:

- Direct H.264/AAC MP4 output with trailing `moov`
- Separate independent AAC audio tracks
- Temporary `.part` staging strategy
- Atomic finalization
- Automated media validation

Exit criteria:

- 100 consecutive MP4 clips save and decode
- Capture continues throughout save operations

### Milestone 6: Hotkeys and tray

Deliverables:

- Global hotkey registration
- Conflict handling
- Tray status
- Save notifications

Exit criteria:

- Hotkey works while other applications are focused
- Repeated activation does not corrupt output

### Milestone 7: Desktop UI and library

Deliverables:

- First-run flow
- Main recorder interface
- Settings
- Clip library
- Playback and file actions
- Storage management

Exit criteria:

- Full MVP user flow works without CLI tools

### Milestone 8: Reliability and recovery

Deliverables:

- Display recovery
- Audio-device recovery
- Sleep/resume handling
- Encoder fallback
- Crash cleanup
- Endurance test suite

Exit criteria:

- Acceptance recovery scenarios pass

### Milestone 9: Packaging and release

Deliverables:

- Installer
- Code signing
- Update strategy
- License notices
- Diagnostic export
- Release documentation

Exit criteria:

- Clean-machine installation passes
- Signed release artifacts are reproducible

---

## 30. Agent execution rules

The OpenCode agent must:

1. Work on one approved milestone at a time.
2. Present a plan before modifying multiple components.
3. Avoid adding dependencies without documenting purpose and license.
4. Prefer official documentation.
5. Add tests with every behavior change.
6. Run formatting, linting, unit tests, and relevant integration tests.
7. Report commands run and their results.
8. Clearly state hardware behavior that was not tested.
9. Never claim native functionality works based only on compilation.
10. Never copy or adapt OBS code.
11. Avoid broad refactors unrelated to the active milestone.
12. Preserve abstraction boundaries.
13. Record major technical decisions as architecture decision records.
14. Stop and ask for approval before introducing GPL components.
15. Keep the media engine independent from the UI.

---

## 31. Suggested `AGENTS.md`

```markdown
# Agent Instructions

## Product

This repository contains an original Windows-first instant-replay clipping
application.

OBS Studio and similar products may be used only as behavioral references.
They are not implementation dependencies.

## Prohibited actions

Do not:

- Copy, translate, adapt, or paraphrase OBS source code.
- Link against or import libobs.
- Reproduce OBS internal classes, identifiers, or module structure.
- Copy OBS assets, UI, branding, comments, or documentation.
- Use code from OBS issues, patches, or pull requests.
- Introduce GPL or similarly restrictive dependencies without approval.
- Implement process injection or graphics hooks during the MVP.
- Place real-time media processing in the UI.
- Claim hardware behavior is verified unless it was actually tested.

## Engineering requirements

- Prefer official Microsoft, FFmpeg, codec, and hardware-vendor
  documentation.
- Keep capture, encoding, replay buffering, and muxing behind independent
  interfaces.
- Use bounded channels and bounded buffers.
- Do not block capture or encoding threads on disk or UI work.
- Add deterministic tests for timestamp and replay-buffer behavior.
- Record dependency names, versions, licenses, and purposes.
- Record major decisions in architecture decision records.
- Run formatting, linting, and tests before completing a task.
- Report all untested assumptions and known limitations.

## Workflow

For each task:

1. Restate the scope.
2. Inspect the existing architecture.
3. Propose a short implementation plan.
4. Implement the smallest complete change.
5. Add or update tests.
6. Run validation commands.
7. Summarize changed files, results, and remaining risks.
```

---

## 32. Initial implementation prompt

```text
Read the complete specification and AGENTS.md before making changes.

Implement Milestone 0 only.

Create:
- A Rust workspace.
- A Tauri/React/TypeScript desktop application.
- The module structure defined by the specification.
- Shared media and configuration types.
- Recorder state definitions and validated state transitions.
- Structured logging.
- Versioned JSON configuration loading and saving.
- Mock video capture, audio capture, encoder, and muxer implementations.
- Unit-test and integration-test scaffolding.
- Architecture and dependency documentation.
- A third-party license inventory template.

Do not implement:
- Windows Graphics Capture.
- WASAPI.
- FFmpeg integration.
- Hardware encoding.
- Real global hotkeys.
- Real replay saving.
- OBS integration or copied OBS behavior.

Requirements:
- Keep the recorder engine independent from Tauri and React.
- Use bounded communication channels.
- Document thread ownership assumptions.
- Add tests for configuration validation and recorder state transitions.
- Run all available formatting, linting, type checking, and tests.
- Report exact commands and results.
- Clearly identify anything that could not be verified.
```

---

## 33. Open technical decisions

The following decisions require prototypes or benchmarks:

1. FFmpeg encoding versus Media Foundation for the first encoder backend
2. Direct MP4 container authoring and trailing moov metadata layout (ADR 0021)
3. In-memory versus disk-assisted packet buffering
4. Single recorder process versus separate recorder service
5. Exact hardware encoder ranking
6. Maximum officially supported replay duration
7. A/V drift tolerance
8. Default quality preset and bitrate
9. Separate audio tracks in the first public MVP
10. Clip thumbnail and playback implementation
11. Application update framework
12. Minimum supported Windows build

Each decision should be recorded in an architecture decision record before
the associated implementation is finalized.

---

## 34. Definition of done

A task is complete only when:

- Its behavior is implemented.
- Public interfaces are documented.
- Tests cover success and relevant failure paths.
- Formatting and linting pass.
- Relevant tests pass.
- New dependencies are documented.
- Error handling is present.
- Logging and metrics are adequate.
- No unrelated regressions are introduced.
- Hardware assumptions are clearly identified.
- Documentation reflects the final behavior.
- The implementation complies with the reference-use policy.

This specification defines the clipping application as an original,
focused product. OBS and similar tools establish a quality benchmark, while
the architecture, implementation, naming, tests, and user experience remain
independently developed.
