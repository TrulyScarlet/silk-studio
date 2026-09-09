import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { ClipEntry } from "./types";
import { isMockMode } from "./ipc";

export function formatBadgeDuration(durationMs: number | null | undefined): string {
  if (durationMs === null || durationMs === undefined || durationMs < 0) {
    return "--:--";
  }
  const totalSeconds = durationMs / 1000;
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  const tenths = Math.floor((seconds % 1) * 10);
  const wholeSecs = Math.floor(seconds);

  const mm = String(minutes).padStart(2, "0");
  const ss = String(wholeSecs).padStart(2, "0");

  if (hours > 0) {
    return `${hours}:${mm}:${ss}.${tenths}`;
  }
  return `${mm}:${ss}.${tenths}`;
}

export type ClipThumbnailProps = {
  clip: ClipEntry;
  onClick?: () => void;
  className?: string;
  showBadges?: boolean;
  gameBadge?: string | null;
};

export function ClipThumbnail({
  clip,
  onClick,
  className = "",
  showBadges = false,
  gameBadge,
}: ClipThumbnailProps): JSX.Element {
  const containerRef = useRef<HTMLDivElement>(null);
  const [isVisible, setIsVisible] = useState<boolean>(false);
  const [videoSrc, setVideoSrc] = useState<string>("");
  const [hasError, setHasError] = useState(false);

  // Lazy viewport loading to keep RAM low even with 1,000+ clips in the library
  useEffect(() => {
    if (!containerRef.current || typeof IntersectionObserver === "undefined") {
      setIsVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setIsVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "150px" },
    );
    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (isVisible && !clip.missing && clip.path) {
      try {
        const url = isMockMode() ? "" : convertFileSrc(clip.path);
        setVideoSrc(url);
      } catch {
        setHasError(true);
      }
    }
  }, [isVisible, clip.path, clip.missing]);

  const durationText =
    clip.durationMs !== null && clip.durationMs !== undefined
      ? formatBadgeDuration(clip.durationMs)
      : null;
  const displayGameBadge = gameBadge !== undefined ? gameBadge : clip.gameName;

  if (clip.missing || hasError || !videoSrc) {
    return (
      <div
        ref={containerRef}
        className={`clip-thumb-wrapper clip-thumb-fallback ${onClick ? "is-clickable" : ""} ${className}`}
        onClick={onClick}
        role={onClick ? "button" : undefined}
        tabIndex={onClick ? 0 : undefined}
        aria-label={clip.missing ? "Missing clip file" : `Preview ${clip.name}`}
      >
        <span className="clip-thumb-icon" aria-hidden="true">
          {clip.missing ? "!" : "▶"}
        </span>

        {showBadges && (
          <>
            {displayGameBadge && (
              <span className="clip-thumb-game-badge" title={displayGameBadge}>
                {displayGameBadge}
              </span>
            )}
            {clip.protected && (
              <span className="clip-thumb-protected-badge" title="Protected clip">
                🛡
              </span>
            )}
            {durationText && (
              <span className="clip-thumb-duration-badge" aria-label={`Duration ${durationText}`}>
                {durationText}
              </span>
            )}
          </>
        )}
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className={`clip-thumb-wrapper ${onClick ? "is-clickable" : ""} ${className}`}
      onClick={onClick}
      role={onClick ? "button" : undefined}
      tabIndex={onClick ? 0 : undefined}
      title={onClick ? `Watch ${clip.name}` : undefined}
    >
      <video
        className="clip-thumb-video"
        src={videoSrc}
        preload="auto"
        muted
        playsInline
        onLoadedMetadata={(e) => {
          try {
            e.currentTarget.currentTime = 0.05;
          } catch {
            // ignore
          }
        }}
        onError={() => {
          setHasError(true);
        }}
      />
      <div className="clip-thumb-play-overlay" aria-hidden="true">
        <div className="play-circle-icon">
          <span className="play-triangle">▶</span>
        </div>
      </div>

      {showBadges && (
        <>
          {displayGameBadge && (
            <span className="clip-thumb-game-badge" title={displayGameBadge}>
              {displayGameBadge}
            </span>
          )}
          {clip.protected && (
            <span className="clip-thumb-protected-badge" title="Protected clip">
              🛡
            </span>
          )}
          {durationText && (
            <span className="clip-thumb-duration-badge" aria-label={`Duration ${durationText}`}>
              {durationText}
            </span>
          )}
        </>
      )}
    </div>
  );
}
