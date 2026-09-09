import { useEffect, useState, type FormEvent } from "react";
import {
  configureSettings,
  deleteClip,
  exportDiagnostics,
  getAudioDevices,
  getDisplaySources,
  getStorageSummary,
  getDiagnosticInfo,
  getRecorderStatus,
  getSettings,
  getWindowSources,
  isMockMode,
  listClips,
  onControllerEvent,
  openClip,
  pickClipDirectory,
  renameClip,
  revealClip,
  send,
  setClipProtected,
  testOverlay,
  updateThemeIcon,
} from "./ipc";
import type {
  AppSettings,
  AppTheme,
  AudioDeviceInfo,
  ClipEntry,
  Command,
  ControllerEvent,
  DiagnosticBuildInfo,
  DisplaySourceInfo,
  OverlayMode,
  OverlayPosition,
  RecorderStatus,
  StorageSummary,
  VideoCodec,
  VideoFidelityMode,
  WindowSourceInfo,
} from "./types";
import { ClipThumbnail, formatBadgeDuration } from "./ClipThumbnail";
import { ClipPlayerModal } from "./ClipPlayerModal";
import { SettingsModal } from "./SettingsModal";
import { TitleBar } from "./TitleBar";
import { SilkLogo } from "./SilkLogo";
import "./App.css";

const EMPTY_STATUS: RecorderStatus = {
  state: "Loading",
  bufferDurationMs: 0,
  bufferedPacketCount: 0,
  audioStreamCount: 0,
  saveMetrics: {
    saves_queued: 0,
    saves_completed: 0,
    saves_failed: 0,
    queue_full: 0,
    mux_failures: 0,
    total_save_duration_ms: 0,
    last_save_duration_ms: 0,
    available_disk_space_bytes: null,
  },
  fidelity: {
    requested: "standard",
    active: null,
    issue: null,
  },
};

const PREVIEW_MOCK_SETTINGS: AppSettings = {
  capture: {
    sourceType: "game",
    sourceId: "primary",
    frameRate: 60,
    outputResolution: "native",
  },
  audio: {
    tracks: [
      {
        id: "track-1",
        name: "Game Audio",
        enabled: true,
        sourceKind: "output_loopback",
        deviceId: null,
        gain: 1.0,
      },
      {
        id: "track-2",
        name: "Microphone",
        enabled: false,
        sourceKind: "input",
        deviceId: null,
        gain: 1.0,
      },
    ],
  },
  encoding: {
    videoCodec: "h264",
    encoder: "auto",
    qualityPreset: "high",
    videoBitrateKbps: 45000,
    keyframeIntervalSeconds: 2,
    audioCodec: "aac",
    audioBitrateKbps: 192,
    fidelityMode: "standard",
  },
  replay: {
    durationSeconds: 60,
  },
  output: {
    directory: "C:\\Users\\Replay\\Videos\\Silk",
    container: "mp4",
    fileNamePattern: "Clip_{date}_{time}_{source}",
  },
  hotkeys: {
    saveReplay: "Ctrl+Shift+F10",
    startCapture: null,
    stopCapture: null,
  },
  overlay: {
    enabled: false,
    mode: "full",
    position: "bottom_center",
  },
  storage: {
    quotaEnabled: false,
    quotaGigabytes: 100,
    automaticDeletionEnabled: false,
  },
  application: {
    startWithWindows: false,
    minimizeToTray: true,
    notificationsEnabled: true,
    clipSoundEnabled: true,
    theme: "classic",
  },
  diagnostics: {
    loggingLevel: "info",
  },
};

const PREVIEW_MOCK_DIAGNOSTIC_INFO: DiagnosticBuildInfo = {
  appName: "Silk",
  appVersion: "0.1.0",
  buildCommit: "preview-build",
  buildDate: "2026-08-30",
  rustVersion: "1.82.0",
  target: "x86_64-pc-windows-msvc",
  operatingSystem: "windows",
  osVersion: "Windows 11 Pro 10.0.26100",
  architecture: "x86_64",
  graphicsAdapters: [{ name: "AMD Radeon RX 9070 XT", vendorId: 0x1002 }],
  gpuDriverVersions: ["AMD Software: Adrenalin Edition 25.1.1"],
  wgcSupported: true,
  encoderBackends: [
    {
      backendName: "MF H.264 Hardware Encoder",
      codec: "avc",
      hardwareAccelerated: true,
      maxWidth: 4096,
      maxHeight: 2304,
      supportedFps: [30, 60, 120],
      supportedPixelFormats: ["nv12"],
    },
    {
      backendName: "MF H.264 Software Encoder",
      codec: "avc",
      hardwareAccelerated: false,
      maxWidth: 1920,
      maxHeight: 1080,
      supportedFps: [30, 60],
      supportedPixelFormats: ["nv12", "i420"],
    },
    {
      backendName: "MF HEVC Hardware Encoder",
      codec: "hevc",
      hardwareAccelerated: true,
      maxWidth: 8192,
      maxHeight: 4320,
      supportedFps: [30, 60, 120],
      supportedPixelFormats: ["nv12"],
    },
    {
      backendName: "MF AV1 Hardware Encoder",
      codec: "av1",
      hardwareAccelerated: true,
      maxWidth: 8192,
      maxHeight: 4320,
      supportedFps: [30, 60, 120],
      supportedPixelFormats: ["nv12"],
    },
  ],
  fidelityCapabilities: [
    {
      mode: "standard",
      availability: "available",
      issue: null,
    },
    {
      mode: "clarity",
      availability: "runtime_checked",
      issue: null,
    },
    {
      mode: "archival_444",
      availability: "unsupported",
      issue: {
        kind: "unsupported",
        code: "ARCHIVAL_444_UNSUPPORTED",
        message: "Archival 4:4:4 encoding is not supported on this platform.",
      },
    },
  ],
  ffmpegAvailable: true,
  ffprobeAvailable: true,
  probeWarnings: [],
};

const PREVIEW_MOCK_CLIPS: ClipEntry[] = [
  {
    path: "C:\\Users\\Replay\\Videos\\Silk\\Clip_2026-09-08_18-42-10_TheFinals.mp4",
    name: "Clip_2026-09-08_18-42-10_TheFinals.mp4",
    gameName: "The Finals",
    createdAtUnixMs: 1788892930000,
    durationMs: 60000,
    sizeBytes: 136_314_880,
    missing: false,
    protected: true,
  },
  {
    path: "C:\\Users\\Replay\\Videos\\Silk\\Clip_2026-09-08_17-15-22_Valorant.mp4",
    name: "Clip_2026-09-08_17-15-22_Valorant.mp4",
    gameName: "Valorant",
    createdAtUnixMs: 1788887722000,
    durationMs: 45200,
    sizeBytes: 99_614_720,
    missing: false,
    protected: false,
  },
  {
    path: "C:\\Users\\Replay\\Videos\\Silk\\Clip_2026-09-08_15-02-45_ApexLegends.mp4",
    name: "Clip_2026-09-08_15-02-45_ApexLegends.mp4",
    gameName: "Apex Legends",
    createdAtUnixMs: 1788879765000,
    durationMs: 83400,
    sizeBytes: 190_840_832,
    missing: false,
    protected: false,
  },
  {
    path: "C:\\Users\\Replay\\Videos\\Silk\\Clip_2026-09-07_21-10-00_Overwatch2.mp4",
    name: "Clip_2026-09-07_21-10-00_Overwatch2.mp4",
    gameName: "Overwatch 2",
    createdAtUnixMs: 1788815400000,
    durationMs: 30000,
    sizeBytes: 67_108_864,
    missing: false,
    protected: false,
  },
  {
    path: "C:\\Users\\Replay\\Videos\\Silk\\Clip_2026-09-06_19-30-15_Desktop.mp4",
    name: "Clip_2026-09-06_19-30-15_Desktop.mp4",
    gameName: null,
    createdAtUnixMs: 1788723015000,
    durationMs: 15000,
    sizeBytes: 33_554_432,
    missing: false,
    protected: false,
  },
];

