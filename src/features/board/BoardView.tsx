import { useEffect, useMemo, useState, type DragEvent, type ReactNode } from "react";
import { moveTask } from "../../domain/api";
import { latestRunByTask, useAllRuns } from "../../domain/hooks/runs";
import { invalidate, useProjectList, useRepos, useTasks } from "../../domain/hooks/store";
import { taskKey, type Task, type TaskStatus } from "../../domain/types";
import { useToast } from "../../ui/Toasts";
import { resolveExecutor } from "../executors";
import { NewTaskDialog } from "../tasks/NewTaskDialog";
import { StatusRing } from "../tasks/bits";
import { BOARD_COLUMNS, HIDDEN_COLUMNS, providerLabel, STATUS_META } from "../tasks/status";
import { useLaunch } from "../tasks/useLaunch";
import { FilterMenu } from "./FilterMenu";
import { TaskCard, type CardAction, type CardModel } from "./TaskCard";
import { useRunPhases } from "./usePhases";
import "./board.css";

export interface BoardViewProps {
  /** `null`: todos los proyectos (la card muestra el proyecto). */
  projectId: string | null;
  onOpenTask: (taskId: string) => void;
  onOpenRun: (runId: string) => void;
  /** Estado "sin proyectos": si viene, muestra "New project". */
  onNewProject?: () => void;
  /** Estado "sin repos": si viene, muestra "Project settings". */
  onOpenProjectSettings?: (projectId: string) => void;
}

interface Filters {
  q: string;
  repo: string | null;
  source: string | null;
  status: string | null;
  label: string | null;
}
const NO_FILTERS: Filters = { q: "", repo: null, source: null, status: null, label: null };

const DRAG_TYPE = "text/x-nodal-task";

const byPosition = (a: Task, b: Task) => a.position - b.position || a.number - b.number;

/** Posición entre vecinos de la columna destino (sin la tarea arrastrada). */
function positionAt(column: Task[], index: number): number {
  const before = column[index - 1];
  const after = column[index];
  if (before && after) return (before.position + after.position) / 2;
  if (before) return before.position + 1;
  if (after) return after.position - 1;
  return 1;
}

