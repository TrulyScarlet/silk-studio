import React, { useEffect, useRef, useState, type FormEvent } from "react";
import type {
  AppSettings,
  AudioDeviceInfo,
  AudioTrackSettings,
  DiagnosticBuildInfo,
  DisplaySourceInfo,
  FidelityCapability,
  OverlayMode,
  OverlayPosition,
  VideoCodec,
  VideoFidelityMode,
  WindowSourceInfo,
} from "./types";

type CodecOption = {
  id: VideoCodec;
  label: string;
  tag: string;
  tradeoff: string;
};

const CODEC_OPTIONS: CodecOption[] = [
  {
    id: "h264",
    label: "H.264 (AVC)",
    tag: "AVC",
    tradeoff: "Broad playback compatibility across devices and browsers.",
  },
  {
    id: "hevc",
    label: "H.265 (HEVC)",
    tag: "HEVC",
    tradeoff: "High compression efficiency across supported hardware and media players.",
  },
  {
    id: "av1",
    label: "AV1",
    tag: "AV1",
    tradeoff: "High compression efficiency for supported modern hardware and players.",
  },
];

type FidelityOption = {
  id: VideoFidelityMode;
  label: string;
  tag: string;
  tradeoff: string;
  limitations: string;
};

const FIDELITY_OPTIONS: FidelityOption[] = [
  {
    id: "standard",
    label: "Native 1:1 Screen Resolution (Recommended)",
    tag: "1:1 Native",
    tradeoff:
      "Captures your exact monitor resolution pixel-for-pixel (1080p, 1440p, or 4K). Maximum GPU performance, locked 60/120 FPS, and universal in-app playback.",
    limitations: "Standard 4:2:0 chroma subsampling for broad device, browser, and Discord compatibility.",
  },
  {
    id: "archival_444",
    label: "Archival (4:4:4)",
    tag: "4:4:4",
    tradeoff: "Full-chroma intent without color subsampling for post-capture analysis and archival editing.",
    limitations: "Requires significantly more storage bandwidth. Not supported by consumer hardware encoders on this system.",
  },
];

const BITRATE_PRESETS = [
  { label: "16 Mbps", mbps: 16, note: "1080p Balanced" },
  { label: "30 Mbps", mbps: 30, note: "1080p Crisp" },
  { label: "45 Mbps", mbps: 45, note: "1440p Competitive (Recommended)" },
  { label: "60 Mbps", mbps: 60, note: "Pristine High-Motion" },
  { label: "80 Mbps", mbps: 80, note: "Match Live Game (4K / Master)" },
];

const MAX_AUDIO_TRACKS = 8;
const MAX_AUDIO_TRACK_GAIN = 8;
const MIN_VIDEO_BITRATE_KBPS = 2_500;
const MAX_VIDEO_BITRATE_KBPS = 100_000;
const DEFAULT_CUSTOM_BITRATE_MBPS = 45;

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

function formatVideoCodec(codec: string | undefined | null): string {
  const norm = (codec ?? "").trim().toLowerCase().replace(/[^a-z0-9]/g, "");
  switch (norm) {
    case "hevc":
    case "h265":
    case "hvc1":
      return "H.265 (HEVC)";
    case "av1":
    case "av01":
      return "AV1";
    case "h264":
    case "avc":
    case "avc1":
    default:
      return "H.264 (AVC)";
  }
}

function formatVideoFidelityMode(mode: VideoFidelityMode | string | undefined | null): string {
  const norm = (mode ?? "").trim().toLowerCase().replace(/[^a-z0-9_]/g, "");
  switch (norm) {
    case "archival_444":
    case "archival444":
    case "archival":
      return "Archival (4:4:4)";
    case "standard":
    case "clarity":
    default:
      return "Native 1:1 Screen Resolution";
  }
}

function formatResolution(resolution: string): string {
  switch (resolution) {
    case "native":
      return "Native";
    case "3840x2160":
      return "3840 x 2160 (4K)";
    case "2560x1440":
      return "2560 x 1440 (1440p)";
    case "1920x1080":
      return "1920 x 1080 (1080p)";
    case "1280x720":
      return "1280 x 720 (720p)";
    default:
      return resolution;
  }
}

function getFidelityCapability(
  mode: VideoFidelityMode,
  diagnosticInfo: DiagnosticBuildInfo | null,
): FidelityCapability | null {
  if (!diagnosticInfo?.fidelityCapabilities) {
    return null;
  }
  return diagnosticInfo.fidelityCapabilities.find((cap) => cap.mode === mode) ?? null;
}

type DiagnosticStatus = {
  kind: "detected" | "warning" | "unverified";
  title: string;
  detail: string;
};

function getFidelityStatusCallout(
  mode: VideoFidelityMode,
  diagnosticInfo: DiagnosticBuildInfo | null,
): DiagnosticStatus {
  if (!diagnosticInfo) {
    if (mode === "archival_444") {
      return {
        kind: "warning",
        title: "Start-time verification required for Archival 4:4:4",
        detail:
          "System capabilities have not been probed yet. Archival 4:4:4 requires verified hardware support and will fail to start if unsupported.",
      };
    }
    return {
      kind: "detected",
      title: "Standard (4:2:0) universal baseline",
      detail:
        "Universal 4:2:0 output compatible with all hardware and software encoders.",
    };
  }

  const cap = getFidelityCapability(mode, diagnosticInfo);

  if (mode === "standard") {
    return {
      kind: "detected",
      title: "Standard (4:2:0) available",
      detail:
        "Universal 4:2:0 format. Supported across all discovered encoders and video players with balanced storage usage.",
    };
  }

  if (mode === "archival_444") {
    if (cap?.availability === "available") {
      return {
        kind: "detected",
        title: "Archival 4:4:4 supported",
        detail:
          "End-to-end full chroma preservation is supported by the configured encoder backend. Replay files will require significantly more storage.",
      };
    }
    return {
      kind: "warning",
      title: "Archival 4:4:4 is unsupported on this system",
      detail:
        cap?.issue?.message ||
        "End-to-end 4:4:4 encoding is not supported by your hardware or encoder backend. Starting capture will be rejected by the backend.",
    };
  }

  return {
    kind: "unverified",
    title: "Standard (4:2:0) active",
    detail: "Standard video fidelity mode.",
  };
}

type CodecAvailability = {
  hasHardware: boolean;
  hasSoftware: boolean;
  matchingCount: number;
  hardwareCount: number;
  softwareCount: number;
};

function getCodecAvailability(
  codec: VideoCodec,
  diagnosticInfo: DiagnosticBuildInfo | null,
): CodecAvailability | null {
  if (!diagnosticInfo) {
    return null;
  }
  const matching = diagnosticInfo.encoderBackends.filter(
    (backend) => parseVideoCodec(backend.codec) === codec,
  );
  const hw = matching.filter((b) => b.hardwareAccelerated);
  const sw = matching.filter((b) => !b.hardwareAccelerated);
  return {
    hasHardware: hw.length > 0,
    hasSoftware: sw.length > 0,
    matchingCount: matching.length,
    hardwareCount: hw.length,
    softwareCount: sw.length,
  };
}

function getCodecPolicyStatus(
  codec: VideoCodec,
  policy: "auto" | "hardware" | "software",
  diagnosticInfo: DiagnosticBuildInfo | null,
): DiagnosticStatus {
  const codecLabel = formatVideoCodec(codec);

  if (!diagnosticInfo) {
    return {
      kind: "unverified",
      title: "Availability is checked when capture starts",
      detail: `Silk will initialize a compatible encoder for ${codecLabel} when recording begins.`,
    };
  }

  const avail = getCodecAvailability(codec, diagnosticInfo);
  const hwCount = avail?.hardwareCount ?? 0;
  const swCount = avail?.softwareCount ?? 0;

  if (policy === "hardware") {
    if (hwCount > 0) {
      return {
        kind: "detected",
        title: `Hardware encoder detected for ${codecLabel}`,
        detail: `Found ${hwCount} hardware-accelerated ${codecLabel} backend${hwCount === 1 ? "" : "s"}. Direct MP4 output.`,
      };
    }
    return {
      kind: "warning",
      title: `No hardware encoder detected for ${codecLabel}`,
      detail: `Silk did not detect a hardware ${codecLabel} encoder. Capture may fail unless you switch to Automatic or select another codec.`,
    };
  }

  if (policy === "software") {
    if (swCount > 0) {
      return {
        kind: "detected",
        title: `Software encoder detected for ${codecLabel}`,
        detail: `Found ${swCount} software ${codecLabel} backend${swCount === 1 ? "" : "s"}.`,
      };
    }
    return {
      kind: "warning",
      title: `No software encoder detected for ${codecLabel}`,
      detail: `Silk did not detect a software ${codecLabel} encoder backend. Capture may fail in software-only mode.`,
    };
  }

  // policy === "auto"
  if (hwCount > 0) {
    return {
      kind: "detected",
      title: `Hardware encoder detected for ${codecLabel}`,
      detail: `Found ${hwCount} hardware ${codecLabel} backend${hwCount === 1 ? "" : "s"}. Automatic policy will prefer hardware acceleration.`,
    };
  }
  if (swCount > 0) {
    return {
      kind: "detected",
      title: `Software encoder detected for ${codecLabel}`,
      detail: `Found ${swCount} software ${codecLabel} backend${swCount === 1 ? "" : "s"}. No hardware encoder was detected for this codec.`,
    };
  }
  return {
    kind: "warning",
    title: `No encoder detected for ${codecLabel}`,
    detail: `Silk did not detect an encoder backend for ${codecLabel} on this system. Starting capture may fail.`,
  };
}

