import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { ClipEntry, ClipMarker } from "./types";
import {
  extractClipAudioTrack,
  getClipAudioTracks,
  getClipKeyframes,
  isMockMode,
  openUrl,
  trimClip,
} from "./ipc";

type ClipPlayerModalProps = {
  clip: ClipEntry;
  onClose: () => void;
  onRefreshLibrary: () => Promise<void>;
  onOpenExternal: (clip: ClipEntry) => Promise<void>;
};

const MARKER_COLORS = [
  { hex: "#ef4444", label: "Red (Death / Mistake)" },
  { hex: "#22c55e", label: "Green (Kill / Highlight)" },
  { hex: "#eab308", label: "Yellow (Strategy / Rotation)" },
  { hex: "#3b82f6", label: "Blue (Utility / Callout)" },
  { hex: "#a855f7", label: "Purple (Key Moment)" },
];

function formatTime(seconds: number): string {
  if (isNaN(seconds) || seconds < 0) return "00:00.0";
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  const frac = Math.floor((seconds % 1) * 10);
  return `${mins.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}.${frac}`;
}

type ManagedAudioTrack = {
  trackId: number;
  name: string;
  channels: number;
  sampleRate: number;
  volume: number; // 0.0 to 1.0
  isMuted: boolean;
  src?: string; // For additional audio tracks (trackId > 1)
};

