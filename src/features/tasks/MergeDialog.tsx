import { useEffect, useId, useState } from "react";
import { mergeWorktree, worktreeStatus, type MergeReport, type WorktreeStatus } from "../../domain/api";
import type { WorktreeRef } from "../../domain/types";
import { useFocusTrap } from "../../ui/useFocusTrap";
import "../../ui/confirm.css";

export interface MergeDialogProps {
  taskId: string;
  worktree: WorktreeRef;
  /** `PLA-2: tarea 2`: the commit message. */
  message: string;
  /** The worktree has uncommitted changes by design (finish mode "changes"). */
  commitFirst: boolean;
  /** Linked task manager, to say it gets the Done status too. */
  provider: string | null;
  onClose: () => void;
  onMerged: (report: MergeReport) => void;
  /** Hands the conflict off to Claude with these instructions. */
  onHandOff: (instructions: string) => void;
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** Confirmation for "Merge into <base> & done": nothing runs until it's confirmed. */
export function MergeDialog({ taskId, worktree, message, commitFirst, provider, onClose, onMerged, onHandOff }: MergeDialogProps) {
  const [busy, setBusy] = useState(false);
  const close = () => {
    if (!busy) onClose();
  };
  const ref = useFocusTrap<HTMLDivElement>(close);
  const titleId = useId();
  const [status, setStatus] = useState<WorktreeStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [squash, setSquash] = useState(true);
  const [cleanup, setCleanup] = useState(true);
  const [push, setPush] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState<string[] | null>(null);
  const { base, branch } = worktree;

  useEffect(() => {
    let live = true;
    worktreeStatus(taskId)
      .then((s) => live && setStatus(s))
      .catch((e) => live && setStatusError(String(e)));
    return () => {
      live = false;
    };
  }, [taskId]);

  const dirty = status?.dirty ?? commitFirst;
  const commits = status?.ahead ?? 0;
  const nothing = !!status && commits === 0 && !status.dirty;
  const summary = [commits > 0 ? plural(commits, "commit") : null, dirty ? "uncommitted changes" : null]
    .filter(Boolean)
    .join(" + ");

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const report = await mergeWorktree(taskId, { squash, push, cleanup });
      if (report.outcome.kind === "conflict") setConflict(report.outcome.files);
      else onMerged(report);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handOff = () =>
    onHandOff(
      `Merging ${base} into ${branch} conflicts in: ${conflict?.join(", ")}. Merge ${base} into this branch, resolve the conflicts keeping both sides' intent, and commit the merge. Don't push.`,
    );

  return (
    <>
      <div className="confirm-scrim" onClick={close} aria-hidden />
      <div ref={ref} className="confirm" role="alertdialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
        <h2 id={titleId} className="confirm-title">
          {conflict ? `${base} conflicts with ${branch}` : `${dirty ? "Commit & merge" : "Merge"} into ${base}?`}
        </h2>
        {conflict ? (
          <div className="confirm-body">
            <span>The merge was aborted: nothing changed and the task is still open. Conflicting files:</span>
            {conflict.map((f) => (
              <span key={f} className="confirm-path">
                {f}
              </span>
            ))}
            <span>Claude can merge {base} into the worktree and resolve them; then merge again.</span>
          </div>
        ) : (
          <div className="confirm-body">
            <span className="mono tp-merge-summary">
              {status ? (nothing ? "Nothing to merge" : `${summary} → ${base}`) : statusError ? "State unknown" : "Checking…"}
            </span>
            <span>
              {squash ? "One commit" : "Commits land as they are"}
              {squash || dirty ? (
                <>
                  , message <span className="mono">{message}</span>
                </>
              ) : null}
              . {base} only moves forward; the repo folder isn't switched to it. The task moves to Done
              {provider ? ` and ${provider} is updated` : ""}.
            </span>
            <label className="tp-merge-opt">
              <input type="checkbox" checked={squash} disabled={busy} onChange={(e) => setSquash(e.target.checked)} />
              Squash into one commit
            </label>
            <label className="tp-merge-opt">
              <input type="checkbox" checked={cleanup} disabled={busy} onChange={(e) => setCleanup(e.target.checked)} />
              Clean up the worktree and branch afterwards
            </label>
            <label className="tp-merge-opt">
              <input type="checkbox" checked={push} disabled={busy} onChange={(e) => setPush(e.target.checked)} />
              Push {base} to the remote
            </label>
            {(error ?? statusError) && (
              <div className="banner banner-error" role="alert">
                {error ?? statusError}
              </div>
            )}
          </div>
        )}
        <div className="confirm-foot">
          <button type="button" className="btn btn-ghost" data-autofocus disabled={busy} onClick={close}>
            {conflict ? "Close" : "Cancel"}
          </button>
          {conflict ? (
            <button type="button" className="btn btn-primary" onClick={handOff}>
              Hand off to Claude
            </button>
          ) : (
            <button type="button" className="btn btn-primary" disabled={busy || nothing} onClick={() => void run()}>
              {busy ? "Merging…" : dirty ? "Commit & merge" : "Merge & done"}
            </button>
          )}
        </div>
      </div>
    </>
  );
}