function parseKeyboardEventToKeyToken(event: React.KeyboardEvent): string | null {
  const code = event.code;
  const key = event.key;

  if (/^F([1-9]|1[0-9]|2[0-4])$/i.test(code)) return code.toUpperCase();
  if (/^F([1-9]|1[0-9]|2[0-4])$/i.test(key)) return key.toUpperCase();
  if (code.startsWith("Key") && code.length === 4) return code.slice(3).toUpperCase();
  if (code.startsWith("Digit") && code.length === 6) return code.slice(5);
  if (code.startsWith("Numpad") && /^Numpad[0-9]$/.test(code)) return code.slice(6);

  if (code === "ArrowUp" || key === "ArrowUp") return "UP";
  if (code === "ArrowDown" || key === "ArrowDown") return "DOWN";
  if (code === "ArrowLeft" || key === "ArrowLeft") return "LEFT";
  if (code === "ArrowRight" || key === "ArrowRight") return "RIGHT";

  if (code === "Space" || key === " " || key === "Spacebar") return "SPACE";
  if (code === "Tab" || key === "Tab") return "TAB";
  if (code === "Enter" || key === "Enter") return "ENTER";

  if (key.length === 1 && /^[a-zA-Z0-9]$/.test(key)) return key.toUpperCase();
  return null;
}

type HotkeyRecorderProps = {
  label: string;
  sublabel?: string;
  value: string | null;
  defaultValue?: string;
  allowClear?: boolean;
  disabled?: boolean;
  onChange: (value: string | null) => void;
};

function HotkeyRecorder({
  label,
  sublabel,
  value,
  defaultValue,
  allowClear,
  disabled,
  onChange,
}: HotkeyRecorderProps): JSX.Element {
  const [isRecording, setIsRecording] = useState(false);
  const [heldModifiers, setHeldModifiers] = useState<string[]>([]);
  const buttonRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (isRecording && buttonRef.current) {
      buttonRef.current.focus();
    }
  }, [isRecording]);

  function handleKeyDown(event: React.KeyboardEvent<HTMLButtonElement>): void {
    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      setIsRecording(false);
      setHeldModifiers([]);
      return;
    }

    const isMod =
      event.key === "Control" ||
      event.key === "Shift" ||
      event.key === "Alt" ||
      event.key === "Meta";

    if (isMod) {
      const mods: string[] = [];
      if (event.ctrlKey) mods.push("Ctrl");
      if (event.altKey) mods.push("Alt");
      if (event.shiftKey) mods.push("Shift");
      if (event.metaKey) mods.push("Win");
      setHeldModifiers(mods);
      return;
    }

    const keyToken = parseKeyboardEventToKeyToken(event);
    if (!keyToken) return;

    const parts: string[] = [];
    if (event.ctrlKey) parts.push("Ctrl");
    if (event.altKey) parts.push("Alt");
    if (event.shiftKey) parts.push("Shift");
    if (event.metaKey) parts.push("Win");
    parts.push(keyToken);

    onChange(parts.join("+"));
    setIsRecording(false);
    setHeldModifiers([]);
  }

  function handleKeyUp(event: React.KeyboardEvent<HTMLButtonElement>): void {
    if (!isRecording) return;
    const mods: string[] = [];
    if (event.ctrlKey) mods.push("Ctrl");
    if (event.altKey) mods.push("Alt");
    if (event.shiftKey) mods.push("Shift");
    if (event.metaKey) mods.push("Win");
    setHeldModifiers(mods);
  }

  function handleBlur(): void {
    if (isRecording) {
      setIsRecording(false);
      setHeldModifiers([]);
    }
  }

  return (
    <div className="hotkey-recorder-field">
      <div className="hotkey-field-label">
        <span>{label}</span>
        {sublabel ? <small>{sublabel}</small> : null}
      </div>
      <div className="hotkey-recorder-controls">
        <button
          ref={buttonRef}
          type="button"
          className={`hotkey-record-btn ${isRecording ? "is-recording" : ""}`}
          onClick={() => {
            if (!disabled) {
              setIsRecording((prev) => !prev);
              setHeldModifiers([]);
            }
          }}
          onKeyDown={isRecording ? handleKeyDown : undefined}
          onKeyUp={isRecording ? handleKeyUp : undefined}
          onBlur={handleBlur}
          disabled={disabled}
          title={
            isRecording
              ? "Press your desired key combination on the keyboard"
              : "Click to record a hotkey"
          }
          aria-label={`${label} hotkey: ${value || "None"}. Click to record.`}
        >
          {isRecording ? (
            <span className="hotkey-recording-prompt">
              <span className="recording-dot" aria-hidden="true" />
              {heldModifiers.length > 0 ? (
                <span className="hotkey-badge-list">
                  {heldModifiers.map((mod) => (
                    <kbd key={mod} className="key-pill">
                      {mod}
                    </kbd>
                  ))}
                  <span className="key-pill-separator">+</span>
                  <span className="recording-prompt-text">Press key...</span>
                </span>
              ) : (
                <span className="recording-prompt-text">
                  Press keys on keyboard... (Esc to cancel)
                </span>
              )}
            </span>
          ) : value ? (
            <span className="hotkey-badge-list">
              {value.split("+").map((token, index, arr) => (
                <span key={`${token}-${index}`} className="key-token-group">
                  <kbd className="key-pill">{token.trim()}</kbd>
                  {index < arr.length - 1 ? <span className="key-pill-separator">+</span> : null}
                </span>
              ))}
            </span>
          ) : (
            <span className="hotkey-empty-label">Unassigned (Click to record)</span>
          )}
        </button>

        {allowClear && value ? (
          <button
            type="button"
            className="button button-quiet compact-button hotkey-action-btn"
            onClick={() => onChange(null)}
            disabled={disabled || isRecording}
            title="Clear hotkey"
          >
            Clear
          </button>
        ) : null}

        {defaultValue && value !== defaultValue ? (
          <button
            type="button"
            className="button button-quiet compact-button hotkey-action-btn"
            onClick={() => onChange(defaultValue)}
            disabled={disabled || isRecording}
            title={`Reset to default (${defaultValue})`}
          >
            Reset default
          </button>
        ) : null}
      </div>
    </div>
  );
}

type NativeHudPreviewProps = {
  mode?: OverlayMode;
  position?: OverlayPosition;
  status?: "saving" | "saved" | "failed";
  title?: string;
  detail?: string;
  isMini?: boolean;
};

function NativeHudPreview({
  mode = "full",
  position = "bottom_center",
  status = "saved",
  title = "Silk Captured",
  detail = "01m 00s • 94.2 MB",
  isMini = false,
}: NativeHudPreviewProps): JSX.Element {
  const isCompact = mode === "compact";
  const posClass = position.replace("_", "-");

  if (isMini) {
    return (
      <div className={`mini-hud mini-hud-${mode} mini-hud-${status}`} aria-hidden="true">
        <span className="mini-hud-badge">
          {status === "saving" ? (
            <span className="mini-hud-arc" />
          ) : status === "failed" ? (
            "!"
          ) : (
            "✓"
          )}
        </span>
        <div className="mini-hud-content">
          <span className="mini-hud-title">{title}</span>
          {!isCompact && detail ? <span className="mini-hud-subtitle">{detail}</span> : null}
        </div>
      </div>
    );
  }

  return (
    <div
      className={`native-hud-canvas hud-pos-${posClass} hud-mode-${mode}`}
      role="status"
      aria-live="polite"
    >
      <div
        className={`native-hud-pill native-hud-pill-${mode} native-hud-${status} hud-align-${posClass}`}
      >
        <div className="native-hud-badge">
          {status === "saving" ? (
            <svg
              className="native-hud-icon-svg"
              viewBox="0 0 34 34"
              width={isCompact ? 24 : 34}
              height={isCompact ? 24 : 34}
              aria-hidden="true"
            >
              <circle
                cx={isCompact ? 12 : 17}
                cy={isCompact ? 12 : 17}
                r={isCompact ? 5.5 : 8}
                fill="none"
                stroke="rgba(255, 255, 179, 0.18)"
                strokeWidth={isCompact ? 1.2 : 1.5}
              />
              <path
                d={
                  isCompact
                    ? "M 12 6.5 A 5.5 5.5 0 1 1 7.24 14.75"
                    : "M 17 9 A 8 8 0 1 1 10.07 21"
                }
                fill="none"
                stroke="rgba(255, 255, 179, 0.92)"
                strokeWidth={isCompact ? 1.6 : 2.0}
                strokeLinecap="round"
              />
              <circle
                cx={isCompact ? 12 : 17}
                cy={isCompact ? 12 : 17}
                r={isCompact ? 1.4 : 2.0}
                fill="rgba(255, 255, 179, 0.85)"
              />
            </svg>
          ) : status === "failed" ? (
            <span className="native-hud-glyph native-hud-glyph-failed" aria-hidden="true">
              !
            </span>
          ) : (
            <span className="native-hud-glyph native-hud-glyph-saved" aria-hidden="true">
              ✓
            </span>
          )}
        </div>
        <div className="native-hud-text-block">
          <strong className="native-hud-title">{title}</strong>
          {!isCompact && detail ? <span className="native-hud-subtitle">{detail}</span> : null}
        </div>
      </div>
    </div>
  );
}

function getTrackDeviceValue(track: AudioTrackSettings): string {
  if (!track.deviceId || track.deviceId === "default") {
    return track.sourceKind === "input" ? "default:input" : "default:output";
  }
  return `${track.sourceKind === "input" ? "input" : "output"}:${track.deviceId}`;
}

export type SettingsTabId =
  | "capture"
  | "video"
  | "audio"
  | "storage"
  | "hud"
  | "theme"
  | "diagnostics";

export type SettingsModalProps = {
  isOpen: boolean;
  onClose: () => void;
  draft: AppSettings | null;
  settings: AppSettings | null;
  updateDraft: (update: (current: AppSettings) => AppSettings) => void;
  displays: DisplaySourceInfo[];
  windows: WindowSourceInfo[];
  audioDevices: AudioDeviceInfo[];
  displaySourcesBusy: boolean;
  windowSourcesBusy: boolean;
  audioDevicesBusy: boolean;
  settingsBusy: boolean;
  settingsMessage: string;
  onSave: (event: FormEvent<HTMLFormElement>) => Promise<void> | void;
  onDiscardChanges?: () => void;
  testOverlayBusy: boolean;
  testOverlayMessage: string;
  onTestOverlay: () => Promise<void> | void;
  browseClipDirectory: () => Promise<void> | void;
  directoryBusy: boolean;
  refreshAudioDevices: () => Promise<void> | void;
  refreshWindowSources: () => Promise<void> | void;
  refreshDisplaySources: () => Promise<void> | void;
  diagnosticInfo: DiagnosticBuildInfo | null;
  diagnosticMessage: string;
  diagnosticBusy: boolean;
  createDiagnosticPackage: () => Promise<void> | void;
};

