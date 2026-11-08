import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { isMockMode } from "./ipc";

type UpdateStatus = {
  currentVersion: string;
  version: string | null;
  notes: string | null;
  phase: "idle" | "current" | "checking" | "available" | "downloading" | "verifying" | "installing" | "error";
  downloaded: number;
  total: number | null;
  error: string | null;
};

const INITIAL: UpdateStatus = { currentVersion: "", version: null, notes: null, phase: "idle", downloaded: 0, total: null, error: null };
const BUSY = new Set(["checking", "downloading", "verifying", "installing"]);

function useUpdates() {
  const [status, setStatus] = useState(INITIAL);
  useEffect(() => {
    if (isMockMode()) return;
    let mounted = true;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      try {
        const next = await invoke<UpdateStatus>("get_update_status");
        if (mounted) setStatus(next);
      } catch { /* Keep the last state when the app is shutting down. */ }
      if (mounted) timer = setTimeout(() => void refresh(), 750);
    };
    void refresh();
    return () => { mounted = false; clearTimeout(timer); };
  }, []);

  const run = async (command: "check_for_updates" | "install_update") => {
    if (isMockMode() || BUSY.has(status.phase)) return;
    setStatus((current) => ({ ...current, phase: command === "check_for_updates" ? "checking" : "downloading", error: null, downloaded: 0, total: null }));
    try { await invoke(command); }
    catch (error) { setStatus((current) => ({ ...current, phase: "error", error: String(error) })); }
  };
  return { status, run, busy: BUSY.has(status.phase) };
}

function UpdateDetails({ status }: { status: UpdateStatus }) {
  const labels: Partial<Record<UpdateStatus["phase"], string>> = {
    idle: "Check for the latest version of Silk.", current: "You're up to date.", checking: "Checking for updates…",
    available: `Silk ${status.version} is available.`, downloading: "Downloading update…",
    verifying: "Verifying update…", installing: "Installing update. Silk will restart…",
  };
  const percent = status.total && status.total > 0 ? Math.min(100, Math.floor(status.downloaded / status.total * 100)) : null;
  return <>
    <p role="status">{labels[status.phase] ?? "The update couldn't be completed. Please try again."}</p>
    {status.phase === "downloading" && <div className="app-update-progress">
      <progress aria-label="Update download" max={100} value={percent ?? undefined} />
      <span>{percent === null ? `${(status.downloaded / 1_000_000).toFixed(1)} MB` : `${percent}%`}</span>
    </div>}
    {status.notes && <p className="app-update-notes">{status.notes}</p>}
    {status.error && <p className="app-update-error" role="alert">{status.error}</p>}
  </>;
}

export function TitleBarUpdate() {
  const { status, run, busy } = useUpdates();
  const [open, setOpen] = useState(false);
  const container = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!container.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  if (!status.version || status.phase === "current") return null;
  return <div ref={container} className="titlebar-update-wrap" onDoubleClick={(event) => event.stopPropagation()}
    onKeyDown={(event) => { event.stopPropagation(); if (event.key === "Escape") { setOpen(false); trigger.current?.focus(); } }}>
    <button ref={trigger} type="button" className="titlebar-update-pill" aria-expanded={open} aria-haspopup="dialog"
      onClick={() => setOpen((value) => !value)}>
      <span className="titlebar-update-dot" aria-hidden="true" />
      <span>{busy ? "Updating…" : "New update"}</span>
    </button>
    {open && <section className="titlebar-update-panel" role="dialog" aria-label="Silk update">
      <div className="titlebar-update-panel-heading"><div><strong>Update Silk</strong><span>Version {status.version}</span></div></div>
      <UpdateDetails status={status} />
      <button type="button" className="titlebar-update-action" disabled={busy} onClick={() => void run("install_update")}>
        {busy ? "Updating…" : status.phase === "error" ? "Retry update & restart" : "Update & restart"}
      </button>
      <span className="titlebar-update-footnote">Your settings and saved clips stay. Save any replay you want to keep before updating.</span>
    </section>}
  </div>;
}

export function UpdateSettings() {
  const { status, run, busy } = useUpdates();
  return <div className="app-update-settings">
    <div className="app-update-heading"><strong>App updates</strong>{status.currentVersion && <span>Silk {status.currentVersion}</span>}</div>
    <UpdateDetails status={status} />
    <div className="app-update-actions">
      <button type="button" className="button button-secondary" disabled={busy || isMockMode()} onClick={() => void run("check_for_updates")}>Check for updates</button>
      {status.version && <button type="button" className="button button-primary" disabled={busy} onClick={() => void run("install_update")}>Update & restart</button>}
    </div>
    {status.version && <p className="field-hint">Settings and saved clips stay. Save any replay you want to keep before updating.</p>}
    {isMockMode() && <p className="field-hint">Update checks are available in the Windows app.</p>}
  </div>;
}
