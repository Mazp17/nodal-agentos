import { useEffect, useId, useState } from "react";
import { formatDateTime, formatDuration, formatTokens } from "../../lib/format";
import { useFocusTrap } from "../../ui/useFocusTrap";
import type { RunView } from "../runs/status";
import type { TaskActions } from "./actions";
import { readTaskPlan } from "./api";
import { planLabel } from "./TaskCard";
import type { FinishMode, Task } from "./types";
import "./tasks.css";

export interface TaskPanelProps {
  task: Task;
  /** Run vigente (`useTasks().current.get(task.id)`). */
  current?: RunView;
  /** `useTasks().historyOf(task.id)`. */
  history: RunView[];
  /** `useTaskActions(refresh)`. */
  actions: TaskActions;
  onClose: () => void;
  onEdit: (task: Task) => void;
  /** Abrir el detalle del run (RunDetailView); si no viene, no se ofrece. */
  onOpenRun?: (view: RunView) => void;
  /** Cambiarlo fuerza a releer el plan (p. ej. después de editarlo). */
  planVersion?: number;
}

const FINISH: { value: FinishMode; label: string; hint: string }[] = [
  { value: "pr", label: "Open a PR", hint: "Push the branch and open a pull request" },
  { value: "branch", label: "Branch only", hint: "Leave the work on a branch, no PR" },
];

/** Drawer de detalle de una tarea local (mismo layout que el Issue detail). */
export function TaskPanel({ task, current, history, actions, onClose, onEdit, onOpenRun, planVersion = 0 }: TaskPanelProps) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const titleId = useId();
  const [plan, setPlan] = useState<string | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [finish, setFinish] = useState<FinishMode>("pr");
  const busy = actions.pending.has(task.id);
  const done = task.status === "done";
  const planKey = task.plan.kind === "file" ? task.plan.path : "text";

  useEffect(() => {
    let alive = true;
    setPlan(null);
    setPlanError(null);
    readTaskPlan(task.id)
      .then((t) => alive && setPlan(t))
      .catch((e) => alive && setPlanError(String(e)));
    return () => {
      alive = false;
    };
  }, [task.id, planKey, planVersion]);

  const meta: [string, string][] = [
    ["Plan", task.plan.kind === "text" ? "Written plan" : planLabel(task)],
    ["Created", formatDateTime(task.createdAt)],
  ];
  if (task.doneAt) meta.push(["Done", formatDateTime(task.doneAt)]);

  const running = current && (current.kind === "running" || current.kind === "starting");
  const queued = current?.kind === "queued";

  return (
    <>
      <div className="scrim" onClick={onClose} aria-hidden />
      <div
        ref={ref}
        className="sheet tk-panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
      >
        <header className="tk-panel-head">
          <div className="tk-panel-row">
            <span className="tk-kind">Task</span>
            <span className={`tk-state ${done ? "tk-state-done" : ""}`}>
              <span className="tk-state-dot" aria-hidden />
              {done ? "Done" : "To do"}
            </span>
            {current && (
              <span className={`badge tone-${current.tone}`} title={current.title}>
                <span className={`dot dot-sm ${running ? "pulse" : ""}`} aria-hidden />
                {current.label}
              </span>
            )}
            <button type="button" className="icon-btn tk-close" aria-label="Close" onClick={onClose}>
              ✕
            </button>
          </div>
          <h2 id={titleId} className="tk-panel-title">
            {task.title}
          </h2>
          <div className="tk-panel-meta">
            {meta.map(([k, v]) => (
              <span key={k}>
                {k} <span className="tk-meta-v">{v}</span>
              </span>
            ))}
            <span>
              Repo <span className="tk-meta-v mono">{task.repoPath}</span>
            </span>
            {task.plan.kind === "file" && (
              <span>
                File <span className="tk-meta-v mono">{task.plan.path}</span>
              </span>
            )}
          </div>
        </header>

        <div className="tk-panel-body">
          <section className="tk-section" aria-labelledby={`${titleId}-plan`}>
            <h3 id={`${titleId}-plan`} className="section-label">
              Plan
            </h3>
            {/* Markdown mostrado como texto: nunca se interpreta HTML del plan. */}
            {planError ? (
              <div className="banner banner-error" role="alert">
                {planError}
              </div>
            ) : plan === null ? (
              <span className="tk-empty">Loading…</span>
            ) : (
              <pre className="tk-plan">{plan}</pre>
            )}
          </section>

          <section className="tk-section" aria-labelledby={`${titleId}-runs`}>
            <h3 id={`${titleId}-runs`} className="section-label">
              Run history
            </h3>
            {history.map((v) => {
              const body = (
                <>
                  <span className="tk-hist-id">{v.runId ?? "—"}</span>
                  <span className={`badge badge-sm tone-${v.tone}`}>{v.label}</span>
                  <span className="tk-hist-wf ellipsis">{v.workflow}</span>
                  <span className="tk-hist-num num">{v.kind === "queued" ? "—" : formatDuration(v.durationMs)}</span>
                  <span className="tk-hist-num tk-hist-tok num">{formatTokens(v.tokens)}</span>
                </>
              );
              return onOpenRun && v.run ? (
                <button key={v.key} type="button" className="tk-hist" title={v.title} onClick={() => onOpenRun(v)}>
                  {body}
                </button>
              ) : (
                <div key={v.key} className="tk-hist" title={v.title}>
                  {body}
                </div>
              );
            })}
            {history.length === 0 && <span className="tk-empty">No runs yet.</span>}
          </section>

          {!done && !current?.active && (
            <section className="tk-launch" aria-labelledby={`${titleId}-finish`}>
              <h3 id={`${titleId}-finish`} className="section-label">
                When the plan is done
              </h3>
              <div className="tk-finish" role="radiogroup" aria-labelledby={`${titleId}-finish`}>
                {FINISH.map((f) => (
                  <button
                    key={f.value}
                    type="button"
                    role="radio"
                    aria-checked={finish === f.value}
                    className={`tk-finish-opt ${finish === f.value ? "on" : ""}`}
                    onClick={() => setFinish(f.value)}
                  >
                    <span className="tk-finish-name">{f.label}</span>
                    <span className="tk-finish-desc">{f.hint}</span>
                  </button>
                ))}
              </div>
            </section>
          )}
        </div>

        <footer className="tk-panel-foot">
          <button type="button" className="btn btn-ghost tk-foot-left" disabled={busy} onClick={() => void actions.remove(task).then((ok) => ok && onClose())}>
            Delete
          </button>
          <button type="button" className="btn" disabled={busy} onClick={() => onEdit(task)}>
            Edit
          </button>
          <button type="button" className="btn" disabled={busy} onClick={() => void actions.toggleDone(task)}>
            {done ? "Reopen" : "Mark done"}
          </button>
          {current && running && current.runId && (
            <>
              <button type="button" className="btn btn-danger" disabled={busy} onClick={() => void actions.stop(task, current)}>
                Stop
              </button>
              <button type="button" className="btn" disabled={busy} onClick={() => void actions.attach(task, current)}>
                Attach
              </button>
            </>
          )}
          {current && queued && current.label !== "Launching" && (
            <button type="button" className="btn" disabled={busy} onClick={() => void actions.cancel(task)}>
              Remove from queue
            </button>
          )}
          {current && onOpenRun && current.run && (
            <button type="button" className="btn" onClick={() => onOpenRun(current)}>
              Open run
            </button>
          )}
          {!done && !current?.active && (
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void actions.run(task, finish)}>
              {current ? "Run again" : "Run"}
            </button>
          )}
        </footer>
      </div>
    </>
  );
}
