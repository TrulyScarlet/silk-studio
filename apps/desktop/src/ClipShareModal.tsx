import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type PointerEvent, type MouseEvent, type ReactNode, type DragEvent } from "react";
import { createPortal } from "react-dom";
import type { ClipEntry } from "./types";
import { isMockMode } from "./ipc";
import { Icon } from "./Icon";
import { ClipThumbnail } from "./ClipThumbnail";

type ShareState = {
  jobId: number | null;
  status: "preparing" | "ready" | "failed" | "cancelled";
  stage: string;
  sizeBytes: number | null;
  error: string | null;
};

function useFileDrag(path: string, jobId: number | null, enabled: boolean, onError: (message: string) => void) {
  const start = useRef<{ x: number; y: number } | null>(null);
  const dragged = useRef(false);
  return {
    draggable: false,
    onDragStart: (e: DragEvent<HTMLElement>) => e.preventDefault(),
    onPointerDown: (e: PointerEvent<HTMLElement>) => {
      dragged.current = false;
      if (enabled && e.button === 0) start.current = { x: e.clientX, y: e.clientY };
    },
    onPointerMove: (e: PointerEvent<HTMLElement>) => {
      if (!start.current || !(e.buttons & 1)) return;
      if (Math.hypot(e.clientX - start.current.x, e.clientY - start.current.y) < 6) return;
      start.current = null;
      dragged.current = true;
      e.preventDefault();
      if (isMockMode()) { onError("File drag is available in the Windows app."); return; }
      void invoke("drag_clip_share", { path, jobId }).catch((error) => onError(String(error)));
    },
    onPointerUp: () => { start.current = null; },
    onPointerCancel: () => { start.current = null; },
    onPointerLeave: () => { start.current = null; },
    onClickCapture: (e: MouseEvent<HTMLElement>) => {
      if (dragged.current) { e.preventDefault(); e.stopPropagation(); dragged.current = false; }
    },
  };
}

export function ClipDragTarget({ clip, children, className, onClick, onError }: {
  clip: ClipEntry; children: ReactNode; className: string; onClick: () => void; onError: (message: string) => void;
}) {
  const drag = useFileDrag(clip.path, null, !clip.missing, onError);
  return <div className={className} {...drag} onClick={onClick} title="Watch clip, or drag the original file to another app">{children}</div>;
}

const STAGES: Record<string, string> = { queued: "Waiting to prepare", preparing: "Preparing clip", encoding: "Encoding video and mixing audio", probing: "Checking file size and streams", decoding: "Checking playback", publishing: "Finishing your file" };

