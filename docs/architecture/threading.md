# Threading model — current and planned

Tracks worker topology against spec §14. Update this document whenever a
segment adds or changes a worker.

## Current (after deterministic Segment S8 slice)

| Worker | Lifetime | Owns | Notes |
|---|---|---|---|
| caller/controller thread | process | `Controller`, `RecorderEngine` | Executes commands sequentially, takes immutable snapshots, and drains worker results via `poll()`; never performs container I/O. |
| `silk-video-worker` | one per running session | `Box<dyn VideoCapture>`, `Box<dyn VideoEncoder>`; Media Foundation COM/MF interfaces when selected | Configures the encoder after the capture session starts, installs the session GPU context, pulls events, encodes, inserts packets into the shared `ReplayBuffer` under short lock sections, and publishes bounded encoder metrics. Reports priming once. Recovers bounded display-loss events; exits on shutdown flag, exhausted recovery, end of stream, or encoder/buffer failure. |
| `silk-audio-worker-N` | one per enabled audio source | `Box<dyn AudioCapture>`, `Box<dyn AudioEncoder>`, `AudioSynchronizer`; WASAPI and Media Foundation COM/MF interfaces when selected | Starts and recovers one source independently, synchronizes before encoding, inserts only encoded packets under short lock sections, and publishes bounded sync metrics. A source failure does not stop sibling audio workers. |
| `silk-save-worker` | one per controller session | `Box<dyn Muxer>`, free-space probe, and one bounded save queue | Runs in Windows background processing mode, checks storage headroom, receives immutable snapshots, writes direct MP4 files staged to `.part`, validates top-level boxes and timestamp monotonicity, atomically publishes final `.mp4` files, updates bounded save metrics, and returns plain `ClipMetadata` or structured errors. It never owns capture or encoder resources. |
| `silk-hotkey-loop` | one per desktop process | Win32 hotkey registrations and one bounded control/event pair | Registers global chords, receives `WM_HOTKEY`, applies per-binding debounce, and emits plain actions. It never touches controller media state directly. |
| `silk-library-worker` | one per desktop process | versioned JSON catalog, storage policy, and filesystem actions | Reconciles completed output files, reads lightweight metadata, persists the index, accounts for quota, performs oldest-first deletion of eligible unprotected clips, and validates rename/delete/protection/reveal/open operations. It never touches capture, encoding, or replay-buffer state. |
| Tauri UI/tray bridge | one per desktop process | window/tray callbacks, `HudRuntime` coordination, and plain controller events | Owns menu/window visibility, settings persistence, native notification calls, and controller command routing; routes only allowlisted commands and never processes media. |
| `silk-native-hud` | one per active native HUD instance | fixed unactivated Win32 HWND, DComposition visual tree, D3D11 swapchain, D2D device context, DirectWrite resources, and bounded command receiver | Runs a dedicated Win32 STA message pump with timer-driven phase transitions; receives cues via bounded channel and renders capture confirmation pills without cue-time HWND mutation or blocking controller, capture, or encoding threads. |
| MMDevice notification callback | managed by each audio worker's WASAPI enumerator | bounded atomic notification latch and capture event signal | Filters the selected endpoint/default flow and only latches device loss; it performs no COM, media, disk, or blocking work. |
| `silk-wgc-session` | one per WGC session | COM/WGC objects, D3D11 device/context, frame pool, capture session | Waits on a bounded control lifetime; owns platform objects and unregisters event handlers before closing the session/pool. |
| WGC `FrameArrived` callback | frame-pool managed | callback-owned cloned frame pool, D3D11 copy handles | Runs without blocking on the controller or disk. Copies into a private GPU texture, inserts it into the bounded staging registry, and drops the newest event when the event queue is full. |
| WGC GPU resolver | session-scoped; consumed by the video worker | `GpuFrameContext` and short-lived `GpuFrameLease` values | Resolver lookup is bounded to the staging registry lock; leases retain a cloned native resource, and the encoder converts it on the same D3D11 device without CPU readback. |

## Shared state (S1)

| Object | Guard | Held during |
|---|---|---|
| `ReplayBuffer` | `Mutex` (short sections) | single insert; single metrics/snapshot read; never across encode or file output |

## Communication channels

