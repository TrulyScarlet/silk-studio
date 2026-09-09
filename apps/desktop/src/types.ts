export type Command =
  | { type: "start_capture" }
  | { type: "stop_capture" }
  | { type: "save_replay" }
  | { type: "poll" }
  | { type: "ping"; message: string };

export type HotkeysSettings = {
  saveReplay: string;
  startCapture: string | null;
  stopCapture: string | null;
};

export type OverlayMode = "full" | "compact";

export type OverlayPosition =
  | "top_left"
  | "top_center"
  | "top_right"
  | "center_left"
  | "center_right"
  | "bottom_left"
  | "bottom_center"
  | "bottom_right";

export type OverlaySettings = {
  enabled: boolean;
  mode: OverlayMode;
  position: OverlayPosition;
};

export type AudioTrackSettings = {
  id: string;
  name: string;
  enabled: boolean;
  sourceKind: "output_loopback" | "input";
  deviceId: string | null;
  gain: number;
};

export type AudioDeviceInfo = {
  id: string;
  name: string;
  kind: "output" | "input";
  is_default: boolean;
};

export type DisplaySourceInfo = {
  id: string;
  name: string;
  is_primary: boolean;
  width: number;
  height: number;
};

export type WindowSourceInfo = {
  id: string;
  title: string;
  appName?: string | null;
};

export type VideoCodec = "h264" | "hevc" | "av1";

export type VideoFidelityMode = "standard" | "clarity" | "archival_444";

export type FidelityIssueKind = "fallback" | "unsupported";

export type FidelityIssue = {
  kind: FidelityIssueKind;
  code: string;
  message: string;
};

export type FidelityStatus = {
  requested: VideoFidelityMode;
  active: VideoFidelityMode | null;
  issue: FidelityIssue | null;
};

export type FidelityAvailability =
  | "available"
  | "runtime_checked"
  | "fallback_only"
  | "unsupported";

export type FidelityCapability = {
  mode: VideoFidelityMode;
  availability: FidelityAvailability;
  issue: FidelityIssue | null;
};

export type PixelFormat =
  | "nv12"
  | "i420"
  | "bgra8"
  | "rgba8"
  | "ayuv"
  | string;

export type AppTheme = "studio" | "classic" | "ember" | "vamp";

export type EncoderSelection = "auto" | "hardware" | "software";

export type AppSettings = {
  capture: {
    sourceType: string;
    sourceId: string;
    frameRate: number;
    outputResolution: "native" | "3840x2160" | "2560x1440" | "1920x1080" | "1280x720";
  };
  audio: {
    tracks: AudioTrackSettings[];
  };
  encoding: {
    videoCodec: VideoCodec;
    encoder: EncoderSelection;
    qualityPreset: "low" | "medium" | "high";
    videoBitrateKbps: number | null;
    keyframeIntervalSeconds: number;
    audioCodec: string;
    audioBitrateKbps: number;
    fidelityMode: VideoFidelityMode;
  };
  replay: {
    durationSeconds: number;
  };
  output: {
    directory: string;
    container: "mp4";
    fileNamePattern: string;
  };
  hotkeys: HotkeysSettings;
  overlay: OverlaySettings;
  storage: {
    quotaEnabled: boolean;
    quotaGigabytes: number;
    automaticDeletionEnabled: boolean;
  };
  application: {
    startWithWindows: boolean;
    minimizeToTray: boolean;
    notificationsEnabled: boolean;
    clipSoundEnabled: boolean;
    theme: AppTheme;
  };
  diagnostics: {
    loggingLevel: "trace" | "debug" | "info" | "warn" | "error";
  };
};

export type RecorderStatus = {
  state: string;
  bufferDurationMs: number;
  bufferedPacketCount: number;
  audioStreamCount: number;
  saveMetrics: {
    saves_queued: number;
    saves_completed: number;
    saves_failed: number;
    queue_full: number;
    mux_failures: number;
    total_save_duration_ms: number;
    last_save_duration_ms: number;
    available_disk_space_bytes: number | null;
  };
  fidelity?: FidelityStatus | null;
};

export type ClipEntry = {
  path: string;
  name: string;
  gameName?: string | null;
  createdAtUnixMs: number | null;
  durationMs: number | null;
  sizeBytes: number;
  missing: boolean;
  protected: boolean;
};

export type StorageSummary = {
  outputDirectory: string;
  totalBytes: number;
  quotaBytes: number | null;
  protectedBytes: number;
  eligibleBytes: number;
  automaticDeletionEnabled: boolean;
  deletedCount: number;
  deletedBytes: number;
};

export type DiagnosticGraphicsAdapter = {
  name: string;
  vendorId: number;
};

export type DiagnosticEncoderBackend = {
  backendName: string;
  codec: string;
  hardwareAccelerated: boolean;
  maxWidth: number;
  maxHeight: number;
  supportedFps: number[];
  supportedPixelFormats?: PixelFormat[];
};

export type DiagnosticBuildInfo = {
  appName: string;
  appVersion: string;
  buildCommit: string | null;
  buildDate: string | null;
  rustVersion: string | null;
  target: string;
  operatingSystem: string;
  osVersion: string | null;
  architecture: string;
  graphicsAdapters: DiagnosticGraphicsAdapter[];
  gpuDriverVersions: string[];
  wgcSupported: boolean;
  encoderBackends: DiagnosticEncoderBackend[];
  fidelityCapabilities?: FidelityCapability[];
  ffmpegAvailable: boolean;
  ffprobeAvailable: boolean;
  probeWarnings: string[];
};

export type ControllerEvent =
  | { type: "status_changed"; state: string }
  | { type: "clip_saved"; path: string; duration_ms: number; size_bytes: number }
  | { type: "save_queued"; path: string }
  | { type: "save_failed"; code: string; message: string }
  | { type: "command_rejected"; command: string; reason: string }
  | { type: "notification"; level: string; title: string; body: string }
  | { type: "warning"; code: string; message: string };

export type ClipMarker = {
  id: string;
  timestampSeconds: number;
  color: string;
  label: string;
  note?: string;
  createdAt: number;
};

export type ClipMetadata = {
  markers: ClipMarker[];
  notes?: string;
};

export type ClipAudioTrackInfo = {
  trackId: number;
  name: string;
  channels: number;
  sampleRate: number;
};

export type TrimClipRequest = {
  sourcePath: string;
  outputPath: string;
  startSeconds: number;
  endSeconds: number;
  includedAudioTracks?: number[];
};