export function BoardView({ projectId, onOpenTask, onOpenRun, onNewProject, onOpenProjectSettings }: BoardViewProps) {
  const push = useToast();
  const launch = useLaunch();
  const projects = useProjectList();
  const repos = useRepos(projectId);
  const tasks = useTasks(projectId);
  const runs = useAllRuns();

  const [filters, setFilters] = useState<Filters>(NO_FILTERS);
  const [showHidden, setShowHidden] = useState<Record<string, boolean>>({});
  const [newTask, setNewTask] = useState(false);
  const [busy, setBusy] = useState<Set<string>>(new Set());
  // Cambios locales (arrastre) hasta que el polling traiga algo igual o más nuevo.
  const [patched, setPatched] = useState<Map<string, Task>>(new Map());
  const [drag, setDrag] = useState<{ id: string; status: TaskStatus; index: number } | null>(null);

  useEffect(() => {
    setFilters(NO_FILTERS);
    setShowHidden({});
    setPatched(new Map());
  }, [projectId]);

  const projectById = useMemo(() => new Map((projects.data ?? []).map((p) => [p.id, p])), [projects.data]);
  const repoById = useMemo(() => new Map((repos.data ?? []).map((r) => [r.id, r])), [repos.data]);

  const allTasks = useMemo(() => {
    const list = tasks.data ?? [];
    if (!patched.size) return list;
    return list.map((t) => {
      const p = patched.get(t.id);
      return p && p.updatedAt >= t.updatedAt ? p : t;
    });
  }, [tasks.data, patched]);

  // Olvida los parches que el servidor ya superó.
  useEffect(() => {
    if (!patched.size || !tasks.data) return;
    const stale = tasks.data.filter((t) => {
      const p = patched.get(t.id);
      return p && t.updatedAt >= p.updatedAt && t.status === p.status && t.position === p.position;
    });
    if (stale.length) {
      setPatched((m) => {
        const n = new Map(m);
        for (const t of stale) n.delete(t.id);
        return n;
      });
    }
  }, [tasks.data, patched]);

  const latest = useMemo(() => latestRunByTask(runs.data), [runs.data]);
  const taskIds = useMemo(() => new Set(allTasks.map((t) => t.id)), [allTasks]);
  const viewRuns = useMemo(() => [...latest.values()].filter((r) => r.taskId && taskIds.has(r.taskId)), [latest, taskIds]);
  const phases = useRunPhases(viewRuns);

  const labels = useMemo(() => [...new Set(allTasks.flatMap((t) => t.labels))].sort(), [allTasks]);
  const providers = useMemo(
    () => [...new Set(allTasks.flatMap((t) => (t.source ? [t.source.provider] : [])))].sort(),
    [allTasks],
  );

  const q = filters.q.trim().toLowerCase();
  const visible = allTasks.filter((t) => {
    if (filters.repo && t.repoId !== filters.repo) return false;
    if (filters.source && (filters.source === "local" ? !!t.source : t.source?.provider !== filters.source)) return false;
    if (filters.label && !t.labels.includes(filters.label)) return false;
    if (q) {
      const p = projectById.get(t.projectId);
      const hay = `${p ? taskKey(p.key, t.number) : ""} ${t.title} ${t.source?.identifier ?? ""}`.toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });

  const columns: TaskStatus[] = filters.status
    ? [filters.status as TaskStatus]
    : [
        ...(showHidden.backlog ? (["backlog"] as TaskStatus[]) : []),
        ...BOARD_COLUMNS,
        ...(showHidden.canceled ? (["canceled"] as TaskStatus[]) : []),
      ];
  const byColumn = (s: TaskStatus) => visible.filter((t) => t.status === s).sort(byPosition);
  const hiddenCounts = HIDDEN_COLUMNS.filter((s) => !filters.status && !showHidden[s]).map((s) => ({
    status: s,
    count: visible.filter((t) => t.status === s).length,
  }));

  const model = (t: Task): CardModel => {
    const repo = repoById.get(t.repoId);
    const project = projectById.get(t.projectId);
    const run = latest.get(t.id);
    return {
      task: t,
      project,
      repo,
      assignee: resolveExecutor(t, repo, project),
      run,
      phase: run ? phases.get(run.id) : undefined,
    };
  };

  const onAction = async (m: CardModel, a: CardAction) => {
    if (a === "choose-repo") return onOpenTask(m.task.id);
    if (a === "open-run") {
      if (m.run) onOpenRun(m.run.id);
      return;
    }
    const id = m.task.id;
    setBusy((s) => new Set(s).add(id));
    const name = m.project ? taskKey(m.project.key, m.task.number) : m.task.title;
    await launch(id, name, { kind: "run" });
    setBusy((s) => {
      const n = new Set(s);
      n.delete(id);
      return n;
    });
  };

  // ---------- Arrastre ----------

  const onDragStart = (t: Task) => (e: DragEvent<HTMLElement>) => {
    e.dataTransfer.setData(DRAG_TYPE, t.id);
    e.dataTransfer.effectAllowed = "move";
    setDrag({ id: t.id, status: t.status, index: -1 });
  };

  const dropIndex = (e: DragEvent<HTMLElement>, list: Task[]) => {
    const cards = [...e.currentTarget.querySelectorAll<HTMLElement>("[data-card-id]")].filter(
      (el) => el.dataset.cardId !== drag?.id,
    );
    const y = e.clientY;
    const i = cards.findIndex((el) => {
      const r = el.getBoundingClientRect();
      return y < r.top + r.height / 2;
    });
    return i === -1 ? list.length : i;
  };

  const onDragOver = (status: TaskStatus, list: Task[]) => (e: DragEvent<HTMLElement>) => {
    if (!drag) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    const index = dropIndex(e, list);
    if (drag.status !== status || drag.index !== index) setDrag({ ...drag, status, index });
  };

  const setPatch = (t: Task) => setPatched((m) => new Map(m).set(t.id, t));
  const dropPatch = (id: string) =>
    setPatched((m) => {
      const n = new Map(m);
      n.delete(id);
      return n;
    });

  /**
   * Coloca `task` en `status`, en el índice `index` de `others` (la columna destino ordenada,
   * sin la tarea). Si el hueco entre vecinos se agotó, renumera la columna antes.
   */
  const place = async (task: Task, status: TaskStatus, others: Task[], index: number) => {
    const from = byColumn(task.status).findIndex((t) => t.id === task.id);
    if (task.status === status && from === index) return;
    try {
      let column = others;
      const before = column[index - 1];
      const after = column[index];
      if (before && after && after.position - before.position < 1e-6) {
        const renumbered: Task[] = [];
        for (const [k, t] of column.entries()) {
          const saved = await moveTask(t.id, t.status, k + 1);
          setPatch(saved);
          renumbered.push(saved);
        }
        column = renumbered;
      }
      const position = positionAt(column, index);
      setPatch({ ...task, status, position });
      const saved = await moveTask(task.id, status, position);
      setPatch(saved);
      if (task.status !== status) invalidate("tasks");
    } catch (err) {
      dropPatch(task.id);
      push("Couldn't move the task", String(err), "danger");
    }
  };

  const onDrop = (status: TaskStatus, others: Task[]) => (e: DragEvent<HTMLElement>) => {
    e.preventDefault();
    const id = e.dataTransfer.getData(DRAG_TYPE) || drag?.id;
    const index = dropIndex(e, others);
    setDrag(null);
    const task = id ? allTasks.find((t) => t.id === id) : undefined;
    if (task) void place(task, status, others, index);
  };

  /** Teclado: Alt+↑/↓ reordena en la columna; Alt+←/→ la pasa a la columna vecina. */
  const onKeyMove = (task: Task, key: string) => {
    const others = byColumn(task.status).filter((t) => t.id !== task.id);
    const from = byColumn(task.status).findIndex((t) => t.id === task.id);
    if (key === "ArrowUp" && from > 0) void place(task, task.status, others, from - 1);
    else if (key === "ArrowDown" && from < others.length) void place(task, task.status, others, from + 1);
    else if (key === "ArrowLeft" || key === "ArrowRight") {
      const i = columns.indexOf(task.status);
      const to = columns[i + (key === "ArrowLeft" ? -1 : 1)];
      if (i >= 0 && to) void place(task, to, byColumn(to), 0);
    }
  };

  // ---------- Estados ----------

  const loadError = tasks.error ?? repos.error ?? projects.error;
  const noProjects = projects.data !== undefined && projects.data.length === 0;
  const noRepos = projectId !== null && repos.data !== undefined && repos.data.length === 0;
  const loading = tasks.data === undefined || repos.data === undefined || projects.data === undefined;
  const hasFilters = !!(q || filters.repo || filters.source || filters.status || filters.label);

  const set = (p: Partial<Filters>) => setFilters((f) => ({ ...f, ...p }));
  const statusOptions = [...HIDDEN_COLUMNS.slice(0, 1), ...BOARD_COLUMNS, ...HIDDEN_COLUMNS.slice(1)].map((s) => ({
    value: s,
    label: STATUS_META[s].label,
    dot: STATUS_META[s].color,
  }));

  let body;
  if (noProjects) {
    body = (
      <CenterState title="No projects yet" text="Projects hold your repos and tasks. Create one to start handing work to Claude.">
        {onNewProject && (
          <button type="button" className="btn btn-primary" onClick={onNewProject}>
            New project
          </button>
        )}
      </CenterState>
    );
  } else if (loadError && (tasks.data === undefined || repos.data === undefined || projects.data === undefined)) {
    body = (
      <CenterState title="Couldn't load the board" text={loadError}>
        <button type="button" className="btn" onClick={() => invalidate("projects", "repos", "tasks", "runs")}>
          Retry
        </button>
      </CenterState>
    );
  } else if (noRepos) {
    body = (
      <CenterState title="Add a repo to start" text="Every task runs in one repo of this project. Add the root of a git checkout.">
        {onOpenProjectSettings && projectId && (
          <button type="button" className="btn btn-primary" onClick={() => onOpenProjectSettings(projectId)}>
            Project settings
          </button>
        )}
      </CenterState>
    );
  } else if (loading) {
    body = (
      <div className="bd-columns" aria-busy="true" aria-label="Loading tasks">
        {BOARD_COLUMNS.map((s, i) => (
          <div key={s} className="bd-col">
            <div className="bd-skel-head" />
            {Array.from({ length: 3 - (i % 2) }, (_, k) => (
              <div key={k} className="bd-skel-card" style={{ height: 76 + ((i + k) % 3) * 18 }} />
            ))}
          </div>
        ))}
      </div>
    );
  } else if (visible.length === 0) {
    body = (
      <CenterState
        title={hasFilters ? "No tasks match" : "No tasks yet"}
        text={
          hasFilters
            ? "Clear filters, or create a task and pick one of this project's repos."
            : "Create a task, or import issues from a connected source."
        }
      >
        {hasFilters && (
          <button type="button" className="btn" onClick={() => setFilters(NO_FILTERS)}>
            Clear filters
          </button>
        )}
        <button type="button" className="btn btn-primary" onClick={() => setNewTask(true)}>
          New task
        </button>
      </CenterState>
    );
  } else {
    body = (
      <div className="bd-columns">
        {columns.map((status) => {
          const list = byColumn(status);
          const meta = STATUS_META[status];
          const others = list.filter((t) => t.id !== drag?.id);
          const over = drag && drag.status === status && drag.index >= 0 ? drag.index : -1;
          return (
            <section
              key={status}
              className={`bd-col ${over >= 0 ? "drop-target" : ""}`}
              aria-label={`${meta.label}, ${list.length} tasks`}
              onDragOver={onDragOver(status, others)}
              onDragLeave={(e) => {
                if (!e.currentTarget.contains(e.relatedTarget as Node | null) && drag?.status === status) {
                  setDrag({ ...drag, index: -1 });
                }
              }}
              onDrop={onDrop(status, others)}
            >
              <header className="bd-col-head">
                <StatusRing status={status} />
                <span className="bd-col-name">{meta.label}</span>
                <span className="bd-col-count num">{list.length}</span>
                {HIDDEN_COLUMNS.includes(status) && !filters.status && (
                  <button
                    type="button"
                    className="bd-col-hide"
                    aria-label={`Hide ${meta.label}`}
                    onClick={() => setShowHidden((h) => ({ ...h, [status]: false }))}
                  >
                    Hide
                  </button>
                )}
              </header>
              {(() => {
                let k = -1;
                return list.map((t) => {
                  const isDragged = t.id === drag?.id;
                  if (!isDragged) k++;
                  const m = model(t);
                  return (
                    <div key={t.id} data-card-id={t.id} className="bd-slot">
                      {!isDragged && over === k && <div className="bd-drop-line" />}
                      <TaskCard
                        model={m}
                        showProject={projectId === null}
                        busy={busy.has(t.id)}
                        dragging={isDragged}
                        onOpen={() => onOpenTask(t.id)}
                        onAction={(a) => void onAction(m, a)}
                        onDragStart={onDragStart(t)}
                        onDragEnd={() => setDrag(null)}
                        onKeyMove={(k) => onKeyMove(t, k)}
                      />
                    </div>
                  );
                });
              })()}
              {over >= others.length && <div className="bd-drop-line" />}
              {list.length === 0 && over < 0 && <div className="bd-col-empty">Nothing here</div>}
            </section>
          );
        })}
        {hiddenCounts.some((h) => h.count > 0) && (
          <div className="bd-hidden-toggles">
            {hiddenCounts
              .filter((h) => h.count > 0)
              .map((h) => (
                <button
                  key={h.status}
                  type="button"
                  className="bd-hidden-btn"
                  onClick={() => setShowHidden((s) => ({ ...s, [h.status]: true }))}
                >
                  {STATUS_META[h.status].label} · {h.count}
                </button>
              ))}
          </div>
        )}
      </div>
    );
  }

  const showTools = !noProjects && !noRepos;

  return (
    <div className="bd-root">
      {showTools && (
        <div className="bd-toolbar" role="toolbar" aria-label="Board filters">
          <input
            className="input bd-q"
            placeholder="Filter"
            aria-label="Filter tasks"
            value={filters.q}
            onChange={(e) => set({ q: e.target.value })}
          />
          <FilterMenu
            label="Repo"
            value={filters.repo}
            options={(repos.data ?? []).map((r) => ({
              value: r.id,
              label: projectId === null ? `${projectById.get(r.projectId)?.name ?? "?"} / ${r.name}` : r.name,
            }))}
            onChange={(v) => set({ repo: v })}
          />
          <FilterMenu
            label="Source"
            value={filters.source}
            options={[{ value: "local", label: "Local" }, ...providers.map((p) => ({ value: p, label: providerLabel(p) }))]}
            onChange={(v) => set({ source: v })}
          />
          <FilterMenu label="Status" value={filters.status} options={statusOptions} onChange={(v) => set({ status: v })} />
          <FilterMenu
            label="Label"
            value={filters.label}
            options={labels.map((l) => ({ value: l, label: l }))}
            onChange={(v) => set({ label: v })}
          />
          {hasFilters && (
            <button type="button" className="btn btn-ghost btn-sm" onClick={() => setFilters(NO_FILTERS)}>
              Clear
            </button>
          )}
          {runs.error && (
            <span className="bd-tool-warn" title={runs.error}>
              Run status unavailable
            </span>
          )}
        </div>
      )}
      {body}
      {newTask && (
        <NewTaskDialog
          projectId={projectId}
          defaultRepoId={filters.repo}
          onClose={() => setNewTask(false)}
          onSaved={() => setNewTask(false)}
        />
      )}
    </div>
  );
}

function CenterState({ title, text, children }: { title: string; text: string; children?: ReactNode }) {
  return (
    <div className="center-state">
      <div className="center-state-body">
        <div className="state-icon-empty" aria-hidden />
        <div className="center-state-title">{title}</div>
        <div className="center-state-text">{text}</div>
        {children && <div className="center-state-actions">{children}</div>}
      </div>
    </div>
  );
}
