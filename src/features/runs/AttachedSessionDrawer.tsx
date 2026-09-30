import { useRuns, projectIdOf } from "../../domain/hooks/runs";
import { useToast } from "../../ui/Toasts";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/Tooltip";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { attachRun } from "./api";
import type { AttachTarget } from "./attachStore";
import { executorLabel, runTaskRef } from "./status";
import { useAttachedTerminal } from "./useAttachedTerminal";
import "@xterm/xterm/css/xterm.css";
import "./attached-session.css";

export interface AttachedSessionDrawerProps {
  target: AttachTarget;
  onClose: () => void;
}

/**
 * "Attached session": `claude attach` in an embedded terminal. Closing only detaches;
 * ↗ opens the same session in Terminal.app.
 */
export function AttachedSessionDrawer({ target, onClose }: AttachedSessionDrawerProps) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const toast = useToast();
  const state = useRuns();
  const term = useAttachedTerminal(target.claudeId, target.cwd, onClose);

  const view = state.byId.get(target.runId);
  const run = view?.run;
  const task = run?.taskId ? state.tasks.get(run.taskId) : undefined;
  const pid = run ? projectIdOf(run, state.tasks, state.repos) : null;
  const taskRef = run ? runTaskRef(run, task, pid ? state.projects.get(pid) : undefined) : null;
  const model = run?.options.model ?? null;
  const branch = run?.branch ?? task?.worktree?.branch ?? null;

  const openExternal = async () => {
    try {
      await attachRun(target.claudeId);
    } catch (e) {
      toast("Couldn't open Terminal", String(e), "danger");
    }
  };

  const exitNote = term.exitCode !== null && term.exitCode !== 0 ? ` (exit code ${term.exitCode})` : "";
  const banner =
    term.status === "ended"
      ? { text: `Detached from the session.${exitNote}`, error: exitNote !== "" }
      : term.status === "error"
        ? { text: term.error ?? "Couldn't attach to the session.", error: true }
        : null;

  return (
    <>
      <div className="scrim as-scrim" onClick={onClose} aria-hidden />
      <div ref={ref} className="sheet as-panel" role="dialog" aria-modal="true" aria-labelledby="as-title" tabIndex={-1}>
        <header className="as-head">
          <div className="as-head-main">
            <div className="as-title-row">
              {view && (
                <span className={`as-status tone-${view.tone}`}>
                  <span className={`dot dot-sm ${view.pulse ? "pulse" : ""}`} aria-hidden />
                  {view.label}
                </span>
              )}
              {taskRef?.key && <span className="as-key">{taskRef.key}</span>}
              <h2 id="as-title" className="as-title ellipsis">
                {taskRef?.title ?? `Session ${target.claudeId}`}
              </h2>
            </div>
            {run && (
              <div className="as-sub">
                <span className="as-mono as-exec">{executorLabel(run.executor)}</span>
                {model && (
                  <>
                    <span aria-hidden>·</span>
                    <span>{model}</span>
                  </>
                )}
                {branch && (
                  <>
                    <span aria-hidden>·</span>
                    <span className="as-mono ellipsis">{branch}</span>
                  </>
                )}
              </div>
            )}
          </div>
          <Tooltip>
            <TooltipTrigger asChild>
              <button type="button" className="icon-btn" aria-label="Open in Terminal.app" onClick={() => void openExternal()}>
                ↗
              </button>
            </TooltipTrigger>
            <TooltipContent>Open in Terminal.app · claude attach {target.claudeId}</TooltipContent>
          </Tooltip>
          <button type="button" className="icon-btn" aria-label="Close" onClick={onClose}>
            ✕
          </button>
        </header>

        {/* Initial focus lands here (not on ↗, whose tooltip would flash) until the terminal takes it. */}
        <div className="as-term" tabIndex={-1} data-autofocus>
          <div ref={term.hostRef} className="as-xterm" />
          {term.status === "connecting" && (
            <p className="as-connecting" role="status">
              Attaching…
            </p>
          )}
          {/* Below the terminal, not over it: what `claude attach` printed stays readable. */}
          {banner && (
            <div className={`as-banner ${banner.error ? "as-banner-error" : ""}`}>
              <p className="as-banner-text">{banner.text}</p>
              <button type="button" className="btn btn-sm btn-primary" autoFocus onClick={term.reattach}>
                Reattach
              </button>
            </div>
          )}
        </div>

        {/* Always mounted, only its text changes: a live region added with its text is often not announced. */}
        <p className="sr-only" role="status" aria-live="polite">
          {banner?.text ?? ""}
        </p>

        <footer className="as-foot">
          <span className="ellipsis">Esc goes to Claude · ⇧Esc to close</span>
          <span className="as-foot-size">
            pty {term.cols ?? "–"}×{term.rows ?? "–"} · {target.claudeId}
          </span>
        </footer>
      </div>
    </>
  );
}
