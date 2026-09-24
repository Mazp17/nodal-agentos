import { useEffect, useState } from "react";
import { openInEditor, openWorktree, runDiff, type DiffFileStatus, type FileDiff, type RunDiff } from "../../domain/api";
import { projectIdOf, useRun } from "../../domain/hooks/runs";
import { useToast } from "../../ui/Toasts";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { runTaskRef } from "./status";
import "./diff.css";

/** Mientras el run sigue activo, el diff incluye el working tree: se refresca. */
const LIVE_POLL_MS = 5000;

const STATUS_BADGE: Record<DiffFileStatus, { letter: string; cls: string; label: string }> = {
  added: { letter: "A", cls: "df-added", label: "Added" },
  modified: { letter: "M", cls: "df-modified", label: "Modified" },
  deleted: { letter: "D", cls: "df-deleted", label: "Deleted" },
  renamed: { letter: "R", cls: "df-renamed", label: "Renamed" },
};

type Load = { status: "loading" } | { status: "error"; error: string } | { status: "ok"; diff: RunDiff };

function useRunDiff(runId: string, live: boolean): Load {
  const [state, setState] = useState<{ id: string; load: Load } | null>(null);
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const load = async () => {
      try {
        const diff = await runDiff(runId);
        if (!cancelled) setState({ id: runId, load: { status: "ok", diff } });
      } catch (e) {
        // Un fallo en un refresco no borra lo que ya se mostraba.
        if (!cancelled) {
          setState((prev) => (prev?.id === runId && prev.load.status === "ok" ? prev : { id: runId, load: { status: "error", error: String(e) } }));
        }
      }
      if (!cancelled && live) timer = setTimeout(load, LIVE_POLL_MS);
    };
    void load();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [runId, live]);
  return state?.id === runId ? state.load : { status: "loading" };
}

const splitPath = (p: string) => {
  const i = p.lastIndexOf("/");
  return i < 0 ? { name: p, dir: "" } : { name: p.slice(i + 1), dir: p.slice(0, i) };
};

export interface RunDiffDrawerProps {
  runId: string;
  onClose: () => void;
}

