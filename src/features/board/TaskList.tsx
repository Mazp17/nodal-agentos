import type { Project, Repo, RunLight, Task, TaskStatus } from "../../domain/types";
import { RunBadge } from "../runs";
import { PriorityBars, StatusRing } from "../tasks/bits";
import { STATUS_META } from "../tasks/status";
import "./board.css";

export interface TaskListProps {
  /** Already filtered by the board's toolbar. */
  tasks: Task[];
  /** Status order of the groups. */
  statuses: TaskStatus[];
  /** Row order within a group (the board's "Order"). */
  order: (a: Task, b: Task) => number;
  /** Shows the project on each row ("All projects"). */
  showProject: boolean;
  projectById: Map<string, Project>;
  repoById: Map<string, Repo>;
  latest: Map<string, RunLight>;
  keyOf: (t: Task) => string;
  onOpen: (taskId: string) => void;
  onMenu: (taskId: string, at: { x: number; y: number }) => void;
}

/** The board's List view: tasks grouped by status. */
export function TaskList({ tasks, statuses, order, showProject, projectById, repoById, latest, keyOf, onOpen, onMenu }: TaskListProps) {
  const groups = statuses
    .map((status) => ({
      status,
      rows: tasks.filter((t) => t.status === status).sort(order),
    }))
    .filter((g) => g.rows.length > 0);

  return (
    <div className="tv-body">
      {groups.map((g) => (
        <section key={g.status} className="tv-group" aria-label={STATUS_META[g.status].label}>
          <header className="tv-group-head">
            <StatusRing status={g.status} />
            <span className="tv-group-name">{STATUS_META[g.status].label}</span>
            <span className="tv-group-count num">{g.rows.length}</span>
          </header>
          <ul className="tv-rows">
            {g.rows.map((t) => {
              const p = projectById.get(t.projectId);
              const repo = repoById.get(t.repoId);
              const run = latest.get(t.id);
              return (
                <li key={t.id}>
                  <button
                    type="button"
                    className="tv-row"
                    onClick={() => onOpen(t.id)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      onMenu(t.id, { x: e.clientX, y: e.clientY });
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
                        e.preventDefault();
                        const r = e.currentTarget.getBoundingClientRect();
                        onMenu(t.id, { x: r.left + 24, y: r.bottom });
                      }
                    }}
                  >
                    <span title={STATUS_META[t.status].label} className="tv-st">
                      <StatusRing status={t.status} size={14} />
                      <span className="sr-only">{STATUS_META[t.status].label}</span>
                    </span>
                    <span className="mono tv-id">{keyOf(t)}</span>
                    <span className="ellipsis tv-title">{t.title}</span>
                    <span className="tv-where">
                      {showProject && p && (
                        <>
                          <span className="tv-proj-dot" style={{ background: p.color }} aria-hidden />
                          <span>{p.name}</span>
                          <span className="tv-sep" aria-hidden>·</span>
                        </>
                      )}
                      <span className={`mono ellipsis ${repo ? "" : "tv-danger"}`}>{repo ? repo.name : "Repo removed"}</span>
                      {t.source && (
                        <>
                          <span className="tv-sep" aria-hidden>·</span>
                          <span className="mono">{t.source.identifier}</span>
                        </>
                      )}
                    </span>
                    <span className="tv-run">{run && <RunBadge run={run} />}</span>
                    <span className="tv-prio">
                      <PriorityBars priority={t.priority} />
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      ))}
    </div>
  );
}