export function ClipPlayerModal({
  clip,
  onClose,
  onRefreshLibrary,
  onOpenExternal,
}: ClipPlayerModalProps): JSX.Element {
  const modalContainerRef = useRef<HTMLDivElement>(null);
  const videoStageRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const progressTrackRef = useRef<HTMLDivElement>(null);
  const extraAudioElsRef = useRef<{ [trackId: number]: HTMLAudioElement | null }>({});
  const isDraggingScrubberRef = useRef<boolean>(false);
  const pendingSeekTimeRef = useRef<number | null>(null);
  const rafSeekIdRef = useRef<number | null>(null);

  const [videoSrc, setVideoSrc] = useState<string>("");
  const [isPlaying, setIsPlaying] = useState<boolean>(false);
  const [currentTime, setCurrentTime] = useState<number>(0);
  const initialDuration = (clip.durationMs ?? 60000) / 1000;
  const [duration, setDuration] = useState<number>(initialDuration);
  const [playbackRate, setPlaybackRate] = useState<number>(1.0);
  const [masterVolume, setMasterVolume] = useState<number>(1.0);
  const [isMasterMuted, setIsMasterMuted] = useState<boolean>(false);
  const [videoDecodeError, setVideoDecodeError] = useState<string | null>(null);
  const [switchedToH264, setSwitchedToH264] = useState<boolean>(false);

  // Layout states: Theater Mode & Fullscreen
  const [isTheaterMode, setIsTheaterMode] = useState<boolean>(false);
  const [isFullscreen, setIsFullscreen] = useState<boolean>(false);

  // Multi-track audio mixer state (open by default when audio tracks exist)
  const [audioTracks, setAudioTracks] = useState<ManagedAudioTrack[]>([]);
  const [mixerOpen, setMixerOpen] = useState<boolean>(true);

  // VOD Markers state
  const storageKey = `silk_vod_markers_${clip.path}`;
  const [markers, setMarkers] = useState<ClipMarker[]>(() => {
    try {
      const saved = localStorage.getItem(storageKey);
      return saved ? (JSON.parse(saved) as ClipMarker[]) : [];
    } catch {
      return [];
    }
  });
  const [selectedColor, setSelectedColor] = useState<string>(MARKER_COLORS[0].hex);
  const [markerLabel, setMarkerLabel] = useState<string>("");
  const [markerNote, setMarkerNote] = useState<string>("");

  // Trimmer & Keyframes state
  const [trimActive, setTrimActive] = useState<boolean>(false);
  const [trimStart, setTrimStart] = useState<number>(0);
  const [trimEnd, setTrimEnd] = useState<number>(initialDuration);
  const [selectedTrimTracks, setSelectedTrimTracks] = useState<number[]>([]);
  const [keyframes, setKeyframes] = useState<number[]>([]);
  const [snapToKeyframe, setSnapToKeyframe] = useState<boolean>(true);
  const [isTrimming, setIsTrimming] = useState<boolean>(false);
  const [trimMessage, setTrimMessage] = useState<string | null>(null);

  // Initialize Video source, Keyframes, and Audio Tracks
  useEffect(() => {
    if (!clip.missing && clip.path) {
      try {
        const url = isMockMode() ? "" : convertFileSrc(clip.path);
        setVideoSrc(url);
      } catch {
        setVideoSrc("");
      }

      // 1. Fetch keyframes
      void getClipKeyframes(clip.path)
        .then((kfs) => setKeyframes(kfs))
        .catch(() => {});

      // 2. Fetch all discrete audio tracks
      void getClipAudioTracks(clip.path)
        .then(async (tracks) => {
          const mapped: ManagedAudioTrack[] = tracks.map((t) => ({
            trackId: t.trackId,
            name: t.name,
            channels: t.channels,
            sampleRate: t.sampleRate,
            volume: 1.0,
            isMuted: false,
          }));

          setAudioTracks(mapped);
          setSelectedTrimTracks(mapped.map((_, idx) => idx)); // 0-indexed for trimmer

          // For any additional audio tracks beyond Track 1 (e.g. Mic, Discord, Music),
          // extract their audio stream paths so secondary <audio> DOM elements can play them
          if (mapped.length > 1) {
            const secondaryTracks = mapped.slice(1);
            for (const t of secondaryTracks) {
              try {
                const path = await extractClipAudioTrack(clip.path, t.trackId);
                const srcUrl = convertFileSrc(path);
                setAudioTracks((prev) =>
                  prev.map((item) => (item.trackId === t.trackId ? { ...item, src: srcUrl } : item))
                );
              } catch (err) {
                console.warn(`Failed extracting secondary audio track ${t.trackId}:`, err);
              }
            }
          }
        })
        .catch(() => {});
    }
  }, [clip.path, clip.missing]);

  // Immediate resource cleanup on unmount to prevent background decoding or network lag
  useEffect(() => {
    return () => {
      if (rafSeekIdRef.current) {
        cancelAnimationFrame(rafSeekIdRef.current);
        rafSeekIdRef.current = null;
      }
      if (videoRef.current) {
        videoRef.current.pause();
        videoRef.current.removeAttribute("src");
        videoRef.current.load();
      }
      Object.values(extraAudioElsRef.current).forEach((el) => {
        if (el) {
          el.pause();
          el.removeAttribute("src");
          el.load();
        }
      });
      extraAudioElsRef.current = {};
    };
  }, []);

  // Listen for native window fullscreen changes
  useEffect(() => {
    let mounted = true;
    const checkFs = async () => {
      try {
        const fs = await invoke<boolean>("app_window_is_fullscreen");
        if (mounted) setIsFullscreen(fs);
      } catch {
        // ignore
      }
    };
    void checkFs();

    const handleResize = () => {
      void checkFs();
    };
    window.addEventListener("resize", handleResize);

    return () => {
      mounted = false;
      window.removeEventListener("resize", handleResize);
      void invoke("app_window_set_fullscreen", { fullscreen: false }).catch(() => {});
    };
  }, []);

  // Keyboard shortcuts (Space = Play/Pause, T = Theater, F = Fullscreen, M = Mute)
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") return;

      if (e.code === "Space") {
        e.preventDefault();
        togglePlay();
      } else if (e.code === "KeyT") {
        e.preventDefault();
        setIsTheaterMode((prev) => !prev);
      } else if (e.code === "KeyF") {
        e.preventDefault();
        handleToggleFullscreen();
      } else if (e.code === "KeyM") {
        e.preventDefault();
        toggleMute();
      } else if (e.code === "ArrowLeft") {
        e.preventDefault();
        handleSeek(currentTime - 5);
      } else if (e.code === "ArrowRight") {
        e.preventDefault();
        handleSeek(currentTime + 5);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [currentTime, isPlaying, audioTracks, masterVolume, isMasterMuted]);

  // Persist markers whenever updated
  useEffect(() => {
    try {
      localStorage.setItem(storageKey, JSON.stringify(markers));
    } catch {
      // Storage unavailable
    }
  }, [markers, storageKey]);

  // Video event handlers
  const handleLoadedMetadata = () => {
    setVideoDecodeError(null);
    if (videoRef.current) {
      const dur = videoRef.current.duration;
      if (dur > 0 && !isNaN(dur)) {
        setDuration(dur);
        if (trimEnd === 0 || trimEnd > dur) {
          setTrimEnd(dur);
        }
      }
      // Set initial volume on Track 1
      const track1 = audioTracks[0];
      const isMuted = isMasterMuted || (track1?.isMuted ?? false);
      const vol = isMuted ? 0 : (track1?.volume ?? 1.0) * masterVolume;
      videoRef.current.muted = isMuted;
      videoRef.current.volume = vol;
    }
  };

  const handleVideoError = () => {
    const error = videoRef.current?.error;
    if (error && error.code === 4) {
      setVideoDecodeError(
        "Cannot decode video stream. This clip may be encoded in HEVC (H.265), which requires the free Windows hardware video extension, or your display driver could not decode this resolution."
      );
    } else {
      setVideoDecodeError(
        `Video playback failed (error code ${error?.code ?? "unknown"}). Try opening in an external player.`
      );
    }
  };

  const handleOpenStoreExtension = async () => {
    try {
      await openUrl("ms-windows-store://pdp/?ProductId=9n4wgh0z6vhq");
    } catch {
      await openUrl("https://apps.microsoft.com/detail/9n4wgh0z6vhq");
    }
  };

  const handleSwitchToH264 = async () => {
    try {
      const current = await getClipAudioTracks(clip.path);
      if (current) setSwitchedToH264(true);
    } catch {
      // Failed to switch
    }
  };

  // Video buffering and seek synchronization to lock audio and video in frame sync
  const handleVideoSeeking = () => {
    Object.values(extraAudioElsRef.current).forEach((el) => el?.pause());
  };

  const handleVideoSeeked = () => {
    if (!videoRef.current) return;
    const cur = videoRef.current.currentTime;
    Object.values(extraAudioElsRef.current).forEach((el) => {
      if (el) {
        el.currentTime = cur;
        if (isPlaying && !videoRef.current?.paused) {
          void el.play().catch(() => {});
        }
      }
    });
  };

  const handleVideoWaiting = () => {
    Object.values(extraAudioElsRef.current).forEach((el) => el?.pause());
  };

  const handleVideoPlaying = () => {
    if (!videoRef.current) return;
    const cur = videoRef.current.currentTime;
    Object.values(extraAudioElsRef.current).forEach((el) => {
      if (el) {
        el.currentTime = cur;
        void el.play().catch(() => {});
      }
    });
    setIsPlaying(true);
  };

  const handleTimeUpdate = () => {
    if (videoRef.current) {
      const cur = videoRef.current.currentTime;
      setCurrentTime(cur);

      // Keep secondary audio tracks in tight sync with video
      Object.values(extraAudioElsRef.current).forEach((el) => {
        if (el && Math.abs(el.currentTime - cur) > 0.08) {
          el.currentTime = cur;
        }
      });
    }
  };

  const togglePlay = () => {
    if (!videoRef.current) return;
    if (isPlaying) {
      videoRef.current.pause();
      Object.values(extraAudioElsRef.current).forEach((el) => el?.pause());
      setIsPlaying(false);
    } else {
      const cur = videoRef.current.currentTime;
      Object.values(extraAudioElsRef.current).forEach((el) => {
        if (el) {
          el.currentTime = cur;
          void el.play().catch(() => {});
        }
      });
      void videoRef.current.play().then(() => {
        setIsPlaying(true);
      });
    }
  };

  // Throttled video seeking using requestAnimationFrame and fastSeek
  const performVideoSeek = (time: number, isFinal: boolean) => {
    const video = videoRef.current;
    if (!video) return;

    // Immediately pause secondary audio so it never plays ahead while video seeks
    Object.values(extraAudioElsRef.current).forEach((el) => el?.pause());

    if (!isFinal && typeof (video as unknown as { fastSeek?: (t: number) => void }).fastSeek === "function") {
      try {
        (video as unknown as { fastSeek: (t: number) => void }).fastSeek(time);
        return;
      } catch {
        // fallback
      }
    }

    video.currentTime = time;
  };

  const scheduleSeek = (time: number, isFinal: boolean) => {
    const clamped = Math.max(0, Math.min(duration, time));
    setCurrentTime(clamped);
    pendingSeekTimeRef.current = clamped;

    if (isFinal) {
      if (rafSeekIdRef.current) {
        cancelAnimationFrame(rafSeekIdRef.current);
        rafSeekIdRef.current = null;
      }
      performVideoSeek(clamped, true);
    } else if (!rafSeekIdRef.current) {
      rafSeekIdRef.current = requestAnimationFrame(() => {
        rafSeekIdRef.current = null;
        if (pendingSeekTimeRef.current !== null) {
          performVideoSeek(pendingSeekTimeRef.current, false);
        }
      });
    }
  };

  const handleSeek = (newTime: number) => {
    scheduleSeek(newTime, true);
  };

  const calculateTrackTime = (clientX: number) => {
    if (!progressTrackRef.current || duration <= 0) return 0;
    const rect = progressTrackRef.current.getBoundingClientRect();
    const clickX = clientX - rect.left;
    const percentage = Math.max(0, Math.min(1, clickX / rect.width));
    return percentage * duration;
  };

  const handleTrackMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    isDraggingScrubberRef.current = true;
    const targetTime = calculateTrackTime(e.clientX);
    scheduleSeek(targetTime, false);

    const onMouseMove = (moveEvent: globalThis.MouseEvent) => {
      if (!isDraggingScrubberRef.current) return;
      const t = calculateTrackTime(moveEvent.clientX);
      scheduleSeek(t, false);
    };

    const onMouseUp = (upEvent: globalThis.MouseEvent) => {
      isDraggingScrubberRef.current = false;
      const t = calculateTrackTime(upEvent.clientX);
      scheduleSeek(t, true);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  };

  const handleSpeedChange = (rate: number) => {
    setPlaybackRate(rate);
    if (videoRef.current) {
      videoRef.current.playbackRate = rate;
    }
    Object.values(extraAudioElsRef.current).forEach((el) => {
      if (el) el.playbackRate = rate;
    });
  };

  // Master Volume & Mute (Scales all audio tracks)
  const handleMasterVolumeChange = (newVol: number) => {
    setMasterVolume(newVol);
    setIsMasterMuted(newVol === 0);

    // Update Track 1 on <video>
    if (videoRef.current) {
      const track1 = audioTracks[0];
      const isMuted = newVol === 0 || (track1?.isMuted ?? false);
      videoRef.current.muted = isMuted;
      videoRef.current.volume = isMuted ? 0 : (track1?.volume ?? 1.0) * newVol;
    }

    // Update secondary tracks on <audio> elements
    audioTracks.slice(1).forEach((t) => {
      const el = extraAudioElsRef.current[t.trackId];
      if (el) {
        const isMuted = newVol === 0 || t.isMuted;
        el.muted = isMuted;
        el.volume = isMuted ? 0 : t.volume * newVol;
      }
    });
  };

  const toggleMute = () => {
    if (isMasterMuted) {
      setIsMasterMuted(false);
      handleMasterVolumeChange(masterVolume > 0 ? masterVolume : 1.0);
    } else {
      setIsMasterMuted(true);
      if (videoRef.current) {
        videoRef.current.muted = true;
      }
      Object.values(extraAudioElsRef.current).forEach((el) => {
        if (el) el.muted = true;
      });
    }
  };

  // Per-Track Audio Mixer Controls
  const handleTrackVolumeChange = (trackId: number, vol: number) => {
    setAudioTracks((prev) =>
      prev.map((t) => {
        if (t.trackId === trackId) {
          const effectiveVol = t.isMuted || isMasterMuted ? 0 : vol * masterVolume;
          if (trackId === prev[0]?.trackId) {
            // Track 1 is played by <video>
            if (videoRef.current) {
              videoRef.current.volume = effectiveVol;
            }
          } else {
            // Secondary tracks played by <audio>
            const el = extraAudioElsRef.current[trackId];
            if (el) {
              el.volume = effectiveVol;
            }
          }
          return { ...t, volume: vol };
        }
        return t;
      })
    );
  };

  const handleToggleTrackMute = (trackId: number) => {
    setAudioTracks((prev) =>
      prev.map((t) => {
        if (t.trackId === trackId) {
          const nextMuted = !t.isMuted;
          const effectiveVol = nextMuted || isMasterMuted ? 0 : t.volume * masterVolume;
          if (trackId === prev[0]?.trackId) {
            if (videoRef.current) {
              videoRef.current.muted = nextMuted || isMasterMuted;
              videoRef.current.volume = effectiveVol;
            }
          } else {
            const el = extraAudioElsRef.current[trackId];
            if (el) {
              el.muted = nextMuted || isMasterMuted;
              el.volume = effectiveVol;
            }
          }
          return { ...t, isMuted: nextMuted };
        }
        return t;
      })
    );
  };

  const handleSoloTrack = (trackId: number) => {
    setAudioTracks((prev) =>
      prev.map((t) => {
        const shouldMute = t.trackId !== trackId;
        const effectiveVol = shouldMute || isMasterMuted ? 0 : t.volume * masterVolume;
        if (t.trackId === prev[0]?.trackId) {
          if (videoRef.current) {
            videoRef.current.muted = shouldMute || isMasterMuted;
            videoRef.current.volume = effectiveVol;
          }
        } else {
          const el = extraAudioElsRef.current[t.trackId];
          if (el) {
            el.muted = shouldMute || isMasterMuted;
            el.volume = effectiveVol;
          }
        }
        return { ...t, isMuted: shouldMute };
      })
    );
  };

  const handleUnmuteAllTracks = () => {
    setAudioTracks((prev) =>
      prev.map((t) => {
        const effectiveVol = isMasterMuted ? 0 : t.volume * masterVolume;
        if (t.trackId === prev[0]?.trackId) {
          if (videoRef.current) {
            videoRef.current.muted = isMasterMuted;
            videoRef.current.volume = effectiveVol;
          }
        } else {
          const el = extraAudioElsRef.current[t.trackId];
          if (el) {
            el.muted = isMasterMuted;
            el.volume = effectiveVol;
          }
        }
        return { ...t, isMuted: false };
      })
    );
  };

  // Native Tauri window fullscreen toggle
  const handleToggleFullscreen = async () => {
    try {
      const nextFs = !isFullscreen;
      await invoke("app_window_set_fullscreen", { fullscreen: nextFs });
      setIsFullscreen(nextFs);
    } catch (err) {
      console.warn("Failed to toggle native fullscreen:", err);
    }
  };

  // VOD Marker handlers
  const handleAddMarker = () => {
    const label = markerLabel.trim() || `Bookmark ${markers.length + 1}`;
    const newMarker: ClipMarker = {
      id: `marker_${Date.now()}_${Math.random().toString(36).slice(2, 6)}`,
      timestampSeconds: currentTime,
      color: selectedColor,
      label,
      note: markerNote.trim() || undefined,
      createdAt: Date.now(),
    };
    setMarkers((prev) => [...prev, newMarker].sort((a, b) => a.timestampSeconds - b.timestampSeconds));
    setMarkerLabel("");
    setMarkerNote("");
  };

  const handleDeleteMarker = (id: string) => {
    setMarkers((prev) => prev.filter((m) => m.id !== id));
  };

  // Trimming handlers
  const handleSetTrimStart = () => {
    let start = Math.min(currentTime, trimEnd - 0.5);
    if (snapToKeyframe && keyframes.length > 0) {
      let closest = keyframes[0];
      let minDiff = Math.abs(closest - start);
      for (const kf of keyframes) {
        if (kf <= start + 0.1) {
          const diff = Math.abs(kf - start);
          if (diff < minDiff) {
            minDiff = diff;
            closest = kf;
          }
        }
      }
      start = closest;
    }
    setTrimStart(Math.max(0, start));
  };

  const handleSetTrimEnd = () => {
    const end = Math.max(currentTime, trimStart + 0.5);
    setTrimEnd(Math.min(duration, end));
  };

  const handleToggleTrimTrack = (trackIndex: number) => {
    setSelectedTrimTracks((prev) =>
      prev.includes(trackIndex) ? prev.filter((idx) => idx !== trackIndex) : [...prev, trackIndex].sort((a, b) => a - b)
    );
  };

  const handleExecuteTrim = async () => {
    if (trimEnd <= trimStart) {
      setTrimMessage("Trim end must be after trim start.");
      return;
    }

    setIsTrimming(true);
    setTrimMessage("Trimming clip losslessly in pure Rust...");

    try {
      const extIndex = clip.path.lastIndexOf(".");
      const basePath = extIndex > 0 ? clip.path.slice(0, extIndex) : clip.path;
      const ext = extIndex > 0 ? clip.path.slice(extIndex) : ".mp4";
      const outputPath = `${basePath}_trim_${Date.now()}${ext}`;

      const allTracks = audioTracks.map((_, i) => i);
      const isAllSelected =
        selectedTrimTracks.length === allTracks.length &&
        selectedTrimTracks.every((val, idx) => val === allTracks[idx]);

      await trimClip({
        sourcePath: clip.path,
        outputPath,
        startSeconds: trimStart,
        endSeconds: trimEnd,
        includedAudioTracks: isAllSelected ? undefined : selectedTrimTracks,
      });

      setTrimMessage("✓ Trim complete! New clip saved to library.");
      await onRefreshLibrary();
    } catch (err) {
      setTrimMessage(`Trimming failed: ${String(err)}`);
    } finally {
      setIsTrimming(false);
    }
  };

  return (
    <div
      className={`clip-modal-backdrop ${isTheaterMode ? "is-theater-backdrop" : ""}`}
      onClick={onClose}
      role="dialog"
      aria-modal="true"
    >
      <div
        ref={modalContainerRef}
        className={`clip-modal-container ${isTheaterMode ? "is-theater-container" : ""} ${isFullscreen ? "is-fullscreen-container" : ""}`}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Hidden Secondary Audio Elements for Multi-Track Playback */}
        {audioTracks.slice(1).map((track) => {
          if (!track.src) return null;
          return (
            <audio
              key={track.trackId}
              ref={(el) => {
                extraAudioElsRef.current[track.trackId] = el;
              }}
              src={track.src}
              preload="auto"
            />
          );
        })}

        {/* Header */}
        <div className="clip-modal-header">
          <div className="clip-modal-title-group">
            <h2>{clip.name}</h2>
            <span className="clip-modal-meta">
              {formatTime(duration)} • {clip.path}
            </span>
          </div>
          <div className="clip-modal-actions">
            <button
              type="button"
              className={`button button-small ${isTheaterMode ? "button-accent" : "button-secondary"}`}
              onClick={() => setIsTheaterMode(!isTheaterMode)}
              title="Toggle Theater Mode (T)"
            >
              {isTheaterMode ? "⬚ Standard View" : "⬚ Theater Mode"}
            </button>
            <button
              type="button"
              className="button button-secondary button-small"
              onClick={() => void onOpenExternal(clip)}
              title="Open in your default media player"
            >
              Open External
            </button>
            <button
              type="button"
              className="button-close"
              onClick={onClose}
              aria-label="Close clip player"
            >
              ✕
            </button>
          </div>
        </div>

        {/* Theater / Standard Split Layout */}
        <div className={`clip-content-layout ${isTheaterMode ? "is-theater-layout" : ""}`}>
          {/* Main Stage (Player, Timeline, Controls, Audio Mixer, Trimmer) */}
          <div className="clip-main-stage">
            {/* Direct Fullscreen Video Stage (Video, Scrub Bar, Controls) */}
            <div
              ref={videoStageRef}
              className={`clip-video-stage ${isFullscreen ? "is-fullscreen-stage" : ""}`}
            >
              {/* Video Viewport */}
              <div
                className={`clip-player-viewport ${isTheaterMode ? "is-theater-viewport" : ""}`}
                onDoubleClick={handleToggleFullscreen}
                title="Double-click to toggle fullscreen"
              >
                {videoSrc ? (
                  <video
                    ref={videoRef}
                    className="clip-video-element"
                    src={videoSrc}
                    controls={false}
                    onLoadedMetadata={handleLoadedMetadata}
                    onTimeUpdate={handleTimeUpdate}
                    onSeeking={handleVideoSeeking}
                    onSeeked={handleVideoSeeked}
                    onWaiting={handleVideoWaiting}
                    onPlaying={handleVideoPlaying}
                    onError={handleVideoError}
                    onEnded={() => setIsPlaying(false)}
                    onClick={togglePlay}
                    playsInline
                  />
                ) : (
                  <div className="clip-player-fallback">
                    <p>Cannot stream media for preview in current mode.</p>
                  </div>
                )}

              {/* Codec Extension Missing Banner */}
              {videoDecodeError ? (
                <div className="clip-player-codec-banner">
                  <span className="codec-banner-icon" aria-hidden="true">⚠️</span>
                  <div className="codec-banner-body">
                    <strong>Playback Codec Notice</strong>
                    <p>{videoDecodeError}</p>
                    <div className="codec-banner-actions">
                      <button
                        type="button"
                        className="button button-accent button-small"
                        onClick={() => void handleOpenStoreExtension()}
                      >
                        Install Free Windows Extension
                      </button>
                      <button
                        type="button"
                        className="button button-secondary button-small"
                        onClick={() => void onOpenExternal(clip)}
                      >
                        Watch in External Player
                      </button>
                      <button
                        type="button"
                        className="button button-secondary button-small"
                        onClick={() => void handleSwitchToH264()}
                        disabled={switchedToH264}
                      >
                        {switchedToH264 ? "✓ Switched to H.264" : "Switch Future Clips to H.264"}
                      </button>
                    </div>
                  </div>
                </div>
              ) : null}

              {/* Large center play button when paused */}
              {!isPlaying && videoSrc ? (
                <button
                  type="button"
                  className="clip-player-center-play"
                  onClick={togglePlay}
                  aria-label="Play clip"
                >
                  ▶
                </button>
              ) : null}
            </div>

            {/* Timeline Scrub Bar with Keyframe Ticks & Markers */}
            <div className="clip-timeline-wrapper">
              <div
                ref={progressTrackRef}
                className="clip-progress-track"
                onMouseDown={handleTrackMouseDown}
                role="slider"
                aria-valuemin={0}
                aria-valuemax={duration}
                aria-valuenow={currentTime}
                tabIndex={0}
              >
                {/* Trim region highlight */}
                {trimActive && duration > 0 ? (
                  <div
                    className="clip-trim-region"
                    style={{
                      left: `${(trimStart / duration) * 100}%`,
                      width: `${((trimEnd - trimStart) / duration) * 100}%`,
                    }}
                  />
                ) : null}

                {/* Keyframe tick marks */}
                {trimActive &&
                  duration > 0 &&
                  keyframes.map((kf, idx) => {
                    const posPercent = (kf / duration) * 100;
                    return (
                      <span
                        key={`kf_${idx}`}
                        className="clip-keyframe-tick"
                        style={{ left: `${posPercent}%` }}
                        title={`Keyframe at ${formatTime(kf)}`}
                        onClick={(e) => {
                          e.stopPropagation();
                          handleSeek(kf);
                        }}
                      />
                    );
                  })}

                {/* Current progress */}
                <div
                  className="clip-progress-fill"
                  style={{ width: `${duration > 0 ? (currentTime / duration) * 100 : 0}%` }}
                />

                {/* VOD Marker pins */}
                {markers.map((m) => {
                  const posPercent = duration > 0 ? (m.timestampSeconds / duration) * 100 : 0;
                  return (
                    <span
                      key={m.id}
                      className="clip-marker-pin"
                      style={{ left: `${posPercent}%`, backgroundColor: m.color }}
                      title={`${formatTime(m.timestampSeconds)} - ${m.label}${m.note ? `: ${m.note}` : ""}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        handleSeek(m.timestampSeconds);
                      }}
                    />
                  );
                })}
              </div>

              <div className="clip-timeline-timecodes">
                <span>{formatTime(currentTime)}</span>
                <span>{formatTime(duration)}</span>
              </div>
            </div>

            {/* Playback Controls Bar */}
            <div className="clip-controls-bar">
              <div className="clip-controls-left">
                <button
                  type="button"
                  className="button button-small button-primary"
                  onClick={togglePlay}
                  aria-label={isPlaying ? "Pause" : "Play"}
                >
                  {isPlaying ? "⏸ Pause" : "▶ Play"}
                </button>
                <button
                  type="button"
                  className="button button-small button-secondary"
                  onClick={() => handleSeek(currentTime - 5)}
                  title="Rewind 5s (Left Arrow)"
                >
                  -5s
                </button>
                <button
                  type="button"
                  className="button button-small button-secondary"
                  onClick={() => handleSeek(currentTime + 5)}
                  title="Forward 5s (Right Arrow)"
                >
                  +5s
                </button>

                {/* Playback Rate */}
                <div className="rate-selector-group">
                  {[0.5, 1.0, 1.5, 2.0].map((rate) => (
                    <button
                      key={rate}
                      type="button"
                      className={`chip-button ${playbackRate === rate ? "is-selected" : ""}`}
                      onClick={() => handleSpeedChange(rate)}
                    >
                      {rate}x
                    </button>
                  ))}
                </div>
              </div>

              <div className="clip-controls-right">
                {/* Audio Mixer Toggle */}
                {audioTracks.length > 0 ? (
                  <button
                    type="button"
                    className={`button button-small ${mixerOpen ? "button-accent" : "button-secondary"}`}
                    onClick={() => setMixerOpen(!mixerOpen)}
                    title="Audio Tracks Mixer"
                  >
                    🎛 Audio Tracks ({audioTracks.length})
                  </button>
                ) : null}

                {/* Master Volume */}
                <button
                  type="button"
                  className="icon-button"
                  onClick={toggleMute}
                  title={isMasterMuted ? "Unmute (M)" : "Mute (M)"}
                >
                  {isMasterMuted ? "🔇" : "🔊"}
                </button>
                <input
                  type="range"
                  className="volume-slider"
                  min={0}
                  max={1}
                  step={0.05}
                  value={isMasterMuted ? 0 : masterVolume}
                  onChange={(e) => handleMasterVolumeChange(parseFloat(e.target.value))}
                  aria-label="Master volume"
                />

                {/* Trimmer Toggle */}
                <button
                  type="button"
                  className={`button button-small ${trimActive ? "button-accent" : "button-secondary"}`}
                  onClick={() => setTrimActive(!trimActive)}
                >
                  ✂ {trimActive ? "Close Trimmer" : "Trim Clip"}
                </button>

                {/* Sleek Fullscreen Symbol Button */}
                <button
                  type="button"
                  className="icon-button clip-fullscreen-symbol-btn"
                  onClick={handleToggleFullscreen}
                  title={isFullscreen ? "Exit Fullscreen (F)" : "Fullscreen (F)"}
                  aria-label={isFullscreen ? "Exit Fullscreen" : "Fullscreen"}
                >
                  <span aria-hidden="true" style={{ fontSize: "16px", lineHeight: 1 }}>
                    {isFullscreen ? "🗗" : "⛶"}
                  </span>
                </button>
              </div>
            </div>
            </div> {/* End direct fullscreen video stage */}

            {/* Audio Mixer Studio Drawer (Displays all audio tracks playing together) */}
            {mixerOpen && audioTracks.length > 0 ? (
              <div className="audio-mixer-drawer">
                <div className="audio-mixer-header">
                  <div>
                    <strong>🎛 Audio Tracks Mixer ({audioTracks.length} tracks playing together)</strong>
                    <p className="audio-mixer-subtitle">
                      All enabled tracks play simultaneously. Adjust volume or mute individual tracks below.
                    </p>
                  </div>
                  <div className="audio-mixer-actions">
                    <button
                      type="button"
                      className="button button-small button-secondary"
                      onClick={handleUnmuteAllTracks}
                    >
                      Unmute All
                    </button>
                  </div>
                </div>

                <div className="audio-track-cards-grid">
                  {audioTracks.map((track, idx) => (
                    <div
                      key={track.trackId}
                      className={`audio-track-card ${track.isMuted ? "is-muted" : ""}`}
                    >
                      <div className="audio-track-info-row">
                        <span className="audio-track-label">
                          <strong>{track.name}</strong>
                          <small>Audio {idx + 1} • {track.channels === 1 ? "Mono" : "Stereo"}</small>
                        </span>
                        <div className="audio-track-buttons">
                          <button
                            type="button"
                            className="button button-small button-secondary audio-solo-btn"
                            onClick={() => handleSoloTrack(track.trackId)}
                            title={`Solo ${track.name} (mutes all others)`}
                          >
                            Solo
                          </button>
                          <button
                            type="button"
                            className={`button button-small ${track.isMuted ? "button-danger" : "button-secondary"}`}
                            onClick={() => handleToggleTrackMute(track.trackId)}
                            title={track.isMuted ? "Unmute track" : "Mute track"}
                          >
                            {track.isMuted ? "Muted" : "Mute"}
                          </button>
                        </div>
                      </div>

                      <div className="audio-track-slider-row">
                        <input
                          type="range"
                          min={0}
                          max={1}
                          step={0.05}
                          value={track.isMuted ? 0 : track.volume}
                          onChange={(e) => handleTrackVolumeChange(track.trackId, parseFloat(e.target.value))}
                          disabled={track.isMuted}
                          className="track-volume-slider"
                          aria-label={`${track.name} volume`}
                        />
                        <span className="audio-track-vol-percent">
                          {track.isMuted ? "0%" : `${Math.round(track.volume * 100)}%`}
                        </span>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}

            {/* Trimmer Drawer */}
            {trimActive ? (
              <div className="trimmer-panel">
                <div className="trimmer-header">
                  <strong>✂ Clip Trimmer & Track Export</strong>
                  <span>
                    Trim range: <strong>{formatTime(trimStart)}</strong> to{" "}
                    <strong>{formatTime(trimEnd)}</strong> (Duration:{" "}
                    {((trimEnd - trimStart) || 0).toFixed(1)}s)
                  </span>
                </div>

                <div className="trimmer-controls-grid">
                  <div className="trim-time-group">
                    <label>Start point:</label>
                    <div className="trim-input-row">
                      <input
                        type="number"
                        step={0.1}
                        min={0}
                        max={trimEnd - 0.5}
                        value={+trimStart.toFixed(1)}
                        onChange={(e) => setTrimStart(Math.max(0, parseFloat(e.target.value) || 0))}
                      />
                      <button
                        type="button"
                        className="button button-small button-secondary"
                        onClick={handleSetTrimStart}
                        title="Set trim start to current playback position"
                      >
                        Set In [
                      </button>
                    </div>
                    <label className="keyframe-snap-toggle" title="Snaps cut to nearest I-frame to prevent video stutter">
                      <input
                        type="checkbox"
                        checked={snapToKeyframe}
                        onChange={(e) => setSnapToKeyframe(e.target.checked)}
                      />
                      <span>Snap to nearest keyframe (clean cut)</span>
                    </label>
                  </div>

                  <div className="trim-time-group">
                    <label>End point:</label>
                    <div className="trim-input-row">
                      <input
                        type="number"
                        step={0.1}
                        min={trimStart + 0.5}
                        max={duration}
                        value={+trimEnd.toFixed(1)}
                        onChange={(e) => setTrimEnd(Math.min(duration, parseFloat(e.target.value) || duration))}
                      />
                      <button
                        type="button"
                        className="button button-small button-secondary"
                        onClick={handleSetTrimEnd}
                        title="Set trim end to current playback position"
                      >
                        Set Out ]
                      </button>
                    </div>
                  </div>

                  {/* Audio Track Export Selector */}
                  <div className="trim-audio-group">
                    <label>Included audio tracks:</label>
                    <div className="trim-audio-checkboxes">
                      {audioTracks.length > 0 ? (
                        audioTracks.map((t, idx) => (
                          <label key={t.trackId} className="checkbox-pill">
                            <input
                              type="checkbox"
                              checked={selectedTrimTracks.includes(idx)}
                              onChange={() => handleToggleTrimTrack(idx)}
                            />
                            <span>Audio {idx + 1}: {t.name}</span>
                          </label>
                        ))
                      ) : (
                        <p className="empty-copy">Standard container audio</p>
                      )}
                    </div>
                  </div>
                </div>

                <div className="trimmer-footer">
                  <button
                    type="button"
                    className="button button-accent"
                    onClick={() => void handleExecuteTrim()}
                    disabled={isTrimming}
                  >
                    {isTrimming ? "Saving Trim..." : "Save Trimmed Clip"}
                  </button>
                  {trimMessage ? <span className="trim-feedback-msg">{trimMessage}</span> : null}
                </div>
              </div>
            ) : null}
          </div>

          {/* Sidebar (VOD Review Markers & Timestamped Notes) */}
          <div className={`vod-review-panel ${isTheaterMode ? "is-theater-sidebar" : ""}`}>
            <div className="vod-review-header">
              <div>
                <strong>🎯 VOD Review & Timestamps</strong>
                <span className="vod-meta-badge">{markers.length} markers</span>
              </div>
            </div>

            <div className="vod-add-form">
              <div className="color-palette-picker" role="radiogroup" aria-label="Marker color">
                {MARKER_COLORS.map((c) => (
                  <button
                    key={c.hex}
                    type="button"
                    className={`color-swatch ${selectedColor === c.hex ? "is-selected" : ""}`}
                    style={{ backgroundColor: c.hex }}
                    onClick={() => setSelectedColor(c.hex)}
                    title={c.label}
                    aria-label={c.label}
                  />
                ))}
              </div>

              <input
                type="text"
                className="vod-note-input"
                placeholder="Marker title (e.g. 1v3 Clutch, Missed Smoke)..."
                value={markerLabel}
                onChange={(e) => setMarkerLabel(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleAddMarker();
                }}
              />

              <input
                type="text"
                className="vod-note-input vod-note-detail"
                placeholder="Detailed review notes (optional)..."
                value={markerNote}
                onChange={(e) => setMarkerNote(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleAddMarker();
                }}
              />

              <button
                type="button"
                className="button button-small button-secondary"
                onClick={handleAddMarker}
              >
                + Add Bookmark at {formatTime(currentTime)}
              </button>
            </div>

            {/* Marker List */}
            {markers.length > 0 ? (
              <div className="vod-marker-list">
                {markers.map((m) => (
                  <div key={m.id} className="vod-marker-row">
                    <span
                      className="vod-marker-dot"
                      style={{ backgroundColor: m.color }}
                      aria-hidden="true"
                    />
                    <button
                      type="button"
                      className="vod-marker-timestamp"
                      onClick={() => handleSeek(m.timestampSeconds)}
                      title="Seek to this moment"
                    >
                      {formatTime(m.timestampSeconds)}
                    </button>
                    <div className="vod-marker-info">
                      <strong>{m.label}</strong>
                      {m.note ? <p>{m.note}</p> : null}
                    </div>
                    <button
                      type="button"
                      className="vod-marker-delete"
                      onClick={() => handleDeleteMarker(m.id)}
                      title="Delete marker"
                      aria-label="Delete marker"
                    >
                      ✕
                    </button>
                  </div>
                ))}
              </div>
            ) : (
              <p className="vod-empty-text">
                No review markers yet. Play the video, choose a color, and click "+ Add Bookmark" to save timestamped feedback for your VOD reviews.
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