/** Pantalla "Diff": cambios del run contra su base, archivo por archivo. */
export function RunDiffDrawer({ runId, onClose }: RunDiffDrawerProps) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const toast = useToast();
  const { view, state } = useRun(runId);
  const live = view ? view.phase === "running" || view.phase === "waiting" || view.phase === "starting" : false;
  const load = useRunDiff(runId, live);
  const [selected, setSelected] = useState<string | null>(null);

  const run = view?.run;
  const task = run?.taskId ? state.tasks.get(run.taskId) : undefined;
  const pid = run ? projectIdOf(run, state.tasks, state.repos) : null;
  const ref2 = run ? runTaskRef(run, task, pid ? state.projects.get(pid) : undefined) : null;
  const diff = load.status === "ok" ? load.diff : null;
  const files = diff?.files ?? [];
  const file = files.find((f) => f.path === selected) ?? files[0];
  const adds = files.reduce((a, f) => a + f.additions, 0);
  const dels = files.reduce((a, f) => a + f.deletions, 0);
  const branch = run?.branch ?? task?.worktree?.branch ?? null;

  const copy = async () => {
    if (!diff) return;
    try {
      await navigator.clipboard.writeText(diff.patch);
      toast("Patch copied", `git diff ${diff.base}...HEAD${diff.includesWorkingTree ? " + uncommitted changes" : ""}`, "ok");
    } catch (e) {
      toast("Couldn't copy the patch", String(e), "danger");
    }
  };
  const finder = async () => {
    try {
      await openWorktree(runId);
    } catch (e) {
      toast("Couldn't open the folder", String(e), "danger");
    }
  };
  const editor = async () => {
    const target = file && file.status !== "deleted" ? file.path : null;
    try {
      await openInEditor(runId, target);
    } catch (e) {
      toast("Couldn't open the editor", String(e), "danger");
    }
  };

  return (
    <>
      <div className="scrim" onClick={onClose} aria-hidden />
      <div ref={ref} className="sheet diff-panel" role="dialog" aria-modal="true" aria-labelledby="diff-title" tabIndex={-1}>
        <header className="diff-head">
          <div className="diff-head-main">
            <div className="diff-title-row">
              <h2 id="diff-title" className="diff-title">
                Changes
              </h2>
              {ref2?.key && <span className="diff-key">{ref2.key}</span>}
              {ref2 && <span className="diff-task ellipsis">{ref2.title}</span>}
            </div>
            <div className="diff-sub">
              {diff && (
                <>
                  <span className="diff-ref">{diff.base}</span>
                  <span aria-label="from">←</span>
                  <span className="diff-ref">{branch ?? "HEAD"}</span>
                  <span>·</span>
                  <span>
                    {files.length} file{files.length === 1 ? "" : "s"} · +{adds} −{dels}
                  </span>
                  {diff.includesWorkingTree && (
                    <>
                      <span>·</span>
                      <span>Uncommitted changes · live</span>
                      {live && <span className="dot dot-sm pulse tone-accent" aria-hidden />}
                    </>
                  )}
                </>
              )}
            </div>
          </div>
          <button type="button" className="btn btn-sm" disabled={!diff} onClick={() => void copy()}>
            Copy patch
          </button>
          <button type="button" className="btn btn-sm" onClick={() => void finder()}>
            Open worktree
          </button>
          <button type="button" className="btn btn-sm btn-primary" onClick={() => void editor()}>
            Open in editor
          </button>
          <button type="button" className="icon-btn" aria-label="Close" onClick={onClose}>
            ✕
          </button>
        </header>

        {load.status === "loading" ? (
          <p className="diff-note">Loading changes…</p>
        ) : load.status === "error" ? (
          <p className="diff-note diff-error" role="alert">
            Couldn't read the changes: {load.error}
          </p>
        ) : files.length === 0 ? (
          <p className="diff-note">No changes against {load.diff.base}.</p>
        ) : (
          <div className="diff-body">
            <nav className="diff-files" aria-label="Changed files">
              {files.map((f) => {
                const b = STATUS_BADGE[f.status];
                const { name, dir } = splitPath(f.path);
                const on = f === file;
                return (
                  <button
                    key={f.path}
                    type="button"
                    className={`diff-file ${on ? "on" : ""}`}
                    aria-current={on ? "true" : undefined}
                    title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}
                    onClick={() => setSelected(f.path)}
                  >
                    <span className={`df-badge ${b.cls}`} aria-label={b.label}>
                      {b.letter}
                    </span>
                    <span className="diff-file-name">
                      <span className="ellipsis">{name}</span>
                      {dir && <span className="diff-file-dir ellipsis">{dir}</span>}
                    </span>
                    <span className="diff-add">+{f.additions}</span>
                    {f.deletions > 0 && <span className="diff-del">−{f.deletions}</span>}
                  </button>
                );
              })}
            </nav>
            {file && <FileView file={file} />}
          </div>
        )}
      </div>
    </>
  );
}

function FileView({ file }: { file: FileDiff }) {
  return (
    <div className="diff-view">
      <div className="diff-view-head">
        <span className="diff-view-path ellipsis">
          {file.oldPath && file.oldPath !== file.path ? `${file.oldPath} → ${file.path}` : file.path}
        </span>
        <span className="diff-add">+{file.additions}</span>
        {file.deletions > 0 && <span className="diff-del">−{file.deletions}</span>}
      </div>
      <div className="diff-lines" role="table" aria-label={`Diff of ${file.path}`}>
        {file.binary ? (
          <p className="diff-note">Binary file; no text diff.</p>
        ) : file.hunks.length === 0 ? (
          <p className="diff-note">No content changes.</p>
        ) : (
          file.hunks.map((h, hi) => (
            <div key={hi} role="rowgroup">
              <div className="diff-hunk" role="row">
                {h.header}
              </div>
              {h.lines.map((l, li) => (
                <div key={li} className={`diff-line dl-${l.kind}`} role="row">
                  <span className="dl-no">{l.oldNo ?? ""}</span>
                  <span className="dl-no">{l.newNo ?? ""}</span>
                  <span className="dl-sign" aria-hidden>
                    {l.kind === "add" ? "+" : l.kind === "del" ? "−" : ""}
                  </span>
                  <span className="dl-text">{l.text || " "}</span>
                </div>
              ))}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
