import type { DragEvent, MouseEvent } from "react";
import { isRunActive } from "../../domain/hooks/runs";
import { taskKey, type Executor, type Project, type Repo, type RunLight, type Task } from "../../domain/types";
import { executorLabel } from "../executors";
import { RunBadge } from "../runs";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/Tooltip";
import { PriorityBars } from "../tasks/bits";
import { isClosed, providerLabel } from "../tasks/status";
import type { PhaseProgress } from "./usePhases";

export type CardAction = "run" | "retry" | "open-run" | "choose-repo";

export interface CardModel {
  task: Task;
  project: Project | undefined;
  repo: Repo | undefined;
  assignee: Executor;
  run: RunLight | undefined;
  phase: PhaseProgress | undefined;
}

/** The card's main action based on repo, current run and status. */
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

export const ACTION_LABEL: Record<CardAction, string> = {
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
  /** Alt+arrows on the card: reorder or change column without a mouse. */
  onKeyMove: (key: "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight") => void;
  /** Right click or Shift+F10: opens the card's context menu at that point. */
  onMenu: (at: { x: number; y: number }) => void;
}

export function TaskCard({ model, showProject, busy, dragging, onOpen, onAction, onDragStart, onDragEnd, onKeyMove, onMenu }: Props) {
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
      className={`bd-card ${dragging ? "dragging" : ""} ${busy ? "busy" : ""}`}
      draggable
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onClick={onOpen}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu({ x: e.clientX, y: e.clientY });
      }}
      aria-label={showProject && project ? `${id} ${task.title}, ${project.name}` : `${id} ${task.title}`}
      title={id}
    >
      {showProject && project && (
        <span className="bd-card-stripe" style={{ background: project.color }} title={project.name} aria-hidden />
      )}

      {/* Real button to open with the keyboard; the whole card also opens on click.
          `draggable`: WebKit never starts a drag from a <button>, and the title covers most of
          the card; the drag bubbles to the card, which sets the whole card as the image. */}
      <button
        type="button"
        className="bd-card-title"
        draggable
        aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown Alt+ArrowLeft Alt+ArrowRight Shift+F10"
        title="Alt+arrows to move · Shift+F10 for actions"
        onClick={(e) => {
          e.stopPropagation();
          onOpen();
        }}
        onKeyDown={(e) => {
          if ((e.shiftKey && e.key === "F10") || e.key === "ContextMenu") {
            e.preventDefault();
            const r = e.currentTarget.getBoundingClientRect();
            onMenu({ x: r.left, y: r.bottom + 4 });
            return;
          }
          if (!e.altKey) return;
          if (e.key === "ArrowUp" || e.key === "ArrowDown" || e.key === "ArrowLeft" || e.key === "ArrowRight") {
            e.preventDefault();
            onKeyMove(e.key);
          }
        }}
      >
        {task.title}
      </button>

      {(src?.moved || reviewing || run) && (
        <div className="bd-card-run">
          {src?.moved && (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="badge tone-warn">
                  <span className="dot dot-sm" aria-hidden />
                  <span className="badge-label">Moved in {providerLabel(src.provider)}</span>
                </span>
              </TooltipTrigger>
              <TooltipContent>
                Moved in {providerLabel(src.provider)} · Moved from {src.moved.fromProject.name} to{" "}
                {src.moved.toProject?.name ?? "no project"}. Open the task to decide.
              </TooltipContent>
            </Tooltip>
          )}
          {reviewing ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="badge tone-warn">
                  <span className="dot dot-sm pulse" aria-hidden />
                  <span className="badge-label">Reviewing</span>
                </span>
              </TooltipTrigger>
              <TooltipContent>Reviewing · The reviewer is checking the acceptance criteria</TooltipContent>
            </Tooltip>
          ) : (
            run && <RunBadge run={run} tooltip />
          )}
        </div>
      )}

      {phase && run && isRunActive(run) && (
        <div
          className="bd-progress"
          role="img"
          aria-label={`Phase ${phase.index} of ${phase.total}${phase.name ? `: ${phase.name}` : ""}`}
          title={`Phase ${phase.index}/${phase.total}${phase.name ? ` · ${phase.name}` : ""}`}
        >
          <span style={{ width: `${Math.min(100, (phase.index / phase.total) * 100)}%` }} />
        </div>
      )}

      <div className="bd-card-foot">
        {src ? (
          <span className="bd-ext mono" title={`${providerLabel(src.provider)} · ${src.externalState?.name ?? "unknown state"}`}>
            {src.identifier}
          </span>
        ) : (
          <span className="bd-local">Local</span>
        )}
        <span className="bd-sep" aria-hidden>·</span>
        <span className={`bd-repo mono ${repo ? "" : "bd-danger"}`}>{repo ? repo.name : "Repo removed"}</span>
        {src?.syncError && (
          <span className="bd-sync-err" title={`Sync failed: ${src.syncError}`} role="img" aria-label="Sync failed">
            ⚠
          </span>
        )}
        <span className="bd-spacer" />
        <span className={`bd-prio ${action ? "has-act" : ""}`}>
          <PriorityBars priority={task.priority} />
        </span>
        {action && (
          <button
            type="button"
            className={`btn btn-xs bd-act ${primary ? "btn-primary" : ""}`}
            disabled={busy}
            title={primary ? `Assignee: ${executorLabel(assignee)}` : undefined}
            onClick={act}
          >
            {ACTION_LABEL[action]}
          </button>
        )}
      </div>
    </article>
  );
}