export function SettingsModal({
  isOpen,
  onClose,
  draft,
  settings,
  updateDraft,
  displays,
  windows,
  audioDevices,
  displaySourcesBusy,
  windowSourcesBusy,
  audioDevicesBusy,
  settingsBusy,
  settingsMessage,
  onSave,
  onDiscardChanges,
  testOverlayBusy,
  testOverlayMessage,
  onTestOverlay,
  browseClipDirectory,
  directoryBusy,
  refreshAudioDevices,
  refreshWindowSources,
  refreshDisplaySources,
  diagnosticInfo,
  diagnosticMessage,
  diagnosticBusy,
  createDiagnosticPackage,
}: SettingsModalProps): JSX.Element | null {
  const [activeTab, setActiveTab] = useState<SettingsTabId>("capture");
  const modalRef = useRef<HTMLDivElement>(null);

  // Close on Escape key
  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen || !draft) {
    return null;
  }

  const isSettingsDirty = settings ? JSON.stringify(draft) !== JSON.stringify(settings) : false;

  function updateAudioTrack(
    trackId: string,
    update: (track: AudioTrackSettings) => AudioTrackSettings,
  ): void {
    updateDraft((current) => ({
      ...current,
      audio: {
        tracks: current.audio.tracks.map((track) =>
          track.id === trackId ? update(track) : track,
        ),
      },
    }));
  }

  function handleTrackDeviceChange(trackId: string, value: string): void {
    if (value === "default:output") {
      updateAudioTrack(trackId, (current) => ({
        ...current,
        sourceKind: "output_loopback",
        deviceId: null,
      }));
    } else if (value === "default:input") {
      updateAudioTrack(trackId, (current) => ({
        ...current,
        sourceKind: "input",
        deviceId: null,
      }));
    } else if (value.startsWith("output:")) {
      const deviceId = value.slice("output:".length);
      updateAudioTrack(trackId, (current) => ({
        ...current,
        sourceKind: "output_loopback",
        deviceId,
      }));
    } else if (value.startsWith("input:")) {
      const deviceId = value.slice("input:".length);
      updateAudioTrack(trackId, (current) => ({
        ...current,
        sourceKind: "input",
        deviceId,
      }));
    }
  }

  function isCustomTrackDevice(track: AudioTrackSettings): boolean {
    if (!track.deviceId || track.deviceId === "default") {
      return false;
    }
    const targetKind = track.sourceKind === "output_loopback" ? "output" : "input";
    return !audioDevices.some((d) => d.id === track.deviceId && d.kind === targetKind);
  }

  function addAudioTrack(): void {
    if (draft && draft.audio.tracks.length >= MAX_AUDIO_TRACKS) {
      return;
    }
    const usedIds = new Set(draft?.audio.tracks.map((track) => track.id));
    const baseId = `track-${Date.now()}`;
    let id = baseId;
    let suffix = 2;
    while (usedIds.has(id)) {
      id = `${baseId}-${suffix}`;
      suffix += 1;
    }
    const isFirst = (draft?.audio.tracks.length ?? 0) === 0;
    const track: AudioTrackSettings = {
      id,
      name: isFirst ? "Desktop Audio" : `Track ${(draft?.audio.tracks.length ?? 0) + 1}`,
      enabled: true,
      sourceKind: isFirst ? "output_loopback" : "input",
      deviceId: null,
      gain: 1,
    };
    updateDraft((current) => ({
      ...current,
      audio: { tracks: [...current.audio.tracks, track] },
    }));
  }

  function removeAudioTrack(trackId: string): void {
    updateDraft((current) => ({
      ...current,
      audio: {
        tracks: current.audio.tracks.filter((track) => track.id !== trackId),
      },
    }));
  }

  function moveAudioTrack(trackId: string, direction: -1 | 1): void {
    updateDraft((current) => {
      const index = current.audio.tracks.findIndex((track) => track.id === trackId);
      const nextIndex = index + direction;
      if (index < 0 || nextIndex < 0 || nextIndex >= current.audio.tracks.length) {
        return current;
      }
      const tracks = [...current.audio.tracks];
      const [moved] = tracks.splice(index, 1);
      tracks.splice(nextIndex, 0, moved);
      return { ...current, audio: { tracks } };
    });
  }

  return (
    <div
      className="settings-modal-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) {
          onClose();
        }
      }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="settings-dialog-title"
    >
      <div className="settings-modal-container" ref={modalRef}>
        {/* Modal Header */}
        <div className="settings-modal-header">
          <div className="settings-header-title-group">
            <span className="settings-header-glyph" aria-hidden="true">
              ⚙
            </span>
            <div>
              <h2 id="settings-dialog-title" className="settings-modal-title">
                Settings
              </h2>
              <span className="settings-modal-subtitle">
                Configure capture targets, video fidelity, audio tracks, and shortcuts
              </span>
            </div>
          </div>
          <div className="settings-header-actions">
            {isSettingsDirty && (
              <span className="settings-dirty-pill">Unsaved Changes</span>
            )}
            <button
              type="button"
              className="settings-modal-close-btn"
              onClick={onClose}
              title="Close Settings (Esc)"
              aria-label="Close Settings"
            >
              ✕
            </button>
          </div>
        </div>

        {/* Modal Main Body (Sidebar Tabs + Content Area) */}
        <form onSubmit={(e) => void onSave(e)} className="settings-modal-form">
          <div className="settings-modal-body">
            {/* Left Nav Tabs */}
            <nav className="settings-nav-sidebar" aria-label="Settings categories">
              <button
                type="button"
                className={`settings-tab-btn ${activeTab === "capture" ? "is-active" : ""}`}
                onClick={() => setActiveTab("capture")}
              >
                <span className="tab-icon" aria-hidden="true">🎮</span>
                <span className="tab-label">Capture & Hotkeys</span>
              </button>

              <button
                type="button"
                className={`settings-tab-btn ${activeTab === "video" ? "is-active" : ""}`}
                onClick={() => setActiveTab("video")}
              >
                <span className="tab-icon" aria-hidden="true">🎬</span>
                <span className="tab-label">Video & Bitrate</span>
              </button>

              <button
                type="button"
                className={`settings-tab-btn ${activeTab === "audio" ? "is-active" : ""}`}
                onClick={() => setActiveTab("audio")}
              >
                <span className="tab-icon" aria-hidden="true">🔊</span>
                <span className="tab-label">Audio Tracks</span>
                <span className="tab-badge">{draft.audio.tracks.length}</span>
              </button>

              <button
                type="button"
                className={`settings-tab-btn ${activeTab === "storage" ? "is-active" : ""}`}
                onClick={() => setActiveTab("storage")}
              >
                <span className="tab-icon" aria-hidden="true">💾</span>
                <span className="tab-label">Storage & Quota</span>
              </button>

              <button
                type="button"
                className={`settings-tab-btn ${activeTab === "hud" ? "is-active" : ""}`}
                onClick={() => setActiveTab("hud")}
              >
                <span className="tab-icon" aria-hidden="true">🔔</span>
                <span className="tab-label">HUD & Sound</span>
              </button>

              <button
                type="button"
                className={`settings-tab-btn ${activeTab === "theme" ? "is-active" : ""}`}
                onClick={() => setActiveTab("theme")}
              >
                <span className="tab-icon" aria-hidden="true">🎨</span>
                <span className="tab-label">Theme & App</span>
              </button>

              <button
                type="button"
                className={`settings-tab-btn ${activeTab === "diagnostics" ? "is-active" : ""}`}
                onClick={() => setActiveTab("diagnostics")}
              >
                <span className="tab-icon" aria-hidden="true">🩺</span>
                <span className="tab-label">Diagnostics</span>
              </button>
            </nav>

            {/* Right Scrollable Tab Content Pane */}
            <div className="settings-tab-pane">
              {/* TAB 1: CAPTURE & HOTKEYS */}
              {activeTab === "capture" && (
                <div className="settings-pane-section">
                  <div className="pane-section-header">
                    <h3>Capture Target & Resolution</h3>
                    <p>Select what Silk records and choose your target capture frame rate.</p>
                  </div>

                  <div className="settings-group-card">
                    <label className="field">
                      <span>Capture source mode</span>
                      <select
                        value={draft.capture.sourceType}
                        onChange={(event) => {
                          const newType = event.target.value;
                          updateDraft((current) => {
                            let newSourceId = current.capture.sourceId;
                            if (newType === "game") {
                              newSourceId = "auto";
                            } else if (newType === "display") {
                              if (
                                newSourceId === "auto" ||
                                !displays.some((d) => d.id === newSourceId)
                              ) {
                                newSourceId =
                                  displays.find((d) => d.is_primary)?.id ??
                                  displays[0]?.id ??
                                  "display-1";
                              }
                            } else if (newType === "window") {
                              if (
                                newSourceId === "auto" ||
                                !windows.some((w) => w.id === newSourceId)
                              ) {
                                newSourceId = windows[0]?.id ?? "";
                              }
                            }
                            return {
                              ...current,
                              capture: {
                                ...current.capture,
                                sourceType: newType,
                                sourceId: newSourceId,
                              },
                            };
                          });
                        }}
                        disabled={settingsBusy}
                      >
                        <option value="display">Display Capture (Monitor)</option>
                        <option value="game">Game Capture (Fullscreen / Borderless)</option>
                        <option value="window">Window Capture (Specific Application)</option>
                      </select>
                    </label>

                    {draft.capture.sourceType === "display" && (
                      <div className="field target-field">
                        <div className="field-label-group">
                          <label htmlFor="capture-target-display">Target monitor</label>
                          <small>Detected monitor to record</small>
                        </div>
                        <div className="target-control">
                          <select
                            id="capture-target-display"
                            value={draft.capture.sourceId}
                            onChange={(event) =>
                              updateDraft((current) => ({
                                ...current,
                                capture: { ...current.capture, sourceId: event.target.value },
                              }))
                            }
                            disabled={settingsBusy || displaySourcesBusy}
                          >
                            {displays.length === 0 ? (
                              <option value={draft.capture.sourceId || "display-1"}>
                                {draft.capture.sourceId || "Display 1 (Primary)"}
                              </option>
                            ) : (
                              displays.map((disp) => (
                                <option key={disp.id} value={disp.id}>
                                  {disp.name} {disp.is_primary ? "(Primary)" : ""}
                                </option>
                              ))
                            )}
                            {draft.capture.sourceId &&
                              !displays.some((d) => d.id === draft.capture.sourceId) && (
                                <option value={draft.capture.sourceId}>{draft.capture.sourceId}</option>
                              )}
                          </select>
                          <button
                            className="button button-secondary compact-button"
                            type="button"
                            onClick={() => void refreshDisplaySources()}
                            disabled={settingsBusy || displaySourcesBusy}
                            title="Refresh detected monitors"
                          >
                            {displaySourcesBusy ? "Scanning..." : "Refresh"}
                          </button>
                        </div>
                      </div>
                    )}

                    {draft.capture.sourceType === "game" && (
                      <div className="field target-field">
                        <div className="field-label-group">
                          <label htmlFor="capture-target-game">Target application</label>
                          <small>Foreground game capture</small>
                        </div>
                        <div className="target-control">
                          <input
                            id="capture-target-game"
                            type="text"
                            value="Automatic (Foreground / Fullscreen Game)"
                            disabled
                            readOnly
                            className="readonly-target-input"
                          />
                        </div>
                      </div>
                    )}

                    {draft.capture.sourceType === "window" && (
                      <div className="field target-field">
                        <div className="field-label-group">
                          <label htmlFor="capture-target-window">Target application</label>
                          <small>Select active application window</small>
                        </div>
                        <div className="target-control">
                          <select
                            id="capture-target-window"
                            value={draft.capture.sourceId}
                            onChange={(event) =>
                              updateDraft((current) => ({
                                ...current,
                                capture: { ...current.capture, sourceId: event.target.value },
                              }))
                            }
                            disabled={settingsBusy || windowSourcesBusy}
                          >
                            {windows.length === 0 ? (
                              <option value="">No capturable windows detected</option>
                            ) : (
                              windows.map((win) => (
                                <option key={win.id} value={win.id}>
                                  {win.title} {win.appName ? `(${win.appName})` : ""}
                                </option>
                              ))
                            )}
                            {draft.capture.sourceId &&
                              !windows.some((w) => w.id === draft.capture.sourceId) && (
                                <option value={draft.capture.sourceId}>
                                  {draft.capture.sourceId} (Previous window)
                                </option>
                              )}
                          </select>
                          <button
                            className="button button-secondary compact-button"
                            type="button"
                            onClick={() => void refreshWindowSources()}
                            disabled={settingsBusy || windowSourcesBusy}
                            title="Refresh open application windows"
                          >
                            {windowSourcesBusy ? "Scanning..." : "Refresh windows"}
                          </button>
                        </div>
                      </div>
                    )}

                    <label className="field">
                      <span>Frame rate</span>
                      <select
                        value={draft.capture.frameRate}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            capture: { ...current.capture, frameRate: Number(event.target.value) },
                          }))
                        }
                        disabled={settingsBusy}
                      >
                        <option value={30}>30 FPS</option>
                        <option value={60}>60 FPS (Recommended)</option>
                        <option value={120}>120 FPS (High Refresh Rate)</option>
                        {![30, 60, 120].includes(draft.capture.frameRate) && (
                          <option value={draft.capture.frameRate}>{draft.capture.frameRate} FPS</option>
                        )}
                      </select>
                    </label>

                    <label className="field">
                      <span>Output resolution</span>
                      <select
                        value={draft.capture.outputResolution}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            capture: {
                              ...current.capture,
                              outputResolution: event.target
                                .value as AppSettings["capture"]["outputResolution"],
                            },
                          }))
                        }
                        disabled={settingsBusy}
                      >
                        <option value="native">Native (Exact Screen Resolution)</option>
                        <option value="3840x2160">3840 x 2160 (4K)</option>
                        <option value="2560x1440">2560 x 1440 (1440p)</option>
                        <option value="1920x1080">1920 x 1080 (1080p)</option>
                        <option value="1280x720">1280 x 720 (720p)</option>
                      </select>
                    </label>
                  </div>

                  <div className="pane-section-header" style={{ marginTop: "24px" }}>
                    <h3>Global Hotkeys</h3>
                    <p>Click any hotkey below and press your desired keyboard combination.</p>
                  </div>

                  <div className="settings-group-card">
                    <HotkeyRecorder
                      label="Save Replay"
                      sublabel="required"
                      value={draft.hotkeys.saveReplay}
                      defaultValue="Ctrl+Shift+F10"
                      disabled={settingsBusy}
                      onChange={(val) =>
                        updateDraft((current) => ({
                          ...current,
                          hotkeys: {
                            ...current.hotkeys,
                            saveReplay: val || "Ctrl+Shift+F10",
                          },
                        }))
                      }
                    />
                    <HotkeyRecorder
                      label="Start Capture"
                      sublabel="optional"
                      value={draft.hotkeys.startCapture}
                      allowClear
                      disabled={settingsBusy}
                      onChange={(val) =>
                        updateDraft((current) => ({
                          ...current,
                          hotkeys: { ...current.hotkeys, startCapture: val },
                        }))
                      }
                    />
                    <HotkeyRecorder
                      label="Stop Capture"
                      sublabel="optional"
                      value={draft.hotkeys.stopCapture}
                      allowClear
                      disabled={settingsBusy}
                      onChange={(val) =>
                        updateDraft((current) => ({
                          ...current,
                          hotkeys: { ...current.hotkeys, stopCapture: val },
                        }))
                      }
                    />
                  </div>
                </div>
              )}

              {/* TAB 2: VIDEO & BITRATE */}
              {activeTab === "video" && (
                <div className="settings-pane-section">
                  <div className="pane-section-header">
                    <h3>Video Codec & Compression</h3>
                    <p>Select your hardware-accelerated video codec and quality configuration.</p>
                  </div>

                  <div className="settings-group-card">
                    <div
                      className="codec-selection-field"
                      role="radiogroup"
                      aria-labelledby="codec-selection-heading"
                    >
                      <div className="codec-card-grid">
                        {CODEC_OPTIONS.map((opt) => {
                          const isSelected = draft.encoding.videoCodec === opt.id;
                          const avail = getCodecAvailability(opt.id, diagnosticInfo);

                          let badgeText = "Checked on start";
                          let badgeStyle = "badge-neutral";

                          if (diagnosticInfo) {
                            if (avail?.hasHardware) {
                              badgeText = "Hardware detected";
                              badgeStyle = "badge-success";
                            } else if (avail?.hasSoftware) {
                              badgeText = "Software detected";
                              badgeStyle = "badge-info";
                            } else {
                              badgeText = "Not detected";
                              badgeStyle = "badge-warning";
                            }
                          }

                          return (
                            <label
                              key={opt.id}
                              className={`codec-card ${isSelected ? "is-selected" : ""} ${settingsBusy ? "is-disabled" : ""}`}
                            >
                              <input
                                type="radio"
                                name="videoCodec"
                                value={opt.id}
                                checked={isSelected}
                                onChange={() =>
                                  updateDraft((current) => ({
                                    ...current,
                                    encoding: { ...current.encoding, videoCodec: opt.id },
                                  }))
                                }
                                disabled={settingsBusy}
                                className="sr-only"
                              />
                              <div className="codec-card-header">
                                <div className="codec-title-group">
                                  <span className="codec-radio-dot" aria-hidden="true" />
                                  <strong className="codec-name">{opt.label}</strong>
                                </div>
                                <span className="codec-tag">{opt.tag}</span>
                              </div>
                              <p className="codec-tradeoff">{opt.tradeoff}</p>
                              <div className="codec-card-footer">
                                <span className={`codec-avail-badge ${badgeStyle}`}>
                                  <span className="avail-dot" aria-hidden="true" />
                                  {badgeText}
                                </span>
                              </div>
                            </label>
                          );
                        })}
                      </div>

                      {(() => {
                        const status = getCodecPolicyStatus(
                          draft.encoding.videoCodec,
                          draft.encoding.encoder,
                          diagnosticInfo,
                        );
                        return (
                          <div className={`codec-status-callout status-${status.kind}`}>
                            <div className="status-callout-icon" aria-hidden="true">
                              {status.kind === "warning" ? "!" : status.kind === "detected" ? "✓" : "ℹ"}
                            </div>
                            <div className="status-callout-content">
                              <strong>{status.title}</strong>
                              <p>{status.detail}</p>
                            </div>
                          </div>
                        );
                      })()}
                    </div>
                  </div>

                  <div className="pane-section-header" style={{ marginTop: "24px" }}>
                    <h3>Video Fidelity & Chroma</h3>
                    <p>Pixel format and color conversion intent.</p>
                  </div>

                  <div className="settings-group-card">
                    <div
                      className="fidelity-selection-field"
                      role="radiogroup"
                      aria-labelledby="fidelity-selection-heading"
                    >
                      <div className="fidelity-card-grid">
                        {FIDELITY_OPTIONS.map((opt) => {
                          const isSelected = draft.encoding.fidelityMode === opt.id;
                          const cap = getFidelityCapability(opt.id, diagnosticInfo);
                          const isUnsupported = cap ? cap.availability === "unsupported" : false;
                          const isFallback = cap ? cap.availability === "fallback_only" : false;
                          const isAvailable = cap ? cap.availability === "available" : false;
                          const isRuntimeChecked = cap ? cap.availability === "runtime_checked" : false;

                          let badgeText = "Checked when recording starts";
                          let badgeStyle = "badge-neutral";

                          if (diagnosticInfo) {
                            if (isAvailable) {
                              badgeText = "Available";
                              badgeStyle = "badge-success";
                            } else if (isRuntimeChecked) {
                              badgeText = "Checked when recording starts";
                              badgeStyle = "badge-runtime";
                            } else if (isFallback) {
                              badgeText = "Falls back to Standard";
                              badgeStyle = "badge-fallback";
                            } else if (isUnsupported) {
                              badgeText = "Unsupported";
                              badgeStyle = "badge-warning";
                            }
                          } else {
                            if (opt.id === "standard") {
                              badgeText = "Standard baseline";
                              badgeStyle = "badge-info";
                            } else {
                              badgeText = "Requires verification";
                              badgeStyle = "badge-neutral";
                            }
                          }

                          const isBlocked =
                            (isUnsupported || (!diagnosticInfo && opt.id === "archival_444")) &&
                            !isSelected;

                          return (
                            <label
                              key={opt.id}
                              className={`fidelity-card ${isSelected ? "is-selected" : ""} ${isBlocked ? "is-blocked" : ""} ${settingsBusy ? "is-disabled" : ""} ${isSelected && isUnsupported ? "is-unsupported-selected" : ""}`}
                              title={isBlocked ? `${opt.label} is unsupported on this system.` : undefined}
                            >
                              <input
                                type="radio"
                                name="fidelityMode"
                                value={opt.id}
                                checked={isSelected}
                                onChange={() => {
                                  if (!isBlocked) {
                                    updateDraft((current) => ({
                                      ...current,
                                      encoding: { ...current.encoding, fidelityMode: opt.id },
                                    }));
                                  }
                                }}
                                disabled={settingsBusy || isBlocked}
                                className="sr-only"
                              />
                              <div className="fidelity-card-header">
                                <div className="fidelity-title-group">
                                  <span className="fidelity-radio-dot" aria-hidden="true" />
                                  <strong className="fidelity-name">{opt.label}</strong>
                                </div>
                                <span className="fidelity-tag">{opt.tag}</span>
                              </div>
                              <p className="fidelity-tradeoff">{opt.tradeoff}</p>
                              <p className="fidelity-limitations">{opt.limitations}</p>
                              <div className="fidelity-card-footer">
                                <span className={`fidelity-avail-badge ${badgeStyle}`}>
                                  <span className="avail-dot" aria-hidden="true" />
                                  {badgeText}
                                </span>
                              </div>
                            </label>
                          );
                        })}
                      </div>

                      {(() => {
                        const status = getFidelityStatusCallout(
                          draft.encoding.fidelityMode,
                          diagnosticInfo,
                        );
                        return (
                          <div className={`fidelity-status-callout status-${status.kind}`}>
                            <div className="status-callout-icon" aria-hidden="true">
                              {status.kind === "warning" ? "!" : status.kind === "detected" ? "✓" : "ℹ"}
                            </div>
                            <div className="status-callout-content">
                              <strong>{status.title}</strong>
                              <p>{status.detail}</p>
                            </div>
                          </div>
                        );
                      })()}
                    </div>
                  </div>

                  <div className="pane-section-header" style={{ marginTop: "24px" }}>
                    <h3>Bitrate & Quality Presets</h3>
                    <p>Tune visual bitrate from 2.5 Mbps up to 100 Mbps.</p>
                  </div>

                  <div className="settings-group-card">
                    <div className="field bitrate-field">
                      <div className="field-label-group">
                        <label>Video bitrate</label>
                        <small>Target encoding bitrate in Mbps (2.5 - 100 Mbps)</small>
                      </div>
                      <div className="bitrate-control-wrapper">
                        <div className="bitrate-mode-toggle" role="group" aria-label="Bitrate mode">
                          <button
                            type="button"
                            className={`bitrate-mode-btn ${draft.encoding.videoBitrateKbps === null ? "is-active" : ""}`}
                            onClick={() =>
                              updateDraft((current) => ({
                                ...current,
                                encoding: { ...current.encoding, videoBitrateKbps: null },
                              }))
                            }
                            disabled={settingsBusy}
                          >
                            Auto (Optimized)
                          </button>
                          <button
                            type="button"
                            className={`bitrate-mode-btn ${draft.encoding.videoBitrateKbps !== null ? "is-active" : ""}`}
                            onClick={() =>
                              updateDraft((current) => ({
                                ...current,
                                encoding: {
                                  ...current.encoding,
                                  videoBitrateKbps:
                                    current.encoding.videoBitrateKbps ??
                                    DEFAULT_CUSTOM_BITRATE_MBPS * 1000,
                                },
                              }))
                            }
                            disabled={settingsBusy}
                          >
                            Custom Bitrate
                          </button>
                        </div>

                        {draft.encoding.videoBitrateKbps === null ? (
                          <div className="bitrate-auto-card">
                            <span className="bitrate-auto-icon" aria-hidden="true">
                              ⚙
                            </span>
                            <div className="bitrate-auto-text">
                              <strong>Auto Bitrate Active</strong>
                              <small>
                                Silk dynamically balances bitrate for your selected resolution (
                                {formatResolution(draft.capture.outputResolution)}) and frame rate (
                                {draft.capture.frameRate} FPS) with {draft.encoding.qualityPreset}{" "}
                                quality preset.
                              </small>
                            </div>
                          </div>
                        ) : (
                          <div className="bitrate-custom-controls">
                            <div className="bitrate-slider-row">
                              <input
                                type="range"
                                min={2.5}
                                max={100}
                                step={0.5}
                                value={draft.encoding.videoBitrateKbps / 1000}
                                onChange={(event) => {
                                  const mbps = parseFloat(event.target.value);
                                  const kbps = Math.min(
                                    MAX_VIDEO_BITRATE_KBPS,
                                    Math.max(MIN_VIDEO_BITRATE_KBPS, Math.round(mbps * 1000)),
                                  );
                                  updateDraft((current) => ({
                                    ...current,
                                    encoding: { ...current.encoding, videoBitrateKbps: kbps },
                                  }));
                                }}
                                disabled={settingsBusy}
                                aria-label="Video bitrate in Mbps"
                              />
                              <div className="input-with-suffix bitrate-number-input">
                                <input
                                  type="number"
                                  min={2.5}
                                  max={100}
                                  step={0.5}
                                  value={+(draft.encoding.videoBitrateKbps / 1000).toFixed(1)}
                                  onChange={(event) => {
                                    const val = parseFloat(event.target.value);
                                    if (!Number.isNaN(val)) {
                                      const kbps = Math.min(
                                        MAX_VIDEO_BITRATE_KBPS,
                                        Math.max(MIN_VIDEO_BITRATE_KBPS, Math.round(val * 1000)),
                                      );
                                      updateDraft((current) => ({
                                        ...current,
                                        encoding: { ...current.encoding, videoBitrateKbps: kbps },
                                      }));
                                    }
                                  }}
                                  disabled={settingsBusy}
                                />
                                <span>Mbps</span>
                              </div>
                            </div>

                            <div
                              className="bitrate-presets-row"
                              role="group"
                              aria-label="Bitrate quick presets"
                            >
                              {BITRATE_PRESETS.map((preset) => (
                                <button
                                  key={preset.mbps}
                                  type="button"
                                  className={`preset-chip ${Math.round(draft.encoding.videoBitrateKbps! / 1000) === preset.mbps ? "is-selected" : ""}`}
                                  onClick={() =>
                                    updateDraft((current) => ({
                                      ...current,
                                      encoding: {
                                        ...current.encoding,
                                        videoBitrateKbps: preset.mbps * 1000,
                                      },
                                    }))
                                  }
                                  disabled={settingsBusy}
                                  title={`${preset.mbps} Mbps (${preset.note})`}
                                >
                                  <strong>{preset.label}</strong>
                                  <small>{preset.note}</small>
                                </button>
                              ))}
                            </div>

                            <div className="bitrate-estimate-note">
                              <span>
                                Target:{" "}
                                <strong>
                                  {(draft.encoding.videoBitrateKbps / 1000).toFixed(1)} Mbps
                                </strong>{" "}
                                ({draft.encoding.videoBitrateKbps.toLocaleString()} kbps)
                              </span>
                              <span>
                                Estimated clip size:{" "}
                                <strong>
                                  ~
                                  {Math.round(
                                    ((draft.encoding.videoBitrateKbps / 1000) *
                                      (draft.replay.durationSeconds || 60)) /
                                      8,
                                  )}{" "}
                                  MB
                                </strong>{" "}
                                for a {draft.replay.durationSeconds || 60}s clip
                              </span>
                            </div>
                          </div>
                        )}
                      </div>
                    </div>

                    <label className="field">
                      <div className="field-label-group">
                        <span>Encoder policy</span>
                        <small>Hardware acceleration or software fallback</small>
                      </div>
                      <select
                        value={draft.encoding.encoder}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            encoding: {
                              ...current.encoding,
                              encoder: event.target.value as AppSettings["encoding"]["encoder"],
                            },
                          }))
                        }
                        disabled={settingsBusy}
                      >
                        <option value="auto">Automatic (Hardware preferred)</option>
                        <option value="hardware">Hardware only</option>
                        <option value="software">Software only</option>
                      </select>
                    </label>

                    <label className="field">
                      <span>Quality preset</span>
                      <select
                        value={draft.encoding.qualityPreset}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            encoding: {
                              ...current.encoding,
                              qualityPreset: event.target
                                .value as AppSettings["encoding"]["qualityPreset"],
                            },
                          }))
                        }
                        disabled={settingsBusy}
                      >
                        <option value="low">Low</option>
                        <option value="medium">Medium</option>
                        <option value="high">High</option>
                      </select>
                    </label>

                    <label className="field">
                      <span>
                        Keyframe interval <small>1-10 seconds</small>
                      </span>
                      <div className="input-with-suffix">
                        <input
                          type="number"
                          min="1"
                          max="10"
                          step="0.1"
                          value={draft.encoding.keyframeIntervalSeconds}
                          onChange={(event) =>
                            updateDraft((current) => ({
                              ...current,
                              encoding: {
                                ...current.encoding,
                                keyframeIntervalSeconds: Number(event.target.value),
                              },
                            }))
                          }
                          disabled={settingsBusy}
                        />
                        <span>sec</span>
                      </div>
                    </label>
                  </div>
                </div>
              )}

              {/* TAB 3: AUDIO TRACKS */}
              {activeTab === "audio" && (
                <div className="settings-pane-section">
                  <div className="pane-section-header audio-pane-header">
                    <div>
                      <h3>Independent Audio Tracks</h3>
                      <p>Capture desktop audio, voice chat, and microphones to separate channels in the MP4.</p>
                    </div>
                    <div className="audio-heading-actions">
                      <button
                        className="button button-secondary compact-button"
                        type="button"
                        onClick={() => void refreshAudioDevices()}
                        disabled={settingsBusy || audioDevicesBusy}
                        title="Refresh audio channels and endpoints"
                      >
                        {audioDevicesBusy ? "Scanning..." : "Refresh devices"}
                      </button>
                      <button
                        className="button button-primary compact-button"
                        type="button"
                        onClick={addAudioTrack}
                        disabled={settingsBusy || draft.audio.tracks.length >= MAX_AUDIO_TRACKS}
                      >
                        + Add track
                      </button>
                    </div>
                  </div>

                  <div className="audio-track-list">
                    {draft.audio.tracks.length === 0 ? (
                      <div className="audio-empty-state">
                        <p>No audio tracks configured.</p>
                        <button
                          type="button"
                          className="button button-secondary"
                          onClick={addAudioTrack}
                        >
                          Add Default Audio Track
                        </button>
                      </div>
                    ) : (
                      draft.audio.tracks.map((track, index) => (
                        <article className="audio-track-card" key={track.id}>
                          <div className="audio-track-heading">
                            <div className="audio-track-title">
                              <span className="track-number">
                                {String(index + 1).padStart(2, "0")}
                              </span>
                              <strong>{track.name || "Unnamed track"}</strong>
                            </div>
                            <div className="track-order-actions">
                              <button
                                className="icon-button"
                                type="button"
                                aria-label={`Move ${track.name || "track"} up`}
                                onClick={() => moveAudioTrack(track.id, -1)}
                                disabled={settingsBusy || index === 0}
                              >
                                ↑
                              </button>
                              <button
                                className="icon-button"
                                type="button"
                                aria-label={`Move ${track.name || "track"} down`}
                                onClick={() => moveAudioTrack(track.id, 1)}
                                disabled={settingsBusy || index === draft.audio.tracks.length - 1}
                              >
                                ↓
                              </button>
                              <button
                                className="icon-button danger-button"
                                type="button"
                                aria-label={`Remove ${track.name || "track"}`}
                                onClick={() => removeAudioTrack(track.id)}
                                disabled={settingsBusy}
                              >
                                ✕
                              </button>
                            </div>
                          </div>
                          <div className="audio-track-fields">
                            <label className="field">
                              <span>
                                Track name <small>shown in editor</small>
                              </span>
                              <input
                                value={track.name}
                                onChange={(event) =>
                                  updateAudioTrack(track.id, (current) => ({
                                    ...current,
                                    name: event.target.value,
                                  }))
                                }
                                disabled={settingsBusy}
                                placeholder="e.g. Game, Discord, Mic"
                              />
                            </label>
                            <label className="field">
                              <span>
                                Audio device <small>endpoint</small>
                              </span>
                              <select
                                value={getTrackDeviceValue(track)}
                                onChange={(event) =>
                                  handleTrackDeviceChange(track.id, event.target.value)
                                }
                                disabled={settingsBusy || !track.enabled}
                              >
                                <optgroup label="Default System Mixes">
                                  <option value="default:output">
                                    Default Output (System / Desktop Audio)
                                  </option>
                                  <option value="default:input">
                                    Default Input (Default Microphone)
                                  </option>
                                </optgroup>

                                {audioDevices.filter((d) => d.kind === "output").length > 0 && (
                                  <optgroup label="Audio Outputs (Playback / Game / Chat)">
                                    {audioDevices
                                      .filter((d) => d.kind === "output")
                                      .map((device) => (
                                        <option
                                          key={`output:${device.id}`}
                                          value={`output:${device.id}`}
                                        >
                                          {device.name} {device.is_default ? "(System Default)" : ""}
                                        </option>
                                      ))}
                                  </optgroup>
                                )}

                                {audioDevices.filter((d) => d.kind === "input").length > 0 && (
                                  <optgroup label="Audio Inputs (Microphones / Line In)">
                                    {audioDevices
                                      .filter((d) => d.kind === "input")
                                      .map((device) => (
                                        <option
                                          key={`input:${device.id}`}
                                          value={`input:${device.id}`}
                                        >
                                          {device.name} {device.is_default ? "(System Default)" : ""}
                                        </option>
                                      ))}
                                  </optgroup>
                                )}

                                {isCustomTrackDevice(track) && (
                                  <optgroup label="Custom / Disconnected Device">
                                    <option value={getTrackDeviceValue(track)}>
                                      {track.deviceId} (
                                      {track.sourceKind === "output_loopback" ? "Output" : "Input"})
                                    </option>
                                  </optgroup>
                                )}
                              </select>
                            </label>
                            <label className="field gain-field">
                              <span>
                                Gain <small>0-8x linear</small>
                              </span>
                              <div className="gain-control">
                                <input
                                  type="range"
                                  min="0"
                                  max={MAX_AUDIO_TRACK_GAIN}
                                  step="0.01"
                                  value={Math.min(MAX_AUDIO_TRACK_GAIN, Math.max(0, track.gain))}
                                  onChange={(event) =>
                                    updateAudioTrack(track.id, (current) => ({
                                      ...current,
                                      gain: Number(event.target.value),
                                    }))
                                  }
                                  disabled={settingsBusy || !track.enabled}
                                />
                                <output>{track.gain.toFixed(2)}x</output>
                              </div>
                            </label>
                          </div>
                          <label className="check-row track-enabled-row">
                            <input
                              type="checkbox"
                              checked={track.enabled}
                              onChange={(event) =>
                                updateAudioTrack(track.id, (current) => ({
                                  ...current,
                                  enabled: event.target.checked,
                                }))
                              }
                              disabled={settingsBusy}
                            />
                            <span>
                              Include this track in new captures{" "}
                              <small>off acts as mute for future sessions</small>
                            </span>
                          </label>
                        </article>
                      ))
                    )}
                  </div>
                </div>
              )}

              {/* TAB 4: STORAGE & QUOTA */}
              {activeTab === "storage" && (
                <div className="settings-pane-section">
                  <div className="pane-section-header">
                    <h3>Replay Buffer & Output Folder</h3>
                    <p>Control buffer length, clip destination, and automated disk quotas.</p>
                  </div>

                  <div className="settings-group-card">
                    <label className="field">
                      <span>
                        Replay duration <small>15-300 seconds (up to 5 min)</small>
                      </span>
                      <div className="input-with-suffix">
                        <input
                          type="number"
                          min="15"
                          max="300"
                          step="1"
                          value={draft.replay.durationSeconds}
                          onChange={(event) =>
                            updateDraft((current) => ({
                              ...current,
                              replay: { durationSeconds: Number(event.target.value) },
                            }))
                          }
                          disabled={settingsBusy}
                        />
                        <span>sec</span>
                      </div>
                    </label>
                    <div className="duration-quick-presets">
                      {[15, 30, 60, 120, 180, 300].map((presetSec) => (
                        <button
                          key={presetSec}
                          type="button"
                          className={`preset-chip ${draft.replay.durationSeconds === presetSec ? "is-active" : ""}`}
                          onClick={() =>
                            updateDraft((current) => ({
                              ...current,
                              replay: { durationSeconds: presetSec },
                            }))
                          }
                          disabled={settingsBusy}
                        >
                          {presetSec >= 60 ? `${presetSec / 60}m` : `${presetSec}s`}
                        </button>
                      ))}
                    </div>

                    <div className="field directory-field">
                      <div className="directory-label">
                        <label htmlFor="clip-directory">Clip directory</label>
                        <small id="clip-directory-help">
                          Instant replays will be saved directly to this folder
                        </small>
                      </div>
                      <div className="directory-control">
                        <input
                          id="clip-directory"
                          value={draft.output.directory}
                          onChange={(event) =>
                            updateDraft((current) => ({
                              ...current,
                              output: { ...current.output, directory: event.target.value },
                            }))
                          }
                          disabled={settingsBusy}
                          spellCheck={false}
                          aria-describedby="clip-directory-help"
                        />
                        <button
                          className="button button-secondary compact-button"
                          type="button"
                          onClick={() => void browseClipDirectory()}
                          disabled={settingsBusy || directoryBusy}
                          aria-label="Browse for clip directory"
                        >
                          {directoryBusy ? "Opening..." : "Browse"}
                        </button>
                      </div>
                    </div>

                    <label className="field">
                      <span>
                        File name pattern <small>tokens: date, time, source</small>
                      </span>
                      <input
                        value={draft.output.fileNamePattern}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            output: { ...current.output, fileNamePattern: event.target.value },
                          }))
                        }
                        disabled={settingsBusy}
                        spellCheck={false}
                      />
                    </label>
                  </div>

                  <div className="pane-section-header" style={{ marginTop: "24px" }}>
                    <h3>Disk Space & Storage Quota</h3>
                    <p>Prevent your hard drive from filling up with old replays.</p>
                  </div>

                  <div className="settings-group-card">
                    <label className="check-row">
                      <input
                        type="checkbox"
                        checked={draft.storage.quotaEnabled}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            storage: { ...current.storage, quotaEnabled: event.target.checked },
                          }))
                        }
                        disabled={settingsBusy}
                      />
                      <span>Enable clip storage quota</span>
                    </label>

                    <label className="field">
                      <span>
                        Quota limit <small>maximum disk allocation</small>
                      </span>
                      <div className="input-with-suffix">
                        <input
                          type="number"
                          min="1"
                          step="1"
                          value={draft.storage.quotaGigabytes}
                          onChange={(event) =>
                            updateDraft((current) => ({
                              ...current,
                              storage: {
                                ...current.storage,
                                quotaGigabytes: Number(event.target.value),
                              },
                            }))
                          }
                          disabled={settingsBusy || !draft.storage.quotaEnabled}
                        />
                        <span>GiB</span>
                      </div>
                    </label>

                    <label className="check-row">
                      <input
                        type="checkbox"
                        checked={draft.storage.automaticDeletionEnabled}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            storage: {
                              ...current.storage,
                              automaticDeletionEnabled: event.target.checked,
                            },
                          }))
                        }
                        disabled={settingsBusy || !draft.storage.quotaEnabled}
                      />
                      <span>
                        Automatically delete oldest unprotected clips when quota is reached{" "}
                        <small>(Protected clips are never deleted)</small>
                      </span>
                    </label>
                  </div>
                </div>
              )}

              {/* TAB 5: HUD & SOUND */}
              {activeTab === "hud" && (
                <div className="settings-pane-section">
                  <div className="pane-section-header">
                    <h3>Native Capture HUD</h3>
                    <p>On-screen confirmation pill when instant replays are saved or queued.</p>
                  </div>

                  <div className="settings-group-card">
                    <label className="check-row">
                      <input
                        type="checkbox"
                        checked={draft.overlay.enabled}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            overlay: {
                              ...current.overlay,
                              enabled: event.target.checked,
                            },
                          }))
                        }
                        disabled={settingsBusy}
                      />
                      <span>
                        Show capture confirmation HUD
                        <small>Direct2D hardware overlay. Turn off for zero game FPS impact at 240Hz+.</small>
                      </span>
                    </label>

                    <div className={`overlay-settings-controls ${!draft.overlay.enabled ? "is-disabled" : ""}`}>
                      <div className="overlay-subheading">
                        <span className="field-title">HUD style</span>
                        <span className="field-hint">Choose between detailed metadata and a low-profile pill</span>
                      </div>

                      <div className="overlay-styles-grid" role="radiogroup" aria-label="HUD layout style">
                        <button
                          type="button"
                          className={`overlay-style-card ${draft.overlay.mode === "full" ? "is-active" : ""}`}
                          onClick={() =>
                            updateDraft((current) => ({
                              ...current,
                              overlay: { ...current.overlay, mode: "full" },
                            }))
                          }
                          disabled={settingsBusy || !draft.overlay.enabled}
                          role="radio"
                          aria-checked={draft.overlay.mode === "full"}
                        >
                          <div className="overlay-card-top">
                            <div className="overlay-style-preview overlay-preview-full">
                              <NativeHudPreview
                                mode="full"
                                position="bottom_center"
                                status="saved"
                                title="Silk Captured"
                                detail="01m 00s • 94.2 MB"
                                isMini
                              />
                            </div>
                            <div className="overlay-radio-circle">
                              <span className="overlay-radio-inner" />
                            </div>
                          </div>
                          <div className="overlay-card-info">
                            <span className="overlay-card-title">Full (Detailed)</span>
                            <span className="overlay-card-desc">
                              Includes status badge, title, clip duration, and file size.
                            </span>
                          </div>
                        </button>

                        <button
                          type="button"
                          className={`overlay-style-card ${draft.overlay.mode === "compact" ? "is-active" : ""}`}
                          onClick={() =>
                            updateDraft((current) => ({
                              ...current,
                              overlay: { ...current.overlay, mode: "compact" },
                            }))
                          }
                          disabled={settingsBusy || !draft.overlay.enabled}
                          role="radio"
                          aria-checked={draft.overlay.mode === "compact"}
                        >
                          <div className="overlay-card-top">
                            <div className="overlay-style-preview overlay-preview-compact">
                              <NativeHudPreview
                                mode="compact"
                                position="bottom_center"
                                status="saved"
                                title="Silk Captured"
                                isMini
                              />
                            </div>
                            <div className="overlay-radio-circle">
                              <span className="overlay-radio-inner" />
                            </div>
                          </div>
                          <div className="overlay-card-info">
                            <span className="overlay-card-title">Compact (Minimal)</span>
                            <span className="overlay-card-desc">
                              Low-profile pill showing status icon and capture title only.
                            </span>
                          </div>
                        </button>
                      </div>

                      <div className="overlay-subheading">
                        <span className="field-title">Screen anchor position</span>
                        <span className="field-hint">Select where the HUD anchors on your display</span>
                      </div>

                      <div className="overlay-screen-widget" role="group" aria-label="HUD screen position">
                        <div className="overlay-screen-bezel">
                          <div className="overlay-screen-header">
                            <div className="screen-dots">
                              <span className="overlay-screen-dot" />
                              <span className="overlay-screen-dot" />
                              <span className="overlay-screen-dot" />
                            </div>
                            <span className="overlay-screen-title">Display Work Area</span>
                          </div>
                          <div className="overlay-screen-grid">
                            <button
                              type="button"
                              className={`overlay-pos-btn pos-tl ${draft.overlay.position === "top_left" ? "is-active" : ""}`}
                              onClick={() =>
                                updateDraft((current) => ({
                                  ...current,
                                  overlay: { ...current.overlay, position: "top_left" },
                                }))
                              }
                              disabled={settingsBusy || !draft.overlay.enabled}
                              aria-label="Top Left"
                            >
                              <span className="pos-indicator" />
                              <span className="pos-label">Top Left</span>
                            </button>

                            <button
                              type="button"
                              className={`overlay-pos-btn pos-tc ${draft.overlay.position === "top_center" ? "is-active" : ""}`}
                              onClick={() =>
                                updateDraft((current) => ({
                                  ...current,
                                  overlay: { ...current.overlay, position: "top_center" },
                                }))
                              }
                              disabled={settingsBusy || !draft.overlay.enabled}
                              aria-label="Top Center"
                            >
                              <span className="pos-indicator" />
                              <span className="pos-label">Top Center</span>
                            </button>

                            <button
                              type="button"
                              className={`overlay-pos-btn pos-tr ${draft.overlay.position === "top_right" ? "is-active" : ""}`}
                              onClick={() =>
                                updateDraft((current) => ({
                                  ...current,
                                  overlay: { ...current.overlay, position: "top_right" },
                                }))
                              }
                              disabled={settingsBusy || !draft.overlay.enabled}
                              aria-label="Top Right"
                            >
                              <span className="pos-indicator" />
                              <span className="pos-label">Top Right</span>
                            </button>

                            <button
                              type="button"
                              className={`overlay-pos-btn pos-cl ${draft.overlay.position === "center_left" ? "is-active" : ""}`}
                              onClick={() =>
                                updateDraft((current) => ({
                                  ...current,
                                  overlay: { ...current.overlay, position: "center_left" },
                                }))
                              }
                              disabled={settingsBusy || !draft.overlay.enabled}
                              aria-label="Center Left"
                            >
                              <span className="pos-indicator" />
                              <span className="pos-label">Center Left</span>
                            </button>

                            <div className="overlay-screen-center" aria-hidden="true">
                              <span className="screen-glyph-label">Display Center</span>
                            </div>

                            <button
                              type="button"
                              className={`overlay-pos-btn pos-cr ${draft.overlay.position === "center_right" ? "is-active" : ""}`}
                              onClick={() =>
                                updateDraft((current) => ({
                                  ...current,
                                  overlay: { ...current.overlay, position: "center_right" },
                                }))
                              }
                              disabled={settingsBusy || !draft.overlay.enabled}
                              aria-label="Center Right"
                            >
                              <span className="pos-indicator" />
                              <span className="pos-label">Center Right</span>
                            </button>

                            <button
                              type="button"
                              className={`overlay-pos-btn pos-bl ${draft.overlay.position === "bottom_left" ? "is-active" : ""}`}
                              onClick={() =>
                                updateDraft((current) => ({
                                  ...current,
                                  overlay: { ...current.overlay, position: "bottom_left" },
                                }))
                              }
                              disabled={settingsBusy || !draft.overlay.enabled}
                              aria-label="Bottom Left"
                            >
                              <span className="pos-indicator" />
                              <span className="pos-label">Bottom Left</span>
                            </button>

                            <button
                              type="button"
                              className={`overlay-pos-btn pos-bc ${draft.overlay.position === "bottom_center" ? "is-active" : ""}`}
                              onClick={() =>
                                updateDraft((current) => ({
                                  ...current,
                                  overlay: { ...current.overlay, position: "bottom_center" },
                                }))
                              }
                              disabled={settingsBusy || !draft.overlay.enabled}
                              aria-label="Bottom Center (Default)"
                            >
                              <span className="pos-indicator" />
                              <span className="pos-label">Bottom Center (Default)</span>
                            </button>

                            <button
                              type="button"
                              className={`overlay-pos-btn pos-br ${draft.overlay.position === "bottom_right" ? "is-active" : ""}`}
                              onClick={() =>
                                updateDraft((current) => ({
                                  ...current,
                                  overlay: { ...current.overlay, position: "bottom_right" },
                                }))
                              }
                              disabled={settingsBusy || !draft.overlay.enabled}
                              aria-label="Bottom Right"
                            >
                              <span className="pos-indicator" />
                              <span className="pos-label">Bottom Right</span>
                            </button>
                          </div>
                        </div>
                      </div>

                      <div className="overlay-test-row">
                        <button
                          type="button"
                          className="button button-secondary overlay-test-btn"
                          onClick={() => void onTestOverlay()}
                          disabled={settingsBusy || testOverlayBusy || !draft.overlay.enabled}
                          title="Display a preview of the native HUD on your screen"
                        >
                          <span>{testOverlayBusy ? "Triggering HUD..." : "▶ Test Native HUD Preview"}</span>
                        </button>
                        {testOverlayMessage ? (
                          <span className="form-status overlay-test-status" role="status" aria-live="polite">
                            {testOverlayMessage}
                          </span>
                        ) : null}
                      </div>
                    </div>
                  </div>

                  <div className="pane-section-header" style={{ marginTop: "24px" }}>
                    <h3>Sound & Notifications</h3>
                    <p>Auditory confirmation and system tray alerts.</p>
                  </div>

                  <div className="settings-group-card">
                    <label className="check-row">
                      <input
                        type="checkbox"
                        checked={draft.application.clipSoundEnabled}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            application: {
                              ...current.application,
                              clipSoundEnabled: event.target.checked,
                            },
                          }))
                        }
                        disabled={settingsBusy}
                      />
                      <span>
                        Play auditory chime when clip is saved
                        <small>Recommended if HUD overlay is disabled for high FPS gaming</small>
                      </span>
                    </label>

                    <label className="check-row">
                      <input
                        type="checkbox"
                        checked={draft.application.notificationsEnabled}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            application: {
                              ...current.application,
                              notificationsEnabled: event.target.checked,
                            },
                          }))
                        }
                        disabled={settingsBusy}
                      />
                      <span>Show Windows notifications when a replay is saved</span>
                    </label>
                  </div>
                </div>
              )}

              {/* TAB 6: THEME & APP */}
              {activeTab === "theme" && (
                <div className="settings-pane-section">
                  <div className="pane-section-header">
                    <h3>Desktop Theme</h3>
                    <p>Select your visual aesthetic palette for the Silk window.</p>
                  </div>

                  <div className="settings-group-card">
                    <div className="theme-card-grid">
                      <button
                        type="button"
                        className={`theme-card ${draft.application.theme === "studio" ? "is-selected" : ""} ${settingsBusy ? "is-disabled" : ""}`}
                        onClick={() =>
                          updateDraft((current) => ({
                            ...current,
                            application: {
                              ...current.application,
                              theme: "studio",
                            },
                          }))
                        }
                        disabled={settingsBusy}
                        role="radio"
                        aria-checked={draft.application.theme === "studio"}
                      >
                        <div className="theme-card-header">
                          <div className="theme-title-group">
                            <span className="theme-radio-dot" aria-hidden="true" />
                            <strong className="theme-name">Silk Studio</strong>
                          </div>
                          <span className="theme-tag">Default</span>
                        </div>
                        <div className="theme-swatch-row" aria-hidden="true">
                          <span className="theme-swatch swatch-studio-bg" />
                          <span className="theme-swatch swatch-studio-surface" />
                          <span className="theme-swatch swatch-studio-accent" />
                        </div>
                        <p className="theme-description">Deep obsidian violet with electric purple & neon accents.</p>
                      </button>

                      <button
                        type="button"
                        className={`theme-card ${draft.application.theme === "classic" ? "is-selected" : ""} ${settingsBusy ? "is-disabled" : ""}`}
                        onClick={() =>
                          updateDraft((current) => ({
                            ...current,
                            application: {
                              ...current.application,
                              theme: "classic",
                            },
                          }))
                        }
                        disabled={settingsBusy}
                        role="radio"
                        aria-checked={draft.application.theme === "classic"}
                      >
                        <div className="theme-card-header">
                          <div className="theme-title-group">
                            <span className="theme-radio-dot" aria-hidden="true" />
                            <strong className="theme-name">Silk (Legacy)</strong>
                          </div>
                          <span className="theme-tag">Gold</span>
                        </div>
                        <div className="theme-swatch-row" aria-hidden="true">
                          <span className="theme-swatch swatch-classic-bg" />
                          <span className="theme-swatch swatch-classic-surface" />
                          <span className="theme-swatch swatch-classic-accent" />
                        </div>
                        <p className="theme-description">Original obsidian-olive with butter yellow typography.</p>
                      </button>

                      <button
                        type="button"
                        className={`theme-card ${draft.application.theme === "ember" ? "is-selected" : ""} ${settingsBusy ? "is-disabled" : ""}`}
                        onClick={() =>
                          updateDraft((current) => ({
                            ...current,
                            application: {
                              ...current.application,
                              theme: "ember",
                            },
                          }))
                        }
                        disabled={settingsBusy}
                        role="radio"
                        aria-checked={draft.application.theme === "ember"}
                      >
                        <div className="theme-card-header">
                          <div className="theme-title-group">
                            <span className="theme-radio-dot" aria-hidden="true" />
                            <strong className="theme-name">Autumn Ember</strong>
                          </div>
                          <span className="theme-tag">Amber</span>
                        </div>
                        <div className="theme-swatch-row" aria-hidden="true">
                          <span className="theme-swatch swatch-ember-bg" />
                          <span className="theme-swatch swatch-ember-surface" />
                          <span className="theme-swatch swatch-ember-accent" />
                        </div>
                        <p className="theme-description">Roasted chestnut and timber with glowing amber & pale mint text.</p>
                      </button>

                      <button
                        type="button"
                        className={`theme-card ${draft.application.theme === "vamp" ? "is-selected" : ""} ${settingsBusy ? "is-disabled" : ""}`}
                        onClick={() =>
                          updateDraft((current) => ({
                            ...current,
                            application: {
                              ...current.application,
                              theme: "vamp",
                            },
                          }))
                        }
                        disabled={settingsBusy}
                        role="radio"
                        aria-checked={draft.application.theme === "vamp"}
                      >
                        <div className="theme-card-header">
                          <div className="theme-title-group">
                            <span className="theme-radio-dot" aria-hidden="true" />
                            <strong className="theme-name">Vamp</strong>
                          </div>
                          <span className="theme-tag">Crimson</span>
                        </div>
                        <div className="theme-swatch-row" aria-hidden="true">
                          <span className="theme-swatch swatch-vamp-bg" />
                          <span className="theme-swatch swatch-vamp-surface" />
                          <span className="theme-swatch swatch-vamp-accent" />
                        </div>
                        <p className="theme-description">Obsidian black and slate grey with crimson & soft coral red text.</p>
                      </button>
                    </div>
                  </div>

                  <div className="pane-section-header" style={{ marginTop: "24px" }}>
                    <h3>System Behavior</h3>
                    <p>Windows startup and system tray integration.</p>
                  </div>

                  <div className="settings-group-card">
                    <label className="check-row">
                      <input
                        type="checkbox"
                        checked={draft.application.minimizeToTray}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            application: {
                              ...current.application,
                              minimizeToTray: event.target.checked,
                            },
                          }))
                        }
                        disabled={settingsBusy}
                      />
                      <span>Minimize to system tray when the window is closed</span>
                    </label>

                    <label className="check-row">
                      <input
                        type="checkbox"
                        checked={draft.application.startWithWindows}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            application: {
                              ...current.application,
                              startWithWindows: event.target.checked,
                            },
                          }))
                        }
                        disabled={settingsBusy}
                      />
                      <span>Start Silk automatically when Windows logs in</span>
                    </label>
                  </div>
                </div>
              )}

              {/* TAB 7: DIAGNOSTICS */}
              {activeTab === "diagnostics" && (
                <div className="settings-pane-section">
                  <div className="pane-section-header">
                    <h3>Diagnostic Logs & Hardware Probe</h3>
                    <p>Inspect detected graphics cards, hardware encoders, and export logs.</p>
                  </div>

                  <div className="settings-group-card">
                    <label className="field">
                      <span>Logging level</span>
                      <select
                        value={draft.diagnostics.loggingLevel}
                        onChange={(event) =>
                          updateDraft((current) => ({
                            ...current,
                            diagnostics: {
                              loggingLevel: event.target
                                .value as AppSettings["diagnostics"]["loggingLevel"],
                            },
                          }))
                        }
                        disabled={settingsBusy}
                      >
                        <option value="error">Errors only</option>
                        <option value="warn">Warnings and errors</option>
                        <option value="info">Normal (Info)</option>
                        <option value="debug">Debug</option>
                        <option value="trace">Trace (Verbose)</option>
                      </select>
                    </label>

                    <div className="diagnostic-actions-row">
                      <button
                        className="button button-secondary"
                        type="button"
                        onClick={() => void createDiagnosticPackage()}
                        disabled={diagnosticBusy}
                      >
                        {diagnosticBusy ? "Collecting..." : "Export diagnostic package"}
                      </button>
                      {diagnosticMessage && (
                        <span className="form-status" role="status" aria-live="polite">
                          {diagnosticMessage}
                        </span>
                      )}
                    </div>

                    {diagnosticInfo && (
                      <div className="diagnostic-summary-box">
                        <div className="diag-item">
                          <small>Environment</small>
                          <strong>
                            Silk {diagnosticInfo.appVersion} • {diagnosticInfo.operatingSystem} ({diagnosticInfo.architecture})
                          </strong>
                        </div>
                        <div className="diag-item">
                          <small>Capture Backend</small>
                          <strong>
                            Windows Graphics Capture (WGC): {diagnosticInfo.wgcSupported ? "Available" : "Unavailable"}
                          </strong>
                        </div>
                        <div className="diag-item">
                          <small>Discovered Encoders</small>
                          <strong>
                            {diagnosticInfo.encoderBackends.length} backend
                            {diagnosticInfo.encoderBackends.length === 1 ? "" : "s"} (
                            {diagnosticInfo.encoderBackends.map((b) => b.backendName).join(", ")})
                          </strong>
                        </div>
                        {diagnosticInfo.fidelityCapabilities &&
                          diagnosticInfo.fidelityCapabilities.length > 0 && (
                            <div className="diag-item">
                              <small>Fidelity Capabilities</small>
                              <strong>
                                {diagnosticInfo.fidelityCapabilities
                                  .map(
                                    (cap) =>
                                      `${formatVideoFidelityMode(cap.mode)} (${cap.availability.replace(/_/g, " ")})`,
                                  )
                                  .join(", ")}
                              </strong>
                            </div>
                          )}
                      </div>
                    )}
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* Modal Sticky Footer */}
          <div className="settings-modal-footer">
            <div className="settings-footer-left">
              {onDiscardChanges && isSettingsDirty && (
                <button
                  type="button"
                  className="button button-quiet compact-button"
                  onClick={onDiscardChanges}
                  disabled={settingsBusy}
                >
                  Discard Changes
                </button>
              )}
              {settingsMessage && (
                <span className="settings-footer-msg" role="status" aria-live="polite">
                  {settingsMessage}
                </span>
              )}
            </div>

            <div className="settings-footer-right">
              <button
                type="button"
                className="button button-secondary"
                onClick={onClose}
                disabled={settingsBusy}
              >
                Close
              </button>
              <button
                type="submit"
                className="button button-primary settings-save-btn"
                disabled={settingsBusy || directoryBusy}
              >
                {settingsBusy ? "Saving Settings..." : "Save Settings"}
              </button>
            </div>
          </div>
        </form>
      </div>
    </div>
  );
}
