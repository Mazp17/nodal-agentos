import { useCallback, useEffect, useMemo, useState, type DragEvent, type ReactNode } from "react";
import { deleteTask, moveTask, reorderTasks, updateTask } from "../../domain/api";
import { useAllRuns, useLatestRunByTask } from "../../domain/hooks/runs";
import { invalidate, useProjectList, useRepos, useSettings, useTasks } from "../../domain/hooks/store";
import { taskKey, type Priority, type Task, type TaskStatus } from "../../domain/types";
import { useConfirm } from "../../ui/ConfirmDialog";
import { EmptyState } from "../../ui/EmptyState";
import { readPref, writePref } from "../../shell/storage";
import { useToast } from "../../ui/Toasts";
import { resolveExecutor } from "../executors";
import { NewTaskDialog } from "../tasks/NewTaskDialog";
import { StatusRing } from "../tasks/bits";
import { BOARD_COLUMNS, HIDDEN_COLUMNS, PRIORITY_LABEL, providerLabel, STATUS_META } from "../tasks/status";
import { useAskLaunch } from "../tasks/LaunchPopover";
import { useLaunch } from "../tasks/useLaunch";
import { CardMenu } from "./CardMenu";
import { FilterMenu } from "./FilterMenu";
import { SortMenu, type BoardSort } from "./SortMenu";
import { TaskList } from "./TaskList";
import { cardAction, TaskCard, type CardAction, type CardModel } from "./TaskCard";
import { useRunPhases } from "./usePhases";
import "./board.css";

