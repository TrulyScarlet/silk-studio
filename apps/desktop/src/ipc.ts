import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AudioDeviceInfo,
  AppSettings,
  ClipAudioTrackInfo,
  ClipEntry,
  Command,
  ControllerEvent,
  DiagnosticBuildInfo,
  DisplaySourceInfo,
  HotkeysSettings,
  RecorderStatus,
  StorageSummary,
  TrimClipRequest,
  WindowSourceInfo,
} from "./types";

/// Send a recorder command through the native allowlisted bridge.
export async function send(command: Command): Promise<void> {
  await invoke("recorder_command", { command });
}

/// Round-trip probe used by the S0 IPC verification.
export async function ping(message: string): Promise<string> {
  return invoke<string>("ping", { message });
}

/// Validate and apply hotkey settings without restarting the recorder.
export async function configureHotkeys(settings: HotkeysSettings): Promise<void> {
  await invoke("configure_hotkeys", { settings });
}

/// Read the validated hotkey subset loaded by the native shell.
export async function getHotkeySettings(): Promise<HotkeysSettings> {
  return invoke<HotkeysSettings>("get_hotkey_settings");
}

/// Read the complete native settings document.
export async function getSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_settings");
}

/// Validate and persist the complete settings document atomically.
export async function configureSettings(settings: AppSettings): Promise<void> {
  await invoke("configure_settings", { settings });
}

/// Enumerate active displays available for monitor capture.
export async function getDisplaySources(): Promise<DisplaySourceInfo[]> {
  return invoke<DisplaySourceInfo[]>("get_display_sources");
}

/// Enumerate active top-level windows available for window capture.
export async function getWindowSources(): Promise<WindowSourceInfo[]> {
  return invoke<WindowSourceInfo[]>("get_window_sources");
}

/// Enumerate active Windows audio endpoints for the track editor.
export async function getAudioDevices(): Promise<AudioDeviceInfo[]> {
  return invoke<AudioDeviceInfo[]>("get_audio_devices");
}

/// Read the latest recorder state and bounded pipeline counters.
export async function getRecorderStatus(): Promise<RecorderStatus> {
  return invoke<RecorderStatus>("get_recorder_status");
}

/// Reconcile the output directory and return index rows.
export async function listClips(): Promise<ClipEntry[]> {
  return invoke<ClipEntry[]>("list_clips");
}

export async function renameClip(path: string, name: string): Promise<ClipEntry> {
  return invoke<ClipEntry>("rename_clip", { path, name });
}

export async function deleteClip(path: string): Promise<void> {
  await invoke("delete_clip", { path });
}

export async function revealClip(path: string): Promise<void> {
  await invoke("reveal_clip", { path });
}

export async function openClip(path: string): Promise<void> {
  await invoke("open_clip", { path });
}

export async function openUrl(url: string): Promise<void> {
  if (isMockMode()) {
    window.open(url, "_blank");
    return;
  }
  await invoke("open_url", { url });
}

export async function trimClip(request: TrimClipRequest): Promise<string> {
  return invoke<string>("trim_clip", { request });
}

export async function getClipKeyframes(path: string): Promise<number[]> {
  if (isMockMode()) {
    return [0, 2, 4, 6, 8, 10];
  }
  return invoke<number[]>("get_clip_keyframes", { path });
}

export async function getClipAudioTracks(path: string): Promise<ClipAudioTrackInfo[]> {
  if (isMockMode()) {
    return [
      { trackId: 1, name: "Game Audio", channels: 2, sampleRate: 48000 },
      { trackId: 2, name: "Microphone", channels: 2, sampleRate: 48000 },
    ];
  }
  return invoke<ClipAudioTrackInfo[]>("get_clip_audio_tracks", { path });
}

export async function extractClipAudioTrack(clipPath: string, trackId: number): Promise<string> {
  if (isMockMode()) {
    return "";
  }
  return invoke<string>("extract_clip_audio_track", { clipPath, trackId });
}

export async function updateThemeIcon(theme: string): Promise<void> {
  if (isMockMode()) return;
  await invoke("update_theme_icon", { theme });
}

export async function setClipProtected(path: string, protectedClip: boolean): Promise<ClipEntry> {
  return invoke<ClipEntry>("set_clip_protected", { path, protected: protectedClip });
}

export async function getStorageSummary(): Promise<StorageSummary> {
  return invoke<StorageSummary>("get_storage_summary");
}

/// Open the native clip-directory picker. A null result means the user cancelled.
export async function pickClipDirectory(): Promise<string | null> {
  return invoke<string | null>("pick_clip_directory");
}

export async function getDiagnosticInfo(): Promise<DiagnosticBuildInfo> {
  return invoke<DiagnosticBuildInfo>("get_diagnostic_info");
}

export async function exportDiagnostics(): Promise<string> {
  return invoke<string>("export_diagnostics");
}

export function isMockMode(): boolean {
  return typeof window === "undefined" || !("__TAURI_INTERNALS__" in window);
}

/// Trigger a transient preview of the native capture confirmation HUD.
export async function testOverlay(): Promise<void> {
  if (!isMockMode()) {
    await invoke("test_overlay");
  }
}

/// Subscribe to controller events forwarded from the engine.
export function onControllerEvent(
  handler: (event: ControllerEvent) => void,
): Promise<UnlistenFn> {
  return listen<ControllerEvent>("controller-event", (e) => handler(e.payload));
}