export function ClipShareModal({ clip, onClose }: { clip: ClipEntry; onClose: () => void }) {
  const [target, setTarget] = useState<number | null>(null);
  const [share, setShare] = useState<ShareState | null>(null);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState("");
  const mounted = useRef(true);
  const activeJob = useRef<number | null>(null);
  const dialog = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const busy = starting || share?.status === "preparing";
  const ready = share?.status === "ready";
  const drag = useFileDrag(clip.path, share?.jobId ?? null, ready, setError);

  useEffect(() => {
    mounted.current = true;
    const previous = document.activeElement as HTMLElement | null;
    dialog.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.code === "Escape") { event.preventDefault(); event.stopImmediatePropagation(); closeRef.current(); }
      if (event.key === "Tab") {
        const elements = Array.from(dialog.current?.querySelectorAll<HTMLElement>("button:not(:disabled), [tabindex='0']") ?? []);
        const first = elements[0], last = elements[elements.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    };
    window.addEventListener("keydown", keydown, true);
    return () => {
      mounted.current = false;
      window.removeEventListener("keydown", keydown, true);
      if (activeJob.current !== null) void invoke("cancel_clip_share", { jobId: activeJob.current }).catch(() => {});
      previous?.focus();
    };
  }, []);

  useEffect(() => {
    if (target !== null) return;
    let live = true;
    if (isMockMode()) {
      setShare({ jobId: null, status: "ready", stage: "ready", sizeBytes: clip.sizeBytes, error: null });
    } else {
      void invoke<ShareState>("prepare_clip_share", { path: clip.path, targetMb: null }).then((state) => { if (live) setShare(state); }).catch((reason) => { if (live) setError(String(reason)); });
    }
    return () => { live = false; };
  }, [clip.path, clip.sizeBytes, target]);

  useEffect(() => {
    if (share?.status !== "preparing" || share.jobId === null) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const state = await invoke<ShareState>("clip_share_status", { jobId: share.jobId });
        if (!live) return;
        setShare(state);
        if (state.status === "preparing") timer = setTimeout(poll, 500);
        else { activeJob.current = null; if (state.error) setError(state.error); }
      } catch (reason) {
        if (live) {
          setError(String(reason));
          setShare(null);
          void invoke("cancel_clip_share", { jobId: share.jobId }).catch(() => {});
        }
      }
    };
    timer = setTimeout(poll, 300);
    return () => { live = false; clearTimeout(timer); };
  }, [share?.jobId, share?.status]);

  const prepare = async () => {
    setError("");
    setStarting(true);
    try {
      if (isMockMode()) throw new Error("Prepare a share copy in the Windows app.");
      const state = await invoke<ShareState>("prepare_clip_share", { path: clip.path, targetMb: target });
      if (!mounted.current) {
        if (state.jobId !== null) void invoke("cancel_clip_share", { jobId: state.jobId }).catch(() => {});
        return;
      }
      activeJob.current = state.status === "preparing" ? state.jobId : null;
      setShare(state);
    } catch (reason) { if (mounted.current) setError(String(reason)); }
    finally { if (mounted.current) setStarting(false); }
  };

  return createPortal(
    <div className="clip-share-backdrop" onClick={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div ref={dialog} className="clip-share-dialog" role="dialog" aria-modal="true" aria-labelledby="share-heading">
        <header><div><h2 id="share-heading">Share clip</h2><p>{clip.name}</p></div>
          <button type="button" className="icon-button" aria-label="Close share dialog" onClick={onClose}><Icon name="close" /></button>
        </header>
        <p className="share-size-label">Choose a file size</p>
        <div className="share-presets" role="group" aria-label="Share file size">
          {[null, 10, 50, 100].map((mb) => <button type="button" key={mb ?? "original"} aria-pressed={target === mb} disabled={busy}
            onClick={() => { if (mb !== target) { setTarget(mb); setShare(null); setError(""); } }}>
            {mb === null ? "Original" : `${mb} MB`}
          </button>)}
        </div>
        <p className="share-explanation">{target === null ? "The original recording, with its separate audio tracks." : "An MP4 copy with all recorded audio mixed into stereo. Your original stays unchanged."}</p>
        <div className={`share-drop-zone ${ready ? "is-ready" : ""}`} {...drag} aria-label={ready ? "Drag clip to another app" : "Share preparation"}>
          {ready ? (
            <div className="share-clip-preview" role="img" aria-label={`Thumbnail of ${clip.name}`}>
              <ClipThumbnail clip={clip} />
            </div>
          ) : <Icon name="share" />}
          {ready ? <><strong>Drag & drop</strong><span>Drag this file into Discord or another app.</span><small>{((share.sizeBytes ?? 0) / 1_000_000).toFixed(2)} MB · Ready to share</small></>
            : busy ? <><strong>Preparing your clip</strong><span role="status">{starting ? "Checking sharing tools" : STAGES[share?.stage ?? "preparing"] ?? "Preparing clip"}</span><progress aria-label="Preparing share copy" /></>
            : <><strong>{share?.status === "cancelled" ? "Preparation cancelled" : "Make a share copy"}</strong><span>{target} MB maximum · H.264 video</span></>}
        </div>
        {error && <p className="share-error" role="alert">{error}</p>}
        <footer>
          {ready ? <button type="button" className="button button-secondary" onClick={() => {
            if (isMockMode()) { setError("File sharing is available in the Windows app."); return; }
            void invoke("reveal_clip_share", { path: clip.path, jobId: share.jobId }).catch((reason) => setError(String(reason)));
          }}><Icon name="folder" /> Show file in folder</button>
            : busy ? <button type="button" className="button button-secondary" onClick={onClose}>Cancel</button>
            : <button type="button" className="button button-primary" onClick={() => void prepare()}>Prepare clip</button>}
          <small>Choose a size within your chat’s upload limit.</small>
        </footer>
      </div>
    </div>, document.body,
  );
}
