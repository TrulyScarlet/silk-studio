import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { isMockMode } from "./ipc";
import { SilkLogo } from "./SilkLogo";

export function TitleBar() {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    if (isMockMode()) return;

    let mounted = true;
    const checkMax = async () => {
      try {
        const max = await invoke<boolean>("app_window_is_maximized");
        if (mounted) setIsMaximized(max);
      } catch {
        // Ignore
      }
    };
    void checkMax();

    const interval = setInterval(() => {
      void checkMax();
    }, 1000);

    return () => {
      mounted = false;
      clearInterval(interval);
    };
  }, []);

  const handleMinimize = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (isMockMode()) return;
    await invoke("app_window_minimize").catch(() => {});
  };

  const handleToggleMaximize = async (e?: React.MouseEvent) => {
    if (e) e.stopPropagation();
    if (isMockMode()) {
      setIsMaximized((prev) => !prev);
      return;
    }
    try {
      const max = await invoke<boolean>("app_window_toggle_maximize");
      setIsMaximized(max);
    } catch {
      // Ignore
    }
  };

  const handleClose = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (isMockMode()) return;
    await invoke("app_window_close").catch(() => {});
  };

  return (
    <header
      className="silk-titlebar"
      data-tauri-drag-region
      onDoubleClick={() => void handleToggleMaximize()}
    >
      {/* Left Branding */}
      <div className="titlebar-left" data-tauri-drag-region>
        <div className="titlebar-brand" data-tauri-drag-region>
          <span className="titlebar-logo-glyph" aria-hidden="true">
            <SilkLogo size={15} />
          </span>
          <span className="titlebar-app-title" data-tauri-drag-region>
            SILK
          </span>
        </div>
      </div>

      {/* Center Draggable Space */}
      <div className="titlebar-center" data-tauri-drag-region>
        <span className="titlebar-empty-drag" data-tauri-drag-region />
      </div>

      {/* High-Precision Native-Feel Window Controls */}
      <div className="titlebar-controls" aria-label="Window controls">
        <button
          type="button"
          className="titlebar-btn titlebar-btn-minimize"
          onClick={handleMinimize}
          title="Minimize"
          aria-label="Minimize"
          tabIndex={-1}
        >
          <svg width="11" height="1" viewBox="0 0 11 1" fill="none">
            <rect width="11" height="1" fill="currentColor" />
          </svg>
        </button>

        <button
          type="button"
          className="titlebar-btn titlebar-btn-maximize"
          onClick={handleToggleMaximize}
          title={isMaximized ? "Restore" : "Maximize"}
          aria-label={isMaximized ? "Restore" : "Maximize"}
          tabIndex={-1}
        >
          {isMaximized ? (
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
              <path
                d="M2.5 1.5H8.5V7.5M1.5 3.5H7.5V9.5H1.5V3.5Z"
                stroke="currentColor"
                strokeWidth="1.1"
              />
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
              <rect
                x="0.55"
                y="0.55"
                width="8.9"
                height="8.9"
                stroke="currentColor"
                strokeWidth="1.1"
              />
            </svg>
          )}
        </button>

        <button
          type="button"
          className="titlebar-btn titlebar-btn-close"
          onClick={handleClose}
          title="Close (Minimizes to tray)"
          aria-label="Close"
          tabIndex={-1}
        >
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
            <path
              d="M0.5 0.5L9.5 9.5M9.5 0.5L0.5 9.5"
              stroke="currentColor"
              strokeWidth="1.1"
            />
          </svg>
        </button>
      </div>
    </header>
  );
}