const ALL_GAMES_FILTER = "all-games";
const UNCATEGORIZED_FILTER = "uncategorized";
const GAME_FILTER_PREFIX = "game:";
const MIN_VIDEO_BITRATE_KBPS = 2_500;
const MAX_VIDEO_BITRATE_KBPS = 100_000;

function parseVideoCodec(raw: string | undefined | null): VideoCodec | null {
  const norm = (raw ?? "").trim().toLowerCase().replace(/[^a-z0-9]/g, "");
  if (norm === "h264" || norm === "avc" || norm === "avc1") {
    return "h264";
  }
  if (norm === "hevc" || norm === "h265" || norm === "hvc1") {
    return "hevc";
  }
  if (norm === "av1" || norm === "av01") {
    return "av1";
  }
  return null;
}

function normalizeVideoCodec(raw: string | undefined | null): VideoCodec {
  return parseVideoCodec(raw) ?? "h264";
}

function normalizeTheme(raw: string | undefined | null): AppTheme {
  if (raw === "ember" || raw === "vamp") {
    return raw;
  }
  return "classic";
}

function parseVideoFidelityMode(raw: string | undefined | null): VideoFidelityMode | null {
  const norm = (raw ?? "").trim().toLowerCase().replace(/[^a-z0-9_]/g, "");
  if (norm === "standard" || norm === "clarity") {
    return "standard";
  }
  if (norm === "archival_444" || norm === "archival444" || norm === "archival") {
    return "archival_444";
  }
  return null;
}

function normalizeVideoFidelityMode(raw: string | undefined | null): VideoFidelityMode {
  return parseVideoFidelityMode(raw) ?? "standard";
}

function formatVideoFidelityMode(mode: VideoFidelityMode | string | undefined | null): string {
  const norm = normalizeVideoFidelityMode(mode);
  switch (norm) {
    case "standard":
    case "clarity":
      return "Native 1:1 Screen Resolution";
    case "archival_444":
      return "Archival (4:4:4)";
  }
}

type ClipFeedback = {
  type: "saving" | "saved" | "error";
  title: string;
  detail: string;
  path?: string;
  durationMs?: number;
  sizeBytes?: number;
};

function describe(event: ControllerEvent): string {
  switch (event.type) {
    case "status_changed":
      return `state -> ${event.state}`;
    case "clip_saved":
      return `saved ${event.path} (${event.duration_ms} ms, ${event.size_bytes} B)`;
    case "save_queued":
      return `save queued ${event.path}`;
    case "save_failed":
      return `save failed [${event.code}] ${event.message}`;
    case "command_rejected":
      return `${event.command} rejected: ${event.reason}`;
    case "notification":
      return `[${event.level}] ${event.title}: ${event.body}`;
    case "warning":
      return `warning [${event.code}] ${event.message}`;
  }
}

function formatDuration(durationMs: number | null): string {
  if (durationMs === null) {
    return "Unknown duration";
  }
  const totalSeconds = Math.max(0, Math.round(durationMs / 1_000));
  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;
  return hours > 0
    ? `${hours}h ${String(minutes).padStart(2, "0")}m ${String(seconds).padStart(2, "0")}s`
    : `${minutes}m ${String(seconds).padStart(2, "0")}s`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1_024) {
    return `${bytes} B`;
  }
  if (bytes < 1_024 * 1_024) {
    return `${(bytes / 1_024).toFixed(1)} KB`;
  }
  if (bytes < 1_024 * 1_024 * 1_024) {
    return `${(bytes / (1_024 * 1_024)).toFixed(1)} MB`;
  }
  return `${(bytes / (1_024 * 1_024 * 1_024)).toFixed(2)} GB`;
}

function formatDate(timestamp: number | null): string {
  if (timestamp === null) {
    return "Creation time unavailable";
  }
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime())
    ? "Creation time unavailable"
    : date.toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}