| Channel | Type | Bound | Overflow policy |
|---|---|---|---|
| replay packet store | `ReplayBuffer` behind mutex | retention window + keyframe slack; per-stream cap 20 000 | Oldest evicted at insertion |
| controller save queue | `std::sync::mpsc::SyncSender<SaveWork>` | 8 (`SAVE_QUEUE_BOUND`) | Reject with structured error |
| save results | bounded `SyncSender<SaveResult>` | 8 (`SAVE_QUEUE_BOUND`) | Save worker waits for controller polling; shutdown drains results |
| hotkey control | `std::sync::mpsc::SyncSender<ControlMessage>` | 8 (`HOTKEY_CONTROL_QUEUE_BOUND`) | Reject replacement when full/unavailable |
| hotkey events | `std::sync::mpsc::SyncSender<HotkeyEvent>` | 64 (`HOTKEY_EVENT_QUEUE_BOUND`) | Drop/report bursts; hotkey thread never waits |
| library requests | `std::sync::mpsc::SyncSender<Request>` | 8 (`LIBRARY_QUEUE_BOUND`) | Reject new filesystem work when the worker queue is full |
| library responses | one-shot bounded `SyncSender` per request | 1 | Return plain index rows or structured action errors |
| engine → controller events | `std::sync::mpsc::SyncSender` | 64 (`ENGINE_EVENT_QUEUE_BOUND`) | Nonblocking `try_send`; overflow is counted/reported and state is held directly so workers never wait for UI polling |
| WGC callback → capture consumer | `std::sync::mpsc::SyncSender` | 64 (`EVENT_QUEUE_BOUND`) | Drop newest frame and increment `frames_dropped_queue_full`; callback never waits |
| WGC staged GPU textures | `VecDeque` + `HashMap` behind `Mutex` | 8 (`STAGING_RING_SIZE`) | Evict oldest texture handle; GPU allocation stays bounded |
| WGC handle resolver → encoder | `GpuFrameContext` + `GpuFrameLease` | one lease per encoded frame (backend may retain with documented ownership) | Stale/evicted handle is reported to the encoder; no wait or registry growth |
| encoder metrics → controller | `Arc<Mutex<EncoderMetrics>>` | one latest snapshot | Replaced by the worker; readers never hold the encoder or replay-buffer lock |
| HUD cue channel | `std::sync::mpsc::SyncSender<HudCommand>` | 16 (`HUD_COMMAND_QUEUE_BOUND`) | Nonblocking `try_send`; drops cue immediately on queue full without waiting or logging |

## Shutdown protocol

1. Controller issues `StopCapture`.
2. Engine validates transition → `Stopping`, sets `Arc<AtomicBool>`.
3. Video and audio workers observe the flag between capture events, drain their
   encoders, and exit.
4. `JoinHandle::join`; the capture backend unregisters MMDevice and capture
   callbacks before closing its event and COM-owned clients.
5. The save worker finishes queued jobs, returns results, and exits before the
   controller reports `Stopped`.
6. WGC session unregisters `FrameArrived`/`Closed`, closes the session and pool,
   and releases staged GPU textures; each Media Foundation encoder releases
   its worker-local runtime state before the controller reaches `Stopped`.

The WGC session uses a normal control receive on its dedicated thread; the
public `next_event` wait is capped at 50 ms so a caller can observe stop/drop
without waiting on an unbounded compositor interval. D3D11 context access is
protected with `ID3D11Multithread` before the free-threaded WGC callback uses
it.

## Planned additions and delivered segment changes (spec §14)

| Segment | Workers added |
|---|---|
| S4 | one bounded audio worker per enabled configured track |
| S5 | bounded clip-save worker for direct pure-Rust MP4 muxing delivered; library-index worker remains planned |
| S6 | hotkey message-loop thread and Tauri tray/UI bridge (delivered); interactive native verification remains pending |
| S7 | initial desktop status/settings UI, bounded filesystem library worker, game-aware managed storage, and secure opener actions; thumbnails and playback remain planned |

S4's `RecorderEngine::new_with_audio` now accepts independently owned audio
inputs and starts one worker per enabled configured track. Each worker owns the
source capture, encoder, label, device ID, and gain, runs its synchronizer before
the audio encoder, and inserts only encoded packets into the shared bounded
replay buffer. A source startup/read failure is reported as a warning and does
not stop sibling audio workers. The default controller factory remains available
for video-only callers; the desktop shell uses the ordered track path and selects
native AAC when native audio capture is requested.

The S7 library worker also owns quota accounting and opt-in cleanup. It removes
only oldest unprotected, present files during a scan; protected and missing
catalog rows are excluded. Temporary save/delete artifacts are cleaned when the
worker opens, and opener actions receive only paths validated against the
managed `Silk` output directory. The shell's foreground-process observation is
lightweight command/poll work and never runs on capture, encoding, or muxing
workers. Native folder selection runs through the Tauri dialog command boundary
and returns only the selected path to the settings layer.

Media Foundation interfaces are deliberately created and consumed on the
owning worker's COM-initialized thread. The video factory's unsafe `Send`
boundary is only a construction handoff, and the AAC encoder is a lazy worker
object: no COM interface is sent through a media channel or accessed
concurrently. CPU/NV12 conversion and F32-to-S16 audio conversion run on their
respective workers; neither path performs WGC texture readback.

*(S1 delivered the replay-buffer coordinator as a guarded shared object
rather than a dedicated thread — ingestion and snapshotting are short
critical sections, matching ADR 0002.)*

## Rules inherited from spec §14 / ADR 0002

- No lock across file output or UI calls; snapshots share payload refs.
- Library scans and file actions run on the library worker; no capture or
  encoding worker waits on catalog or filesystem work.
- Backpressure ladder: skip preview → delay thumbnails → reduce diagnostics
  → drop late video frames → report overload → stop cleanly.
- Thread ownership of platform resources documented in the crate that
  creates them.
