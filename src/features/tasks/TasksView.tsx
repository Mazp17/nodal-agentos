import { useMemo, useState } from "react";
import { useLatestRunByTask } from "../../domain/hooks/runs";
import { useProjectList, useRepos, useTasks } from "../../domain/hooks/store";
import { taskKey, type Task } from "../../domain/types";
import { RunBadge } from "../runs";
import { NewTaskDialog } from "./NewTaskDialog";
import { StatusRing } from "./bits";
import { isClosed, STATUS_META } from "./status";
import "./tasks.css";

export interface TasksViewProps {
  /** `null`: todos los proyectos. */
  projectId: string | null;
  onOpenTask: (taskId: string) => void;
}

type Scope = "open" | "done" | "all";

const SCOPES: [Scope, string][] = [
  ["open", "Open"],
  ["done", "Done"],
  ["all", "All"],
];

/** Lista de tareas agrupada por repo. */
export function TasksView({ projectId, onOpenTask }: TasksViewProps) {
  const [scope, setScope] = useState<Scope>("open");
  const [newTask, setNewTask] = useState(false);
  const tasks = useTasks(projectId);
  const repos = useRepos(projectId);
  const projects = useProjectList();

  const latest = useLatestRunByTask();
  const projectById = useMemo(() => new Map((projects.data ?? []).map((p) => [p.id, p])), [projects.data]);

  const list = useMemo(
    () =>
      (tasks.data ?? []).filter((t) =>
        scope === "all" ? true : scope === "done" ? isClosed(t.status) : !isClosed(t.status),
      ),
    [tasks.data, scope],
  );

  const groups = useMemo(() => {
    const repoList = [...(repos.data ?? [])].sort(
      (a, b) => a.projectId.localeCompare(b.projectId) || a.position - b.position,
    );
    const known = new Set(repoList.map((r) => r.id));
    const byRepo = new Map<string, Task[]>();
    for (const t of list) {
      const k = known.has(t.repoId) ? t.repoId : "__missing";
      byRepo.set(k, [...(byRepo.get(k) ?? []), t]);
    }
    const sortRows = (rows: Task[]) =>
      rows.sort((a, b) => a.projectId.localeCompare(b.projectId) || b.number - a.number);
    const out = repoList
      .filter((r) => byRepo.has(r.id))
      .map((r) => ({ id: r.id, name: r.name, path: r.path, projectId: r.projectId, missing: false, rows: sortRows(byRepo.get(r.id) ?? []) }));
    const missing = byRepo.get("__missing");
    if (missing) {
      out.push({ id: "__missing", name: "Repo removed", path: "Choose a new repo for these tasks", projectId: "", missing: true, rows: sortRows(missing) });
    }
    return out;
  }, [list, repos.data]);

  const loading = (tasks.data === undefined && !tasks.error) || (repos.data === undefined && !repos.error);

  return (
    <div className="tv-root">
      <div className="tv-bar">
        <div className="segmented tk-seg" role="radiogroup" aria-label="Show">
          {SCOPES.map(([k, l]) => (
            <button
              key={k}
              type="button"
              role="radio"
              aria-checked={scope === k}
              className={`tk-seg-opt ${scope === k ? "on" : ""}`}
              onClick={() => setScope(k)}
            >
              {l}
            </button>
          ))}
        </div>
        <span className="tk-muted num">
          {list.length} task{list.length === 1 ? "" : "s"}
        </span>
      </div>

      <div className="tv-body">
        {tasks.error && tasks.data === undefined ? (
          <div className="banner banner-error" role="alert">
            {tasks.error}
          </div>
        ) : loading ? (
          <span className="tk-muted">Loading tasks…</span>
        ) : groups.length === 0 ? (
          <div className="center-state">
            <div className="center-state-body">
              <div className="state-icon-empty" aria-hidden />
              <div className="center-state-title">No tasks here</div>
              <div className="center-state-text">Create a task, or import issues from a connected source.</div>
              {(projects.data?.length ?? 0) > 0 && (
                <div className="center-state-actions">
                  <button type="button" className="btn btn-primary" onClick={() => setNewTask(true)}>
                    New task
                  </button>
                </div>
              )}
            </div>
          </div>
        ) : (
          groups.map((g) => (
            <section key={g.id} className="tv-group" aria-label={g.name}>
              <header className="tv-group-head">
                {projectId === null && !g.missing && (
                  <span className="tv-proj">
                    <span className="dlg-proj-dot" style={{ background: projectById.get(g.projectId)?.color }} aria-hidden />
                    {projectById.get(g.projectId)?.name}
                    <span className="tk-muted">/</span>
                  </span>
                )}
                <span className={`tv-group-name ${g.missing ? "tp-danger" : ""}`}>{g.name}</span>
                <span className="mono tk-muted tv-group-path ellipsis">{g.path}</span>
                <span className="tk-muted num tv-group-count">{g.rows.length}</span>
              </header>
              <ul className="tv-rows">
                {g.rows.map((t) => {
                  const p = projectById.get(t.projectId);
                  const run = latest.get(t.id);
                  return (
                    <li key={t.id}>
                      <button type="button" className="tv-row" onClick={() => onOpenTask(t.id)}>
                        <span title={STATUS_META[t.status].label} className="tv-st">
                          <StatusRing status={t.status} />
                          <span className="sr-only">{STATUS_META[t.status].label}</span>
                        </span>
                        <span className="mono tk-muted tv-id">{p ? taskKey(p.key, t.number) : ""}</span>
                        <span className="ellipsis tv-title">{t.title}</span>
                        <span className="mono tv-ext">{t.source ? t.source.identifier : "Local"}</span>
                        <span className="tv-run">{run && <RunBadge run={run} />}</span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            </section>
          ))
        )}
      </div>

      {newTask && <NewTaskDialog projectId={projectId} onClose={() => setNewTask(false)} onSaved={() => setNewTask(false)} />}
    </div>
  );
}