export interface BoardViewProps {
  /** `null`: every project (the card shows the project). */
  projectId: string | null;
  onOpenTask: (taskId: string) => void;
  onOpenRun: (runId: string) => void;
  /** "No projects" state: when set, shows "New project". */
  onNewProject?: () => void;
  /** "No repos" state: when set, shows "Project settings". */
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

/** Every status, in board order; Backlog and Canceled start collapsed into rails. */
const ALL_COLUMNS: TaskStatus[] = [HIDDEN_COLUMNS[0], ...BOARD_COLUMNS, HIDDEN_COLUMNS[1]];
const DEFAULT_COLLAPSED: Partial<Record<TaskStatus, boolean>> = { backlog: true, canceled: true };

type View = "board" | "list";
const VIEWS: [View, string][] = [
  ["board", "Board"],
  ["list", "List"],
];
const VIEW_PREF = "boardView";
const SORT_PREF = "boardSort";

/** Same order as the backend (`position, created_at, id`). */
const byPosition = (a: Task, b: Task) =>
  a.position - b.position || a.createdAt - b.createdAt || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

export function BoardView({ projectId, onOpenTask, onOpenRun, onNewProject, onOpenProjectSettings }: BoardViewProps) {
  const push = useToast();
  const ask = useConfirm();
  const launch = useLaunch();
  const askLaunch = useAskLaunch();
  const projects = useProjectList();
  const repos = useRepos(projectId);
  const tasks = useTasks(projectId);
  const runs = useAllRuns();
  const globalExecutor = useSettings().data?.defaultExecutor ?? null;

  const [filters, setFilters] = useState<Filters>(NO_FILTERS);
  const [view, setView] = useState<View>(() => (readPref(VIEW_PREF) === "list" ? "list" : "board"));
  const [sort, setSort] = useState<BoardSort>(() => (readPref(SORT_PREF) === "manual" ? "manual" : "last-run"));
  const [collapsed, setCollapsed] = useState(DEFAULT_COLLAPSED);
  const [menu, setMenu] = useState<{ taskId: string; at: { x: number; y: number } } | null>(null);
  const [newTask, setNewTask] = useState(false);
  const [busy, setBusy] = useState<Set<string>>(new Set());
  // Local changes (drag) until polling brings something equal or newer.
  const [patched, setPatched] = useState<Map<string, Task>>(new Map());
  const [drag, setDrag] = useState<{ id: string; status: TaskStatus; index: number; height: number } | null>(null);

  useEffect(() => {
    setFilters(NO_FILTERS);
    setCollapsed(DEFAULT_COLLAPSED);
    setMenu(null);
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

  // Drop the patches the server has already caught up with.
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

  const latest = useLatestRunByTask();
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

  /** Board layout: a status filter shows just that column, expanded. */
  const layout: TaskStatus[] = filters.status ? [filters.status as TaskStatus] : ALL_COLUMNS;
  const isRail = (s: TaskStatus) => !filters.status && !!collapsed[s];
  /** Expanded columns (keyboard moves go between these). */
  const columns = layout.filter((s) => !isRail(s));
  /** "Last run": most recently run first; tasks that never ran go after, in board order. */
  const lastRunAt = (t: Task) => latest.get(t.id)?.queuedAt ?? -Infinity;
  const order =
    sort === "manual" ? byPosition : (a: Task, b: Task) => lastRunAt(b) - lastRunAt(a) || byPosition(a, b);
  const byColumn = (s: TaskStatus) => visible.filter((t) => t.status === s).sort(order);

  const model = (t: Task): CardModel => {
    const repo = repoById.get(t.repoId);
    const project = projectById.get(t.projectId);
    const run = latest.get(t.id);
    return {
      task: t,
      project,
      repo,
      assignee: resolveExecutor(t, repo, project, globalExecutor),
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
    const name = m.project ? taskKey(m.project.key, m.task.number) : m.task.title;
    const config = await askLaunch({ name, repo: m.repo, executor: m.assignee });
    if (!config) return;
    setBusy((s) => new Set(s).add(id));
    await launch(id, name, { kind: "run", config });
    setBusy((s) => {
      const n = new Set(s);
      n.delete(id);
      return n;
    });
  };

  // ---------- Drag ----------

  const onDragStart = (t: Task) => (e: DragEvent<HTMLElement>) => {
    e.dataTransfer.setData(DRAG_TYPE, t.id);
    e.dataTransfer.effectAllowed = "move";
    setMenu(null);
    setDrag({ id: t.id, status: t.status, index: -1, height: e.currentTarget.offsetHeight });
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
  /** Discards these patches, except those a later drag already replaced. */
  const dropPatches = (mine: Task[]) =>
    setPatched((m) => {
      const n = new Map(m);
      for (const p of mine) if (n.get(p.id) === p) n.delete(p.id);
      return n.size === m.size ? m : n;
    });

  /**
   * Places `task` in `status`, before `others[index]` (the sorted visible column, without the
   * task). Sends the full order of the project's column (`reorder_tasks`, a single
   * transaction); tasks hidden by filters keep their relative place.
   */
  const place = async (task: Task, status: TaskStatus, others: Task[], index: number) => {
    const from = byColumn(task.status).findIndex((t) => t.id === task.id);
    if (task.status === status && from === index) return;
    // `reorder_tasks` requires a single project: in "All projects" the task's one is reordered.
    const column = allTasks
      .filter((t) => t.status === status && t.projectId === task.projectId && t.id !== task.id)
      .sort(byPosition);
    const anchor = others.slice(index).find((t) => t.projectId === task.projectId);
    const at = anchor ? column.findIndex((t) => t.id === anchor.id) : column.length;
    const cut = at === -1 ? column.length : at;
    const ordered = [...column.slice(0, cut), task, ...column.slice(cut)];
    const ids = ordered.map((t) => t.id);
    const mine = ordered.map((t, k) => ({ ...t, status, position: k + 1 }));
    mine.forEach(setPatch);
    try {
      if (task.status !== status) await moveTask(task.id, status, cut + 1);
      await reorderTasks(status, ids);
    } catch (err) {
      push("Couldn't move the task", String(err), "danger");
    } finally {
      // Patches only cover the outbound trip: once the list is re-read, the server wins. Otherwise
      // a patch with the same `updatedAt` could mask whatever the backend says indefinitely.
      await invalidate("tasks");
      dropPatches(mine);
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

  /** Keyboard: Alt+↑/↓ reorders within the column; Alt+←/→ moves it to the neighboring column. */
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

  // ---------- Context menu ----------

  const closeMenu = useCallback(() => setMenu(null), []);
  const keyOf = (t: Task) => {
    const p = projectById.get(t.projectId);
    return p ? taskKey(p.key, t.number) : `#${t.number}`;
  };

  const patchTask = async (t: Task, patch: { status?: TaskStatus; priority?: Priority }, msg: string) => {
    try {
      await updateTask(t.id, patch);
      push(msg, t.title, "ok");
    } catch (err) {
      push("Couldn't update the task", String(err), "danger");
    } finally {
      await invalidate("tasks");
    }
  };

  const removeTask = async (t: Task) => {
    const key = keyOf(t);
    const ok = await ask({
      title: `Delete ${key}?`,
      body: `"${t.title}" and its plan are deleted. This can't be undone.`,
      confirmLabel: "Delete task",
    });
    if (!ok) return;
    try {
      await deleteTask(t.id);
      push("Task deleted", t.title, "ok");
    } catch (err) {
      push("Couldn't delete the task", String(err), "danger");
    } finally {
      await invalidate("tasks");
    }
  };

  const copyId = async (t: Task) => {
    try {
      await navigator.clipboard.writeText(keyOf(t));
      push("Copied", keyOf(t), "ok");
    } catch (err) {
      push("Couldn't copy the ID", String(err), "danger");
    }
  };

  // ---------- States ----------

  const loadError = tasks.error ?? repos.error ?? projects.error;
  const noProjects = projects.data !== undefined && projects.data.length === 0;
  const noRepos = projectId !== null && repos.data !== undefined && repos.data.length === 0;
  const loading = tasks.data === undefined || repos.data === undefined || projects.data === undefined;
  const hasFilters = !!(q || filters.repo || filters.source || filters.status || filters.label);

  const set = (p: Partial<Filters>) => setFilters((f) => ({ ...f, ...p }));
  const statusOptions = ALL_COLUMNS.map((s) => ({
    value: s,
    label: STATUS_META[s].label,
    icon: <StatusRing status={s} />,
  }));

  let body;
  if (noProjects) {
    body = (
      <EmptyState title="No projects yet" description="Projects hold your repos and tasks. Create one to start handing work to Claude.">
        {onNewProject && (
          <button type="button" className="btn btn-primary" onClick={onNewProject}>
            New project
          </button>
        )}
      </EmptyState>
    );
  } else if (loadError && (tasks.data === undefined || repos.data === undefined || projects.data === undefined)) {
    body = (
      <ErrorState title="Couldn't load the board" text={loadError}>
        <button type="button" className="btn" onClick={() => invalidate("projects", "repos", "tasks", "runs")}>
          Retry
        </button>
      </ErrorState>
    );
  } else if (noRepos) {
    body = (
      <EmptyState title="Add a repo to start" description="Every task runs in one repo of this project. Add the root of a git checkout.">
        {onOpenProjectSettings && projectId && (
          <button type="button" className="btn btn-primary" onClick={() => onOpenProjectSettings(projectId)}>
            Project settings
          </button>
        )}
      </EmptyState>
    );
  } else if (loading && view === "list") {
    body = (
      <div className="tv-body" aria-busy="true" aria-label="Loading tasks">
        {[62, 48, 70, 40, 56].map((w) => (
          <div key={w} className="tv-skel-row">
            <span className="tv-skel tv-skel-ring" />
            <span className="tv-skel" style={{ width: 54 }} />
            <span className="tv-skel" style={{ width: `${w}%` }} />
          </div>
        ))}
      </div>
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
      <EmptyState
        title={hasFilters ? "No tasks match" : "No tasks yet"}
        description={
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
      </EmptyState>
    );
  } else if (view === "list") {
    body = (
      <TaskList
        tasks={visible}
        statuses={layout}
        order={order}
        showProject={projectId === null}
        projectById={projectById}
        repoById={repoById}
        latest={latest}
        keyOf={keyOf}
        onOpen={onOpenTask}
        onMenu={(taskId, at) => setMenu({ taskId, at })}
      />
    );
  } else {
    const dragged = drag ? allTasks.find((t) => t.id === drag.id) : undefined;
    const placeholder = dragged && (
      <div className="bd-drop-ph" style={{ height: drag?.height }} aria-hidden>
        <span className="bd-drop-ph-title">{dragged.title}</span>
        <span className="bd-drop-ph-meta mono">{keyOf(dragged)}</span>
      </div>
    );
    body = (
      <div className="bd-columns">
        {layout.map((status) => {
          const list = byColumn(status);
          const meta = STATUS_META[status];
          const others = list.filter((t) => t.id !== drag?.id);
          const over = drag && drag.status === status && drag.index >= 0 ? drag.index : -1;
          const dragProps = {
            onDragOver: onDragOver(status, others),
            onDragLeave: (e: DragEvent<HTMLElement>) => {
              if (!e.currentTarget.contains(e.relatedTarget as Node | null) && drag?.status === status) {
                setDrag({ ...drag, index: -1 });
              }
            },
            onDrop: onDrop(status, others),
          };
          if (isRail(status)) {
            return (
              <button
                key={status}
                type="button"
                className={`bd-rail ${over >= 0 ? "drop-target" : ""}`}
                data-rail={status}
                aria-label={`Show ${meta.label}, ${list.length} tasks`}
                title={`Show ${meta.label}`}
                onClick={() => setCollapsed((c) => ({ ...c, [status]: false }))}
                {...dragProps}
              >
                <StatusRing status={status} />
                <span className="bd-rail-name">
                  {meta.label} · <span className="num">{list.length}</span>
                </span>
              </button>
            );
          }
          return (
            <section
              key={status}
              className={`bd-col ${over >= 0 ? "drop-target" : ""}`}
              aria-label={`${meta.label}, ${list.length} tasks`}
              {...dragProps}
            >
              <header className="bd-col-head">
                <StatusRing status={status} />
                <span className="bd-col-name">{meta.label}</span>
                <span className="bd-col-count num">{list.length}</span>
                <span className="bd-spacer" />
                {status === "todo" && (
                  <button type="button" className="icon-btn bd-col-btn" aria-label="New task" title="New task" onClick={() => setNewTask(true)}>
                    +
                  </button>
                )}
                {!filters.status && (
                  <button
                    type="button"
                    className="icon-btn bd-col-btn"
                    aria-label={`Collapse ${meta.label}`}
                    title="Collapse"
                    onClick={() => {
                      setCollapsed((c) => ({ ...c, [status]: true }));
                      // The button unmounts with its column: hand focus to the new rail.
                      requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-rail="${status}"]`)?.focus());
                    }}
                  >
                    ‹
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
                      {!isDragged && over === k && placeholder}
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
                        onMenu={(at) => setMenu({ taskId: t.id, at })}
                      />
                    </div>
                  );
                });
              })()}
              {over >= others.length && placeholder}
              {list.length === 0 && over < 0 && (
                <div className="bd-col-empty">{hasFilters ? "No matching tasks" : "Drop a task here"}</div>
              )}
            </section>
          );
        })}
      </div>
    );
  }

  const menuTask = menu ? allTasks.find((t) => t.id === menu.taskId) : undefined;
  const menuModel = menuTask ? model(menuTask) : undefined;

  const showTools = !noProjects && !noRepos;

  return (
    <div className="bd-root">
      {showTools && (
        <div className="bd-toolbar" role="toolbar" aria-label="Task filters">
          <input
            className="input bd-q"
            placeholder="Search tasks…"
            aria-label="Search tasks"
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
          <span className="bd-spacer" />
          {runs.error && (
            <span className="bd-tool-warn" title={runs.error}>
              Run status unavailable
            </span>
          )}
          <SortMenu
            value={sort}
            onChange={(v) => {
              setSort(v);
              writePref(SORT_PREF, v);
            }}
          />
          <div className="seg bd-view" role="radiogroup" aria-label="View">
            {VIEWS.map(([v, l]) => (
              <button
                key={v}
                type="button"
                role="radio"
                aria-checked={view === v}
                className="seg-opt"
                onClick={() => {
                  setView(v);
                  writePref(VIEW_PREF, v);
                }}
              >
                {l}
              </button>
            ))}
          </div>
        </div>
      )}
      {body}
      {menu && menuTask && menuModel && (
        <CardMenu
          model={menuModel}
          taskId={keyOf(menuTask)}
          at={menu.at}
          action={cardAction(menuModel)}
          busy={busy.has(menuTask.id)}
          onClose={closeMenu}
          onOpen={() => onOpenTask(menuTask.id)}
          onAction={(a) => void onAction(menuModel, a)}
          onStatus={(st) => {
            if (st !== menuTask.status) void patchTask(menuTask, { status: st }, `${keyOf(menuTask)} → ${STATUS_META[st].label}`);
          }}
          onPriority={(p) => {
            if (p !== menuTask.priority) void patchTask(menuTask, { priority: p }, `${keyOf(menuTask)} → ${PRIORITY_LABEL[p]}`);
          }}
          onCopyId={() => void copyId(menuTask)}
          onDelete={() => void removeTask(menuTask)}
        />
      )}
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

function ErrorState({ title, text, children }: { title: string; text: string; children?: ReactNode }) {
  return (
    <div className="center-state">
      <div className="center-state-body">
        <div className="state-icon-error" aria-hidden>
          !
        </div>
        <div className="center-state-title">{title}</div>
        <div className="center-state-text">{text}</div>
        {children && <div className="center-state-actions">{children}</div>}
      </div>
    </div>
  );
}
