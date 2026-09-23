import type { RunView } from "../runs/status";
import type { Task } from "./types";
import "./tasks.css";

export interface TaskCardProps {
  task: Task;
  /** Run vigente de la tarea (de `useTasks().current`). */
  view?: RunView;
  /** Mostrar el nombre del repo (útil cuando la lista mezcla repos). */
  showRepo?: boolean;
  selected?: boolean;
  /** Hay una acción en vuelo para esta tarea. */
  busy?: boolean;
  onOpen: (task: Task) => void;
  onRun: (task: Task) => void;
  onToggleDone: (task: Task) => void;
  /** Click en el badge del run; si no viene, abre la tarea. */
  onOpenRun?: (view: RunView) => void;
}

const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

export function planLabel(task: Task): string {
  return task.plan.kind === "text" ? "Written plan" : basename(task.plan.path);
}

/** Card de tarea local con el mismo lenguaje visual que las cards del board. */
export function TaskCard({ task, view, showRepo, selected, busy, onOpen, onRun, onToggleDone, onOpenRun }: TaskCardProps) {
  const done = task.status === "done";
  const active = view?.active ?? false;
  const segs = view && view.phaseTotal > 0 ? view.phaseTotal : 0;
  return (
    <article className={`tk-card ${selected ? "tk-card-selected" : ""} ${done ? "tk-card-done" : ""}`}>
      <div className="tk-card-top">
        <button
          type="button"
          className="tk-check"
          role="checkbox"
          aria-checked={done}
          aria-label={done ? `Mark "${task.title}" as to do` : `Mark "${task.title}" as done`}
          title={done ? "Done" : "Mark as done"}
          disabled={busy}
          onClick={() => onToggleDone(task)}
        >
          {done ? "✓" : ""}
        </button>
        <span className="tk-kind">Task</span>
        {showRepo && <span className="tk-repo ellipsis mono">{basename(task.repoPath)}</span>}
      </div>
      <button type="button" className="tk-card-title" onClick={() => onOpen(task)}>
        {task.title}
      </button>
      <div className="tk-card-meta">
        <span className="tk-plan-icon" aria-hidden>
          {task.plan.kind === "text" ? "¶" : "▤"}
        </span>
        <span className="ellipsis">{planLabel(task)}</span>
      </div>
      {view && view.kind === "running" && segs > 0 && view.phaseIndex != null && (
        <div
          className={`tk-segs tone-${view.tone}`}
          style={{ gridTemplateColumns: `repeat(${segs}, 1fr)` }}
          role="img"
          aria-label={view.label}
        >
          {Array.from({ length: segs }, (_, k) => {
            const cur = (view.phaseIndex ?? 1) - 1;
            return <span key={k} className={k < cur ? "on" : k === cur ? "on current" : ""} />;
          })}
        </div>
      )}
      <div className="tk-card-actions">
        {view ? (
          <button
            type="button"
            className={`badge tone-${view.tone}`}
            title={view.title}
            onClick={() => (onOpenRun ? onOpenRun(view) : onOpen(task))}
          >
            <span className={`dot dot-sm ${view.kind === "running" || view.kind === "starting" ? "pulse" : ""}`} aria-hidden />
            {view.label}
          </button>
        ) : (
          <span className="tk-norun">Not run yet</span>
        )}
        <div className="tk-card-actions-end">
          {!done && (
            <button
              type="button"
              className="btn btn-xs tk-btn-run"
              disabled={busy || active}
              title={active ? "A run is already in progress" : "Run /plan-task"}
              onClick={() => onRun(task)}
            >
              ▶ Run
            </button>
          )}
        </div>
      </div>
    </article>
  );
}
