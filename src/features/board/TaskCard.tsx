import type { DragEvent, MouseEvent } from "react";
import { isRunActive } from "../../domain/hooks/runs";
import { taskKey, type Executor, type Project, type Repo, type Run, type Task } from "../../domain/types";
import { ExecutorAvatar, executorLabel } from "../executors";
import { RunBadge } from "../runs";
import { PriorityBars } from "../tasks/bits";
import { isClosed } from "../tasks/status";
import type { PhaseProgress } from "./usePhases";

export type CardAction = "run" | "retry" | "open-run" | "choose-repo";

export interface CardModel {
  task: Task;
  project: Project | undefined;
  repo: Repo | undefined;
  assignee: Executor;
  run: Run | undefined;
  phase: PhaseProgress | undefined;
}

/** Acción principal de la card según repo, run vigente y estado. */
export function cardAction(m: CardModel): CardAction | null {
  const { task, repo, run } = m;
  if (!repo) return "choose-repo";
  if (run && isRunActive(run)) return "open-run";
  if (isClosed(task.status) || task.status === "in_review") return run ? "open-run" : null;
  const failed =
    task.status === "blocked" ||
    run?.status === "failed" ||
    (run?.status === "finished" && (run.outcome === "red" || run.outcome === "stopped" || run.verdict?.pass === false));
  return failed ? "retry" : "run";
}

const ACTION_LABEL: Record<CardAction, string> = {
  run: "Run",
  retry: "Retry",
  "open-run": "Open run",
  "choose-repo": "Choose repo",
};

interface Props {
  model: CardModel;
  showProject: boolean;
  busy: boolean;
  dragging: boolean;
  onOpen: () => void;
  onAction: (a: CardAction) => void;
  onDragStart: (e: DragEvent<HTMLElement>) => void;
  onDragEnd: () => void;
  /** Alt+flechas sobre la card: reordenar o cambiar de columna sin mouse. */
  onKeyMove: (key: "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight") => void;
}

export function TaskCard({ model, showProject, busy, dragging, onOpen, onAction, onDragStart, onDragEnd, onKeyMove }: Props) {
  const { task, project, repo, assignee, run, phase } = model;
  const action = cardAction(model);
  const primary = action === "run" || action === "retry";
  const reviewing = task.status === "in_progress" && run?.kind === "review" && isRunActive(run);
  const id = project ? taskKey(project.key, task.number) : `#${task.number}`;
  const src = task.source;

  const act = (e: MouseEvent) => {
    e.stopPropagation();
    if (action) onAction(action);
  };

  return (
    <article
      className={`bd-card ${dragging ? "dragging" : ""}`}
      draggable
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onClick={onOpen}
      aria-label={`${id} ${task.title}`}
    >
      <div className="bd-card-meta">
        {showProject && project && (
          <>
            <span className="bd-proj-dot" style={{ background: project.color }} aria-hidden />
            <span className="nowrap">{project.name}</span>
            <span className="bd-sep" aria-hidden>·</span>
          </>
        )}
        <span className={`nowrap ${repo ? "" : "bd-danger"}`}>{repo ? repo.name : "Repo removed"}</span>
        <span className="bd-sep" aria-hidden>·</span>
        {src ? (
          <span className="bd-ext mono" title={`${src.provider} · ${src.externalState?.name ?? "unknown state"}`}>
            {src.identifier}
          </span>
        ) : (
          <span className="bd-local">Local</span>
        )}
        {src?.syncError && <span className="dot dot-sm tone-danger" title={`Sync error: ${src.syncError}`} />}
        <span className="bd-card-right">
          <span className="mono bd-key">{id}</span>
          <PriorityBars priority={task.priority} />
        </span>
      </div>

      {/* Botón real para abrir con teclado; la card entera también abre con click. */}
      <button
        type="button"
        className="bd-card-title"
        aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown Alt+ArrowLeft Alt+ArrowRight"
        title="Alt+arrows to move"
        onClick={(e) => {
          e.stopPropagation();
          onOpen();
        }}
        onKeyDown={(e) => {
          if (!e.altKey) return;
          if (e.key === "ArrowUp" || e.key === "ArrowDown" || e.key === "ArrowLeft" || e.key === "ArrowRight") {
            e.preventDefault();
            onKeyMove(e.key);
          }
        }}
      >
        {task.title}
      </button>

      {task.labels.length > 0 && (
        <div className="bd-labels">
          {task.labels.slice(0, 3).map((l) => (
            <span key={l} className="bd-label">
              {l}
            </span>
          ))}
          {task.labels.length > 3 && <span className="bd-label">+{task.labels.length - 3}</span>}
        </div>
      )}

      {phase && run && isRunActive(run) && (
        <div
          className="bd-segs"
          style={{ gridTemplateColumns: `repeat(${phase.total}, 1fr)` }}
          role="img"
          aria-label={`Phase ${phase.index} of ${phase.total}${phase.name ? `: ${phase.name}` : ""}`}
          title={`Phase ${phase.index}/${phase.total}${phase.name ? ` · ${phase.name}` : ""}`}
        >
          {Array.from({ length: phase.total }, (_, k) => (
            <span key={k} className={k < phase.index - 1 ? "on" : k === phase.index - 1 ? "on current" : ""} />
          ))}
        </div>
      )}

      <div className="bd-card-foot">
        {reviewing ? (
          <span className="badge tone-warn" title="The reviewer is checking the acceptance criteria">
            <span className="dot dot-sm pulse" aria-hidden />
            Reviewing
          </span>
        ) : (
          run && <RunBadge run={run} />
        )}
        <span className="bd-spacer" />
        <span className="bd-assignee" title={`Assignee: ${executorLabel(assignee)}`}>
          <ExecutorAvatar executor={assignee} />
        </span>
        {action && (
          <button
            type="button"
            className={`btn btn-xs ${primary ? "btn-primary" : ""}`}
            disabled={busy}
            onClick={act}
          >
            {ACTION_LABEL[action]}
          </button>
        )}
      </div>
    </article>
  );
}