function fileStem(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

function fileNameFromPath(path: string): string {
  const lastSlash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return lastSlash >= 0 ? path.slice(lastSlash + 1) : path;
}

function statusClass(state: string): string {
  return state.toLowerCase().replaceAll(" ", "-");
}

function isCaptureActive(state: string): boolean {
  const normalized = state.toLowerCase();
  return (
    normalized === "ready" ||
    normalized === "buffering" ||
    normalized === "saving" ||
    normalized === "degraded" ||
    normalized === "recovering"
  );
}

function getGameName(clip: ClipEntry): string | null {
  const gameName = clip.gameName?.trim();
  return gameName || null;
}

function gameFilterLabel(filter: string): string {
  if (filter === UNCATEGORIZED_FILTER) {
    return "Uncategorized";
  }
  if (filter.startsWith(GAME_FILTER_PREFIX)) {
    return filter.slice(GAME_FILTER_PREFIX.length);
  }
  return "selected game";
}

function formatCaptureSource(
  sourceType: string,
  sourceId: string,
  displays: DisplaySourceInfo[],
  windows: WindowSourceInfo[],
  includePrefix = true,
): string {
  if (sourceType === "game") {
    return "Game Capture (Auto)";
  }
  if (sourceType === "window") {
    const win = windows.find((w) => w.id === sourceId);
    if (win) {
      const name = `${win.appName ? `${win.appName} - ` : ""}${win.title}`;
      return includePrefix ? `Window: ${name}` : name;
    }
    const name = sourceId || "None selected";
    return includePrefix ? `Window: ${name}` : name;
  }
  const disp = displays.find((d) => d.id === sourceId);
  if (disp) {
    const isPrimaryAlreadyInName = disp.name.toLowerCase().includes("primary");
    const primarySuffix = disp.is_primary && !isPrimaryAlreadyInName ? " (Primary)" : "";
    const name = `${disp.name}${primarySuffix}`;
    return includePrefix ? `Display: ${name}` : name;
  }
  const name = sourceId || "Default monitor";
  return includePrefix ? `Display: ${name}` : name;
}

const isDesignPreview = new URLSearchParams(window.location.search).get("preview") === "1";

export default function App() {
  const [status, setStatus] = useState<RecorderStatus>(EMPTY_STATUS);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [draft, setDraft] = useState<AppSettings | null>(null);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [displays, setDisplays] = useState<DisplaySourceInfo[]>([]);
  const [windows, setWindows] = useState<WindowSourceInfo[]>([]);
  const [audioDevices, setAudioDevices] = useState<AudioDeviceInfo[]>([]);
  const [displaySourcesBusy, setDisplaySourcesBusy] = useState(false);
  const [windowSourcesBusy, setWindowSourcesBusy] = useState(false);
  const [audioDevicesBusy, setAudioDevicesBusy] = useState(false);
  const [clips, setClips] = useState<ClipEntry[]>([]);
  const [storageSummary, setStorageSummary] = useState<StorageSummary | null>(null);
  const [diagnosticInfo, setDiagnosticInfo] = useState<DiagnosticBuildInfo | null>(null);
  const [diagnosticMessage, setDiagnosticMessage] = useState("");
  const [diagnosticBusy, setDiagnosticBusy] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [settingsMessage, setSettingsMessage] = useState("Loading settings...");
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [libraryMessage, setLibraryMessage] = useState("Loading clips...");
  const [libraryBusy, setLibraryBusy] = useState(false);
  const [selectedPlayerClip, setSelectedPlayerClip] = useState<ClipEntry | null>(null);
  const [editingClipPath, setEditingClipPath] = useState<string | null>(null);
  const [editingClipName, setEditingClipName] = useState<string>("");
  const [viewMode, setViewMode] = useState<"grid" | "list">(() => {
    try {
      const saved = localStorage.getItem("silk_library_view_mode");
      return saved === "list" ? "list" : "grid";
    } catch {
      return "grid";
    }
  });
  const [searchQuery, setSearchQuery] = useState<string>("");
  const [sortBy, setSortBy] = useState<"newest" | "oldest" | "duration" | "size">("newest");
  const [gameFilter, setGameFilter] = useState(ALL_GAMES_FILTER);
  const [directoryBusy, setDirectoryBusy] = useState(false);
  const [commandMessage, setCommandMessage] = useState("");
  const [clipFeedback, setClipFeedback] = useState<ClipFeedback | null>(null);
  const [testOverlayBusy, setTestOverlayBusy] = useState(false);
  const [testOverlayMessage, setTestOverlayMessage] = useState("");
  const [inAppOverlayPreview, setInAppOverlayPreview] = useState<{
    mode: OverlayMode;
    position: OverlayPosition;
  } | null>(null);

  function handleViewModeChange(mode: "grid" | "list"): void {
    setViewMode(mode);
    try {
      localStorage.setItem("silk_library_view_mode", mode);
    } catch {
      // Storage unavailable
    }
  }

  const activeTheme: AppTheme =
    draft?.application?.theme ?? settings?.application?.theme ?? "classic";

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", activeTheme);
    void updateThemeIcon(activeTheme).catch(() => {});
  }, [activeTheme]);

  // Global shortcut: Ctrl+, opens settings
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === ",") {
        e.preventDefault();
        setIsSettingsOpen((prev) => !prev);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  async function refreshDisplaySources(): Promise<void> {
    setDisplaySourcesBusy(true);
    try {
      const loaded = await getDisplaySources();
      setDisplays(loaded);
    } catch {
      // Keep existing display list
    } finally {
      setDisplaySourcesBusy(false);
    }
  }

  async function refreshWindowSources(): Promise<void> {
    setWindowSourcesBusy(true);
    try {
      const loaded = await getWindowSources();
      setWindows(loaded);
    } catch {
      // Keep existing window list
    } finally {
      setWindowSourcesBusy(false);
    }
  }

  async function refreshAudioDevices(): Promise<void> {
    setAudioDevicesBusy(true);
    try {
      const loaded = await getAudioDevices();
      setAudioDevices(loaded);
    } catch {
      // Keep existing audio endpoints
    } finally {
      setAudioDevicesBusy(false);
    }
  }

  async function refreshLibrary(): Promise<void> {
    setLibraryBusy(true);
    try {
      const nextClips = await listClips();
      setClips(nextClips);
      try {
        setStorageSummary(await getStorageSummary());
      } catch (error: unknown) {
        setLibraryMessage(`Clips loaded, but storage status is unavailable: ${String(error)}`);
        return;
      }
      setLibraryMessage(
        `${nextClips.length} indexed ${nextClips.length === 1 ? "clip" : "clips"}`,
      );
    } catch (error: unknown) {
      if (isDesignPreview) {
        setClips(PREVIEW_MOCK_CLIPS);
        setStorageSummary({
          outputDirectory: "C:\\Users\\Replay\\Videos\\Silk",
          totalBytes: 527_433_728,
          quotaBytes: 107_374_182_400,
          protectedBytes: 136_314_880,
          eligibleBytes: 391_118_848,
          automaticDeletionEnabled: false,
          deletedCount: 0,
          deletedBytes: 0,
        });
        setLibraryMessage(`${PREVIEW_MOCK_CLIPS.length} indexed clips (preview mode)`);
      } else {
        setLibraryMessage(`Could not scan clips: ${String(error)}`);
      }
    } finally {
      setLibraryBusy(false);
    }
  }

  useEffect(() => {
    let cancelled = false;
    void getSettings()
      .then((loaded) => {
        if (!cancelled) {
          const normalized: AppSettings = {
            ...loaded,
            output: {
              ...loaded.output,
              container: "mp4",
            },
            overlay: loaded.overlay ?? {
              enabled: false,
              mode: "full",
              position: "bottom_center",
            },
            application: {
              startWithWindows: loaded.application?.startWithWindows ?? false,
              minimizeToTray: loaded.application?.minimizeToTray ?? true,
              notificationsEnabled: loaded.application?.notificationsEnabled ?? true,
              clipSoundEnabled: loaded.application?.clipSoundEnabled ?? true,
              theme: normalizeTheme(loaded.application?.theme),
            },
            encoding: {
              ...loaded.encoding,
              videoCodec: normalizeVideoCodec(loaded.encoding.videoCodec),
              fidelityMode: normalizeVideoFidelityMode(loaded.encoding.fidelityMode),
              videoBitrateKbps:
                typeof loaded.encoding.videoBitrateKbps === "number" &&
                loaded.encoding.videoBitrateKbps > 0
                  ? loaded.encoding.videoBitrateKbps
                  : 45000,
            },
          };
          setSettings(normalized);
          setDraft(normalized);
          setSettingsMessage("Settings loaded.");
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          if (isDesignPreview) {
            setSettings(PREVIEW_MOCK_SETTINGS);
            setDraft(PREVIEW_MOCK_SETTINGS);
            setSettingsMessage(`Preview mode mock settings loaded (${String(error)}).`);
          } else {
            setSettings(null);
            setDraft(null);
            setSettingsMessage(`Could not load settings: ${String(error)}`);
          }
        }
      });
    void getRecorderStatus()
      .then((loaded) => {
        if (!cancelled) {
          setStatus(loaded);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          if (isDesignPreview) {
            setStatus({
              ...EMPTY_STATUS,
              state: "Ready",
              bufferDurationMs: 45_000,
              fidelity: {
                requested: "standard",
                active: "standard",
                issue: null,
              },
            });
          } else {
            setCommandMessage(`Could not load recorder status: ${String(error)}`);
          }
        }
      });
    void getDisplaySources()
      .then((loaded) => {
        if (!cancelled) {
          setDisplays(loaded);
        }
      })
      .catch(() => {
        if (!cancelled && isDesignPreview) {
          setDisplays([{
            id: "display-1",
            name: "Display 1 (2560x1440 @ 165Hz)",
            is_primary: true,
            width: 2560,
            height: 1440,
          }]);
        }
      });
    void getWindowSources()
      .then((loaded) => {
        if (!cancelled) {
          setWindows(loaded);
        }
      })
      .catch(() => undefined);
    void getAudioDevices()
      .then((loaded) => {
        if (!cancelled) {
          setAudioDevices(loaded);
        }
      })
      .catch(() => {
        if (!cancelled && isDesignPreview) {
          setAudioDevices([
            { id: "default-out", name: "Realtek HD Audio (Speakers)", kind: "output", is_default: true },
            { id: "default-in", name: "USB Condenser Microphone", kind: "input", is_default: true },
          ]);
        }
      });
    void getDiagnosticInfo()
      .then((loaded) => {
        if (!cancelled) {
          setDiagnosticInfo(loaded);
        }
      })
      .catch(() => {
        if (!cancelled && isDesignPreview) {
          setDiagnosticInfo(PREVIEW_MOCK_DIAGNOSTIC_INFO);
        }
      });
    void refreshLibrary();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const subscription = onControllerEvent((event) => {
      setLog((previous) => [describe(event), ...previous].slice(0, 50));
      if (event.type === "status_changed") {
        setStatus((previous) => ({ ...previous, state: event.state }));
      }
      if (event.type === "save_queued") {
        setCommandMessage("Saving replay to disk...");
        setClipFeedback({
          type: "saving",
          title: "Saving replay...",
          detail: "Saving instant replay MP4 to disk",
          path: event.path,
        });
      }
      if (event.type === "clip_saved") {
        setCommandMessage("Replay saved successfully.");
        setClipFeedback({
          type: "saved",
          title: "Replay saved successfully",
          detail: fileNameFromPath(event.path),
          path: event.path,
          durationMs: event.duration_ms,
          sizeBytes: event.size_bytes,
        });
        void refreshLibrary();
      }
      if (event.type === "command_rejected") {
        if (event.command === "save_replay") {
          setCommandMessage(`Save replay rejected: ${event.reason}`);
          setClipFeedback({
            type: "error",
            title: "Save replay rejected",
            detail: event.reason,
          });
        } else {
          setCommandMessage(`Command rejected: ${event.reason}`);
        }
      }
      if (event.type === "save_failed") {
        setCommandMessage(`Save replay failed: ${event.message}`);
        setClipFeedback({
          type: "error",
          title: "Replay save failed",
          detail: `[${event.code}] ${event.message}`,
        });
      }
    });
    return () => {
      void subscription.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const pollTimer = window.setInterval(() => {
      void send({ type: "poll" }).catch(() => undefined);
    }, 100);
    const statusTimer = window.setInterval(() => {
      void getRecorderStatus()
        .then(setStatus)
        .catch(() => undefined);
    }, 500);
    return () => {
      window.clearInterval(pollTimer);
      window.clearInterval(statusTimer);
    };
  }, []);

  function updateDraft(update: (current: AppSettings) => AppSettings): void {
    setDraft((current) => (current ? update(current) : current));
  }

  function handleDiscardChanges(): void {
    if (settings) {
      setDraft(settings);
      setSettingsMessage("Draft changes discarded.");
    }
  }

  async function browseClipDirectory(): Promise<void> {
    setDirectoryBusy(true);
    try {
      const directory = await pickClipDirectory();
      if (directory !== null) {
        updateDraft((current) => ({
          ...current,
          output: { ...current.output, directory },
        }));
        setSettingsMessage("Clip directory selected. Save settings to apply it.");
      }
    } catch (error: unknown) {
      setSettingsMessage(`Could not choose clip directory: ${String(error)}`);
    } finally {
      setDirectoryBusy(false);
    }
  }

  async function runCommand(command: Command, label: string): Promise<void> {
    try {
      if (isDesignPreview || isMockMode()) {
        if (command.type === "start_capture") {
          const reqMode =
            draft?.encoding.fidelityMode ?? settings?.encoding.fidelityMode ?? "standard";
          if (reqMode === "clarity") {
            setStatus((prev) => ({
              ...prev,
              state: "Buffering",
              bufferDurationMs: 0,
              fidelity: {
                requested: "clarity",
                active: null,
                issue: null,
              },
            }));
            setTimeout(() => {
              setStatus((prev) => {
                if (!isCaptureActive(prev.state)) {
                  return prev;
                }
                return {
                  ...prev,
                  state: "Ready",
                  bufferDurationMs: 45_000,
                  fidelity: {
                    requested: "clarity",
                    active: "clarity",
                    issue: null,
                  },
                };
              });
            }, 1200);
          } else {
            setStatus((prev) => ({
              ...prev,
              state: "Ready",
              bufferDurationMs: 45_000,
              fidelity: {
                requested: reqMode,
                active: reqMode,
                issue: null,
              },
            }));
          }
        } else if (command.type === "stop_capture") {
          const reqMode =
            draft?.encoding.fidelityMode ?? settings?.encoding.fidelityMode ?? "standard";
          setStatus((prev) => ({
            ...prev,
            state: "Idle",
            bufferDurationMs: 0,
            fidelity: {
              requested: reqMode,
              active: null,
              issue: null,
            },
          }));
        }
      }
      await send(command);
      setCommandMessage(`${label} requested.`);
    } catch (error: unknown) {
      setCommandMessage(`${label} failed: ${String(error)}`);
    }
  }

  async function handleTestOverlay(): Promise<void> {
    if (testOverlayBusy || !draft) {
      return;
    }
    setTestOverlayBusy(true);
    setTestOverlayMessage("Triggering HUD...");
    try {
      if (isDesignPreview || isMockMode()) {
        setInAppOverlayPreview({
          mode: draft.overlay.mode,
          position: draft.overlay.position,
        });
        setTimeout(() => {
          setInAppOverlayPreview(null);
        }, 2400);
      }
      await testOverlay();
      setTestOverlayMessage("HUD displayed");
    } catch (error: unknown) {
      setTestOverlayMessage(`Could not trigger HUD: ${String(error)}`);
    } finally {
      setTimeout(() => {
        setTestOverlayBusy(false);
        setTestOverlayMessage("");
      }, 2500);
    }
  }

  async function saveSettings(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();
    if (!draft) {
      return;
    }
    const saveReplay = draft.hotkeys.saveReplay.trim();
    const durationSeconds = draft.replay.durationSeconds;
    if (!saveReplay) {
      setSettingsMessage("Save Replay hotkey is required.");
      return;
    }
    if (!Number.isInteger(durationSeconds) || durationSeconds < 15 || durationSeconds > 120) {
      setSettingsMessage("Replay duration must be a whole number from 15 to 120 seconds.");
      return;
    }
    const validatedBitrateKbps =
      draft.encoding.videoBitrateKbps !== null && draft.encoding.videoBitrateKbps > 0
        ? Math.min(
            MAX_VIDEO_BITRATE_KBPS,
            Math.max(MIN_VIDEO_BITRATE_KBPS, Math.round(draft.encoding.videoBitrateKbps)),
          )
        : null;

    const next: AppSettings = {
      ...draft,
      output: {
        ...draft.output,
        directory: draft.output.directory.trim(),
        container: "mp4",
      },
      encoding: {
        ...draft.encoding,
        videoCodec: normalizeVideoCodec(draft.encoding.videoCodec),
        fidelityMode: normalizeVideoFidelityMode(draft.encoding.fidelityMode),
        videoBitrateKbps: validatedBitrateKbps,
      },
      audio: {
        tracks: draft.audio.tracks.map((track) => ({
          ...track,
          name: track.name.trim(),
          deviceId: track.deviceId?.trim() || null,
          gain: Number(track.gain),
        })),
      },
      application: {
        ...draft.application,
        clipSoundEnabled: Boolean(draft.application.clipSoundEnabled),
        theme: normalizeTheme(draft.application.theme),
      },
      replay: { durationSeconds },
      hotkeys: {
        saveReplay,
        startCapture: draft.hotkeys.startCapture?.trim() || null,
        stopCapture: draft.hotkeys.stopCapture?.trim() || null,
      },
    };
    if (!next.output.directory) {
      setSettingsMessage("Clip directory is required.");
      return;
    }
    setSettingsBusy(true);
    setSettingsMessage("Validating and saving settings...");
    try {
      await configureSettings(next);
      setSettings(next);
      setDraft(next);
      setSettingsMessage(
        "Settings saved. Replay duration and hotkeys are live; capture and encoder changes apply on the next start.",
      );
      await refreshLibrary();
    } catch (error: unknown) {
      setDraft((current) =>
        current
          ? {
              ...current,
              application: {
                ...current.application,
                theme: settings?.application.theme ?? "classic",
              },
            }
          : current,
      );
      setSettingsMessage(`Settings were not changed: ${String(error)}`);
    } finally {
      setSettingsBusy(false);
    }
  }

  async function createDiagnosticPackage(): Promise<void> {
    setDiagnosticBusy(true);
    setDiagnosticMessage("Collecting redacted diagnostics...");
    try {
      const info = await getDiagnosticInfo();
      setDiagnosticInfo(info);
      const path = await exportDiagnostics();
      setDiagnosticMessage(`Package exported: ${path}`);
    } catch (error: unknown) {
      setDiagnosticMessage(`Could not export diagnostics: ${String(error)}`);
    } finally {
      setDiagnosticBusy(false);
    }
  }

  function startEditingClip(clip: ClipEntry, e?: React.MouseEvent): void {
    if (e) {
      e.stopPropagation();
    }
    if (clip.missing) return;
    setEditingClipPath(clip.path);
    setEditingClipName(fileStem(clip.name));
  }

  async function saveInlineRename(clip: ClipEntry): Promise<void> {
    const trimmed = editingClipName.trim();
    setEditingClipPath(null);
    if (!trimmed || trimmed === fileStem(clip.name)) {
      return;
    }
    setLibraryBusy(true);
    try {
      const updated = await renameClip(clip.path, trimmed);
      try {
        const oldKey = `silk_vod_markers_${clip.path}`;
        const newKey = `silk_vod_markers_${updated.path}`;
        const existing = localStorage.getItem(oldKey);
        if (existing) {
          localStorage.setItem(newKey, existing);
          localStorage.removeItem(oldKey);
        }
      } catch {
        // Storage unavailable
      }
      await refreshLibrary();
      setLibraryMessage("Clip renamed.");
    } catch (error: unknown) {
      setLibraryMessage(`Could not rename clip: ${String(error)}`);
      setLibraryBusy(false);
    }
  }

  function cancelInlineRename(): void {
    setEditingClipPath(null);
  }

  async function remove(clip: ClipEntry): Promise<void> {
    if (!window.confirm(`Delete "${clip.name}" permanently?`)) {
      return;
    }
    setLibraryBusy(true);
    try {
      await deleteClip(clip.path);
      await refreshLibrary();
      setLibraryMessage("Clip deleted.");
    } catch (error: unknown) {
      setLibraryMessage(`Could not delete clip: ${String(error)}`);
      setLibraryBusy(false);
    }
  }

  async function protect(clip: ClipEntry): Promise<void> {
    setLibraryBusy(true);
    try {
      await setClipProtected(clip.path, !clip.protected);
      await refreshLibrary();
      setLibraryMessage(
        clip.protected ? "Clip protection removed." : "Clip protected from automatic deletion.",
      );
    } catch (error: unknown) {
      setLibraryMessage(`Could not update clip protection: ${String(error)}`);
      setLibraryBusy(false);
    }
  }

  async function reveal(clip: ClipEntry): Promise<void> {
    try {
      await revealClip(clip.path);
    } catch (error: unknown) {
      setLibraryMessage(`Could not reveal clip: ${String(error)}`);
    }
  }

  async function open(clip: ClipEntry): Promise<void> {
    try {
      await openClip(clip.path);
    } catch (error: unknown) {
      setLibraryMessage(`Could not open clip: ${String(error)}`);
    }
  }

  const gameNames = Array.from(
    new Set(
      clips
        .map((clip) => getGameName(clip))
        .filter((gameName): gameName is string => gameName !== null),
    ),
  ).sort((first, second) => first.localeCompare(second));

  const uncategorizedCount = clips.filter((c) => getGameName(c) === null).length;

  const gameFilteredClips = clips.filter((clip) => {
    if (gameFilter === ALL_GAMES_FILTER) {
      return true;
    }
    const gameName = getGameName(clip);
    if (gameFilter === UNCATEGORIZED_FILTER) {
      return gameName === null;
    }
    return gameFilter === `${GAME_FILTER_PREFIX}${gameName}`;
  });

  const searchFilteredClips = gameFilteredClips.filter((clip) => {
    if (!searchQuery.trim()) {
      return true;
    }
    const query = searchQuery.trim().toLowerCase();
    const nameMatch = clip.name.toLowerCase().includes(query);
    const gameMatch = (clip.gameName ?? "uncategorized").toLowerCase().includes(query);
    const dateStr = clip.createdAtUnixMs ? formatDate(clip.createdAtUnixMs).toLowerCase() : "";
    const dateMatch = dateStr.includes(query);
    return nameMatch || gameMatch || dateMatch;
  });

  const visibleClips = [...searchFilteredClips].sort((a, b) => {
    switch (sortBy) {
      case "newest":
        return (b.createdAtUnixMs ?? 0) - (a.createdAtUnixMs ?? 0);
      case "oldest":
        return (a.createdAtUnixMs ?? 0) - (b.createdAtUnixMs ?? 0);
      case "duration":
        return (b.durationMs ?? 0) - (a.durationMs ?? 0);
      case "size":
        return b.sizeBytes - a.sizeBytes;
      default:
        return 0;
    }
  });

  const selectedGameLabel = gameFilterLabel(gameFilter);

  const targetDurationMs = (draft?.replay.durationSeconds ?? 60) * 1000;
  const bufferPercent = Math.min(
    100,
    Math.max(0, Math.round((status.bufferDurationMs / targetDurationMs) * 100)),
  );
  const captureActive = isCaptureActive(status.state);
  const isSettingsDirty = settings && draft ? JSON.stringify(draft) !== JSON.stringify(settings) : false;
  const activeAudioCount =
    draft?.audio.tracks.filter((t) => t.enabled).length || status.audioStreamCount || 1;
  const activeCodecLabel = draft?.encoding.videoCodec
    ? draft.encoding.videoCodec === "h264"
      ? "H.264"
      : draft.encoding.videoCodec.toUpperCase()
    : "H.264";
  const isHardwareEncoder = draft?.encoding.encoder !== "software";
  const encoderDisplay = `${activeCodecLabel} (${isHardwareEncoder ? "HW" : "SW"})`;
  const targetDisplayLabel = draft
    ? formatCaptureSource(
        draft.capture.sourceType,
        draft.capture.sourceId,
        displays,
        windows,
        false,
      )
    : "Automatic Game Capture";

  return (
    <>
      <TitleBar />

      <main className="app-shell">
        {/* Sleek Native-Feel Top Header */}
        <header className="topbar">
          <div className="brand-lockup">
            <span className="brand-mark" aria-hidden="true">
              <SilkLogo size={22} />
            </span>
            <div>
              <p className="eyebrow">LOCAL REPLAY STUDIO</p>
              <h1>Silk</h1>
            </div>
          </div>

          <div className="topbar-meta">
            {/* Prominent, sleek Settings Trigger Button */}
            <button
              type="button"
              className={`settings-open-btn ${isSettingsDirty ? "has-unsaved" : ""}`}
              onClick={() => setIsSettingsOpen(true)}
              title="Open Settings (Ctrl+,)"
              aria-label="Open Settings dialog"
            >
              <span className="settings-gear-icon" aria-hidden="true">
                ⚙
              </span>
              <span className="settings-btn-text">Settings</span>
              {isSettingsDirty && (
                <span className="settings-unsaved-badge" title="Unsaved changes pending">
                  •
                </span>
              )}
            </button>
          </div>
        </header>

        {/* Compact Hero Status Deck */}
        <section className="hero-status-deck" aria-labelledby="recorder-heading">
          {/* Main Action Bar */}
          <div className="hero-control-card">
            <div className="hero-control-top">
              <div className="hero-status-pill-group">
                <span className={`state-badge state-${statusClass(status.state)}`}>
                  <span className="state-dot" aria-hidden="true" />
                  {status.state === "Ready"
                    ? "Capture Ready"
                    : status.state === "Buffering"
                      ? "Buffering Gameplay"
                      : status.state === "Saving"
                        ? "Saving Replay"
                        : status.state === "Stopped"
                          ? "Capture Stopped"
                          : status.state}
                </span>

                {captureActive && (
                  <span className="live-buffer-pill">
                    <span className="live-dot" /> LIVE BUFFER
                  </span>
                )}
              </div>

              <div className="hero-meta-summary">
                <span className="meta-stat">
                  <small>Target</small>
                  <strong>{targetDisplayLabel}</strong>
                </span>
                <span className="meta-stat">
                  <small>FPS</small>
                  <strong>{draft?.capture.frameRate ?? 60} fps</strong>
                </span>
                <span className="meta-stat">
                  <small>Bitrate</small>
                  <strong>
                    {draft?.encoding.videoBitrateKbps
                      ? `${Math.round(draft.encoding.videoBitrateKbps / 1000)} Mbps`
                      : "45 Mbps"}
                  </strong>
                </span>
                <span className="meta-stat">
                  <small>Audio</small>
                  <strong>
                    {activeAudioCount} {activeAudioCount === 1 ? "track" : "tracks"}
                  </strong>
                </span>
                <span className="meta-stat">
                  <small>Encoder</small>
                  <strong>{encoderDisplay}</strong>
                </span>
                <span className="meta-stat">
                  <small>Buffer</small>
                  <strong>
                    {(status.bufferDurationMs / 1000).toFixed(0)}s / {draft?.replay.durationSeconds ?? 60}s
                  </strong>
                </span>
                <span className="meta-stat">
                  <small>Fidelity</small>
                  <strong>{formatVideoFidelityMode(draft?.encoding.fidelityMode ?? "standard")}</strong>
                </span>
                <span className="meta-stat">
                  <small>Saved</small>
                  <strong>{status.saveMetrics.saves_completed} clips</strong>
                </span>
              </div>
            </div>

            <div className="hero-action-row">
              <button
                className={`button button-primary hero-save-btn ${captureActive ? "is-live-btn" : ""}`}
                type="button"
                onClick={() => void runCommand({ type: "save_replay" }, "Save Replay")}
              >
                <span className="save-btn-icon" aria-hidden="true">💾</span>
                <span className="save-btn-label">Save Replay</span>
                <span className="button-hint">{settings?.hotkeys.saveReplay ?? "Ctrl+Shift+F10"}</span>
              </button>

              <button
                className="button button-secondary compact-button"
                type="button"
                onClick={() => void runCommand({ type: "start_capture" }, "Start Capture")}
                disabled={captureActive}
              >
                Start Capture
              </button>

              <button
                className="button button-quiet compact-button"
                type="button"
                onClick={() => void runCommand({ type: "stop_capture" }, "Stop Capture")}
                disabled={status.state === "Stopped"}
              >
                Stop
              </button>
            </div>

            {/* Visual Replay Buffer Gauge */}
            <div className="hero-buffer-bar-wrapper">
              <div className="hero-buffer-header">
                <span className="hero-buffer-label">Replay Buffer Fill</span>
                <span className="hero-buffer-time">
                  {(status.bufferDurationMs / 1000).toFixed(1)}s /{" "}
                  {draft?.replay.durationSeconds ?? 60}s ({bufferPercent}%)
                </span>
              </div>
              <div
                className="buffer-gauge-track"
                role="progressbar"
                aria-label="Replay buffer fill level"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={bufferPercent}
              >
                <div
                  className={`buffer-gauge-fill ${status.state === "Ready" ? "is-ready" : ""}`}
                  style={{ width: `${bufferPercent}%` }}
                />
              </div>
            </div>

            {/* Prominent Clip Feedback Banner */}
            {clipFeedback && (
              <div
                className={`clip-feedback-banner feedback-${clipFeedback.type}`}
                role="status"
                aria-live="polite"
              >
                <div className="feedback-icon-col">
                  {clipFeedback.type === "saving" ? (
                    <span className="feedback-spinner" aria-hidden="true" />
                  ) : clipFeedback.type === "saved" ? (
                    <span className="feedback-success-badge" aria-hidden="true">
                      ✓
                    </span>
                  ) : (
                    <span className="feedback-alert-badge" aria-hidden="true">
                      !
                    </span>
                  )}
                </div>
                <div className="feedback-body">
                  <div className="feedback-header-line">
                    <strong>{clipFeedback.title}</strong>
                    {clipFeedback.durationMs ? (
                      <span className="feedback-pill">{formatDuration(clipFeedback.durationMs)}</span>
                    ) : null}
                    {clipFeedback.sizeBytes ? (
                      <span className="feedback-pill">{formatBytes(clipFeedback.sizeBytes)}</span>
                    ) : null}
                  </div>
                  <p className="feedback-detail-text">{clipFeedback.detail}</p>
                </div>
                <div className="feedback-actions">
                  {clipFeedback.path && clipFeedback.type === "saved" && (
                    <>
                      <button
                        type="button"
                        className="button button-secondary compact-button"
                        onClick={() =>
                          void reveal({
                            path: clipFeedback.path!,
                            name: fileNameFromPath(clipFeedback.path!),
                            sizeBytes: 0,
                            missing: false,
                            protected: false,
                            createdAtUnixMs: null,
                            durationMs: null,
                          })
                        }
                      >
                        Reveal
                      </button>
                      <button
                        type="button"
                        className="button button-primary compact-button"
                        onClick={() =>
                          void open({
                            path: clipFeedback.path!,
                            name: fileNameFromPath(clipFeedback.path!),
                            sizeBytes: 0,
                            missing: false,
                            protected: false,
                            createdAtUnixMs: null,
                            durationMs: null,
                          })
                        }
                      >
                        Open
                      </button>
                    </>
                  )}
                  <button
                    type="button"
                    className="feedback-dismiss-button"
                    onClick={() => setClipFeedback(null)}
                    aria-label="Dismiss message"
                  >
                    ×
                  </button>
                </div>
              </div>
            )}

            {commandMessage && !clipFeedback && (
              <p className="inline-status" role="status" aria-live="polite">
                {commandMessage}
              </p>
            )}
          </div>
        </section>

        {/* Hero Centerpiece: The Clip Library */}
        <section className="panel library-panel hero-library-section" aria-labelledby="library-heading">
          <div className="panel-heading library-heading">
            <div className="library-title-block">
              <div className="library-eyebrow-row">
                <p className="eyebrow">LOCAL ARCHIVE</p>
                <span className="library-count-pill">{clips.length} {clips.length === 1 ? "Clip" : "Clips"}</span>
                {libraryMessage && (
                  <span className="library-message" role="status" aria-live="polite">
                    {libraryMessage}
                  </span>
                )}
              </div>
              <h2 id="library-heading">Clip Library</h2>
              <p className="panel-subtitle">
                Instant replays saved locally to disk in full native resolution.
              </p>
            </div>

            <div className="library-toolbar">
              {/* Live Search Input */}
              <div className="library-search-box">
                <span className="search-icon" aria-hidden="true">🔍</span>
                <input
                  type="text"
                  placeholder="Search clips..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className="library-search-input"
                  aria-label="Search clips by name, game, or date"
                />
                {searchQuery && (
                  <button
                    type="button"
                    className="search-clear-btn"
                    onClick={() => setSearchQuery("")}
                    title="Clear search"
                    aria-label="Clear search"
                  >
                    ×
                  </button>
                )}
              </div>

              {/* Sort Dropdown */}
              <div className="library-sort-box">
                <label htmlFor="library-sort" className="library-sort-label">Sort:</label>
                <select
                  id="library-sort"
                  value={sortBy}
                  onChange={(e) => setSortBy(e.target.value as "newest" | "oldest" | "duration" | "size")}
                  className="library-select"
                  aria-label="Sort clips"
                >
                  <option value="newest">Newest First</option>
                  <option value="oldest">Oldest First</option>
                  <option value="duration">Longest Duration</option>
                  <option value="size">Largest File Size</option>
                </select>
              </div>

              {/* View Toggle Segmented Control */}
              <div className="view-mode-toggle" role="group" aria-label="Library view mode">
                <button
                  type="button"
                  className={`view-toggle-btn ${viewMode === "grid" ? "active" : ""}`}
                  onClick={() => handleViewModeChange("grid")}
                  aria-pressed={viewMode === "grid"}
                  title="Switch to Grid View"
                >
                  <span className="view-icon" aria-hidden="true">⊞</span> Grid
                </button>
                <button
                  type="button"
                  className={`view-toggle-btn ${viewMode === "list" ? "active" : ""}`}
                  onClick={() => handleViewModeChange("list")}
                  aria-pressed={viewMode === "list"}
                  title="Switch to List View"
                >
                  <span className="view-icon" aria-hidden="true">☰</span> List
                </button>
              </div>

              {/* Scan / Refresh Disk Button */}
              <button
                className="button button-secondary compact-button refresh-library-btn"
                type="button"
                onClick={() => void refreshLibrary()}
                disabled={libraryBusy}
                title="Scan disk for new replays"
              >
                {libraryBusy ? "Scanning..." : "↻ Refresh"}
              </button>
            </div>
          </div>

          {/* Interactive Game Filter Chips */}
          <div className="library-chips-bar" role="tablist" aria-label="Filter clips by game">
            <button
              type="button"
              className={`game-chip ${gameFilter === ALL_GAMES_FILTER ? "active" : ""}`}
              onClick={() => setGameFilter(ALL_GAMES_FILTER)}
              role="tab"
              aria-selected={gameFilter === ALL_GAMES_FILTER}
            >
              <span className="chip-label">All Games</span>
              <span className="chip-count">{clips.length}</span>
            </button>
            {gameNames.map((gName) => {
              const count = clips.filter((c) => getGameName(c) === gName).length;
              const filterVal = `${GAME_FILTER_PREFIX}${gName}`;
              const isSelected = gameFilter === filterVal;
              return (
                <button
                  key={gName}
                  type="button"
                  className={`game-chip ${isSelected ? "active" : ""}`}
                  onClick={() => setGameFilter(filterVal)}
                  role="tab"
                  aria-selected={isSelected}
                >
                  <span className="chip-label">{gName}</span>
                  <span className="chip-count">{count}</span>
                </button>
              );
            })}
            {uncategorizedCount > 0 && (
              <button
                type="button"
                className={`game-chip ${gameFilter === UNCATEGORIZED_FILTER ? "active" : ""}`}
                onClick={() => setGameFilter(UNCATEGORIZED_FILTER)}
                role="tab"
                aria-selected={gameFilter === UNCATEGORIZED_FILTER}
              >
                <span className="chip-label">Uncategorized</span>
                <span className="chip-count">{uncategorizedCount}</span>
              </button>
            )}
          </div>

          {storageSummary && (
            <div className="storage-summary" aria-label="Storage summary">
              <span>
                <small>Output Folder</small>
                <strong title={storageSummary.outputDirectory}>
                  {storageSummary.outputDirectory}
                </strong>
              </span>
              <span>
                <small>Storage Used</small>
                <strong>
                  {formatBytes(storageSummary.totalBytes)}
                  {storageSummary.quotaBytes === null
                    ? " / No quota"
                    : ` / ${formatBytes(storageSummary.quotaBytes)}`}
                </strong>
              </span>
              <span>
                <small>Auto Deletion</small>
                <strong
                  className={
                    storageSummary.automaticDeletionEnabled ? "text-warning" : "text-accent"
                  }
                >
                  {storageSummary.automaticDeletionEnabled ? "Enabled" : "Off"}
                </strong>
              </span>
            </div>
          )}

          {clips.length === 0 ? (
            <div className="library-empty">
              <span className="empty-symbol" aria-hidden="true">
                🎮
              </span>
              <p>No replay clips recorded yet.</p>
              <span>
                Start capture and press{" "}
                <kbd className="key-pill">{settings?.hotkeys.saveReplay ?? "Ctrl+Shift+F10"}</kbd> in-game
                to clip your gameplay moments.
              </span>
            </div>
          ) : visibleClips.length === 0 ? (
            <div className="library-empty">
              <span className="empty-symbol" aria-hidden="true">
                🔍
              </span>
              <p>
                {searchQuery
                  ? `No clips found matching "${searchQuery}".`
                  : `No clips found for ${selectedGameLabel}.`}
              </p>
              <span>
                {searchQuery ? (
                  <button
                    type="button"
                    className="button button-secondary compact-button"
                    onClick={() => setSearchQuery("")}
                    style={{ marginTop: "8px" }}
                  >
                    Clear search query
                  </button>
                ) : (
                  "Choose another game filter above to browse other clips."
                )}
              </span>
            </div>
          ) : viewMode === "grid" ? (
            /* Visual Card Grid - Clean Thumbnail First */
            <div className="clip-grid">
              {visibleClips.map((clip) => (
                <article
                  key={clip.path}
                  className={`clip-card ${clip.missing ? "clip-missing" : ""}`}
                >
                  <div
                    className="clip-card-thumb-container"
                    onClick={clip.missing ? undefined : () => setSelectedPlayerClip(clip)}
                    title={clip.missing ? undefined : "Click thumbnail to watch in player"}
                  >
                    <ClipThumbnail
                      clip={clip}
                      onClick={clip.missing ? undefined : () => setSelectedPlayerClip(clip)}
                      showBadges
                      gameBadge={getGameName(clip)}
                    />
                  </div>

                  <div className="clip-card-body">
                    <div className="clip-card-header">
                      {editingClipPath === clip.path ? (
                        <form
                          className="clip-inline-rename-form"
                          onSubmit={(e) => {
                            e.preventDefault();
                            void saveInlineRename(clip);
                          }}
                          onClick={(e) => e.stopPropagation()}
                        >
                          <input
                            type="text"
                            className="clip-inline-rename-input"
                            value={editingClipName}
                            autoFocus
                            onFocus={(e) => e.target.select()}
                            onChange={(e) => setEditingClipName(e.target.value)}
                            onBlur={() => void saveInlineRename(clip)}
                            onKeyDown={(e) => {
                              if (e.key === "Escape") {
                                e.stopPropagation();
                                cancelInlineRename();
                              } else if (e.key === "Enter") {
                                e.stopPropagation();
                              }
                            }}
                            disabled={libraryBusy}
                          />
                        </form>
                      ) : (
                        <div className="clip-title-group">
                          <h3
                            className="clip-card-title"
                            onDoubleClick={(e) => startEditingClip(clip, e)}
                            title="Double-click to rename"
                          >
                            {clip.name}
                          </h3>
                          <button
                            type="button"
                            className="clip-rename-pencil-btn"
                            onClick={(e) => startEditingClip(clip, e)}
                            title="Rename clip"
                            aria-label="Rename clip"
                          >
                            ✎
                          </button>
                        </div>
                      )}

                      <div className="clip-card-btn-group" onClick={(e) => e.stopPropagation()}>
                        <button
                          type="button"
                          className={`clip-favorite-btn ${clip.protected ? "is-active" : ""}`}
                          onClick={(e) => {
                            e.stopPropagation();
                            void protect(clip);
                          }}
                          disabled={libraryBusy}
                          title={
                            clip.protected
                              ? "Protected clip (Click to remove favorite / unprotect)"
                              : "Favorite & Protect from auto-deletion"
                          }
                          aria-label={clip.protected ? "Unprotect clip" : "Protect clip"}
                        >
                          <span className="favorite-star-icon" aria-hidden="true">
                            {clip.protected ? "★" : "☆"}
                          </span>
                        </button>
                        <button
                          type="button"
                          className="clip-delete-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            void remove(clip);
                          }}
                          disabled={libraryBusy}
                          title="Delete clip permanently"
                          aria-label="Delete clip permanently"
                        >
                          <span className="delete-trash-icon" aria-hidden="true">
                            🗑
                          </span>
                        </button>
                      </div>
                    </div>

                    <div className="clip-card-meta">
                      <span className="clip-card-date">{formatDate(clip.createdAtUnixMs)}</span>
                      <span className="meta-separator">•</span>
                      <span className="clip-card-duration">{formatBadgeDuration(clip.durationMs)}</span>
                      <span className="meta-separator">•</span>
                      <span className="clip-card-size">{formatBytes(clip.sizeBytes)}</span>
                    </div>
                  </div>
                </article>
              ))}
            </div>
          ) : (
            /* Preserved Compact List View */
            <div className="clip-list">
              {visibleClips.map((clip) => (
                <article
                  className={`clip-row ${clip.missing ? "clip-missing" : ""}`}
                  key={clip.path}
                >
                  <div
                    className="clip-row-thumb"
                    onClick={clip.missing ? undefined : () => setSelectedPlayerClip(clip)}
                    title={clip.missing ? undefined : "Click thumbnail to watch in player"}
                  >
                    <ClipThumbnail
                      clip={clip}
                      onClick={clip.missing ? undefined : () => setSelectedPlayerClip(clip)}
                      showBadges
                    />
                  </div>
                  <div className="clip-main">
                    <div className="clip-title-line">
                      {editingClipPath === clip.path ? (
                        <form
                          className="clip-inline-rename-form"
                          onSubmit={(e) => {
                            e.preventDefault();
                            void saveInlineRename(clip);
                          }}
                          onClick={(e) => e.stopPropagation()}
                        >
                          <input
                            type="text"
                            className="clip-inline-rename-input"
                            value={editingClipName}
                            autoFocus
                            onFocus={(e) => e.target.select()}
                            onChange={(e) => setEditingClipName(e.target.value)}
                            onBlur={() => void saveInlineRename(clip)}
                            onKeyDown={(e) => {
                              if (e.key === "Escape") {
                                e.stopPropagation();
                                cancelInlineRename();
                              } else if (e.key === "Enter") {
                                e.stopPropagation();
                              }
                            }}
                            disabled={libraryBusy}
                          />
                        </form>
                      ) : (
                        <div className="clip-title-group">
                          <h3
                            className="clip-card-title"
                            onDoubleClick={(e) => startEditingClip(clip, e)}
                            title="Double-click to rename"
                          >
                            {clip.name}
                          </h3>
                          <button
                            type="button"
                            className="clip-rename-pencil-btn"
                            onClick={(e) => startEditingClip(clip, e)}
                            title="Rename clip"
                            aria-label="Rename clip"
                          >
                            ✎
                          </button>
                        </div>
                      )}
                      <span className="game-badge">{getGameName(clip) ?? "Uncategorized"}</span>
                      {clip.missing && <span className="missing-badge">Missing file</span>}
                      {clip.protected && <span className="protected-badge">Protected</span>}
                    </div>
                    <p className="clip-meta-line">
                      {formatDate(clip.createdAtUnixMs)} <span aria-hidden="true">•</span>{" "}
                      {formatBadgeDuration(clip.durationMs)} <span aria-hidden="true">•</span>{" "}
                      {formatBytes(clip.sizeBytes)}
                    </p>
                  </div>
                  <div className="clip-actions" onClick={(e) => e.stopPropagation()}>
                    <button
                      type="button"
                      className={`clip-favorite-btn ${clip.protected ? "is-active" : ""}`}
                      onClick={() => void protect(clip)}
                      disabled={libraryBusy}
                      title={
                        clip.protected
                          ? "Protected clip (Click to remove favorite / unprotect)"
                          : "Favorite & Protect from auto-deletion"
                      }
                      aria-label={clip.protected ? "Unprotect clip" : "Protect clip"}
                    >
                      <span className="favorite-star-icon" aria-hidden="true">
                        {clip.protected ? "★" : "☆"}
                      </span>
                    </button>
                    <button
                      type="button"
                      className="clip-delete-btn"
                      onClick={() => void remove(clip)}
                      disabled={libraryBusy}
                      title="Delete clip permanently"
                      aria-label="Delete clip permanently"
                    >
                      <span className="delete-trash-icon" aria-hidden="true">
                        🗑
                      </span>
                    </button>
                    <button
                      className="button button-small button-accent clip-watch-btn"
                      type="button"
                      onClick={() => setSelectedPlayerClip(clip)}
                      disabled={libraryBusy || clip.missing}
                      title="Watch in Silk player"
                    >
                      ▶ Watch
                    </button>
                    <button
                      className="text-button"
                      type="button"
                      onClick={() => void reveal(clip)}
                      disabled={libraryBusy || clip.missing}
                      title="Reveal in File Explorer"
                    >
                      📁 Reveal
                    </button>
                    <button
                      className="text-button danger-button"
                      type="button"
                      onClick={() => void remove(clip)}
                      disabled={libraryBusy}
                      title="Delete clip"
                    >
                      🗑 Delete
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </section>

        {/* Activity Panel */}
        <details className="activity-panel">
          <summary>
            Recent activity log <span>{log.length}</span>
          </summary>
          {log.length === 0 ? (
            <p className="empty-copy">Controller events will appear here.</p>
          ) : (
            <ol>
              {log.map((line, index) => (
                <li key={`${line}-${index}`}>{line}</li>
              ))}
            </ol>
          )}
        </details>

        {/* Floating In-App HUD Preview */}
        {inAppOverlayPreview ? (
          <div
            className={`in-app-hud-floating in-app-pos-${inAppOverlayPreview.position.replace("_", "-")}`}
            aria-hidden="true"
          >
            <div className="native-hud-pill native-hud-pill-full native-hud-saved">
              <div className="native-hud-badge">
                <span className="native-hud-glyph native-hud-glyph-saved">✓</span>
              </div>
              <div className="native-hud-text-block">
                <strong className="native-hud-title">Silk Captured</strong>
                <span className="native-hud-subtitle">01m 00s • 94.2 MB</span>
              </div>
            </div>
          </div>
        ) : null}

        {/* Floating Settings Modal */}
        <SettingsModal
          isOpen={isSettingsOpen}
          onClose={() => setIsSettingsOpen(false)}
          draft={draft}
          settings={settings}
          updateDraft={updateDraft}
          displays={displays}
          windows={windows}
          audioDevices={audioDevices}
          displaySourcesBusy={displaySourcesBusy}
          windowSourcesBusy={windowSourcesBusy}
          audioDevicesBusy={audioDevicesBusy}
          settingsBusy={settingsBusy}
          settingsMessage={settingsMessage}
          onSave={saveSettings}
          onDiscardChanges={handleDiscardChanges}
          testOverlayBusy={testOverlayBusy}
          testOverlayMessage={testOverlayMessage}
          onTestOverlay={handleTestOverlay}
          browseClipDirectory={browseClipDirectory}
          directoryBusy={directoryBusy}
          refreshAudioDevices={refreshAudioDevices}
          refreshWindowSources={refreshWindowSources}
          refreshDisplaySources={refreshDisplaySources}
          diagnosticInfo={diagnosticInfo}
          diagnosticMessage={diagnosticMessage}
          diagnosticBusy={diagnosticBusy}
          createDiagnosticPackage={createDiagnosticPackage}
        />

        {/* Clip Player / Editor Modal */}
        {selectedPlayerClip ? (
          <ClipPlayerModal
            clip={selectedPlayerClip}
            onClose={() => setSelectedPlayerClip(null)}
            onRefreshLibrary={refreshLibrary}
            onOpenExternal={open}
          />
        ) : null}
      </main>
    </>
  );
}
