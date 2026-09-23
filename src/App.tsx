import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  configApi,
  linearApi,
  resolveRepo,
  toLinearError,
  type AppConfig,
  type Board,
  type Issue,
  type LinearError,
  type Team,
  type Viewer,
} from "./features/linear/api";
import { RepoActivityPanel } from "./features/activity/RepoActivityPanel";
import { useActivitySummary } from "./features/activity/useActivitySummary";
import { BoardSkeleton, BoardView, EmptyState, ErrorState, type BoardTask } from "./features/linear/Board";
import { IssuePanel } from "./features/linear/IssuePanel";
import { IssueDetailCache } from "./features/linear/issueDetail";
import { Onboarding, SettingsView, type SettingsSection } from "./features/linear/Settings";
import { CommandPalette, type PaletteItem } from "./features/palette/CommandPalette";
import { useLauncher, useRunActions } from "./features/runs/actions";
import { RunDetailView } from "./features/runs/RunDetailView";
import { RunsView, type RunsTab } from "./features/runs/RunsView";
import { sessionKey, viewOfSession, type RunView } from "./features/runs/status";
import { useRuns } from "./features/runs/useRuns";
import { readLastWorkflow, useWorkflows, writeLastWorkflow } from "./features/runs/useWorkflows";
import { useTaskActions } from "./features/tasks/actions";
import { NewTaskDialog } from "./features/tasks/NewTaskDialog";
import { TaskPanel } from "./features/tasks/TaskPanel";
import { TasksView } from "./features/tasks/TasksView";
import type { FinishMode, Task } from "./features/tasks/types";
import { samePath, useTasks } from "./features/tasks/useTasks";
import { localGet, localSet } from "./lib/format";
import { Sidebar, type NavView } from "./shell/Sidebar";
import { FilterChip } from "./ui/FilterChip";
import { ToastProvider } from "./ui/Toasts";
import "./shell/shell.css";

const TEAM_FILTER_KEY = "agent-desk.teamFilter";
const ACTIVITY_REPO_KEY = "agent-desk.activityRepo";
/** Como las issues cerradas que trae el board: las tareas hechas se muestran 14 días. */
const DONE_TASK_WINDOW_MS = 14 * 24 * 60 * 60_000;
const DEFAULT_CONFIG: AppConfig = { repos: [], concurrency: 3 };
const UNASSIGNED = "__unassigned";

type View = NavView | "run";
type Source = "linear" | "local";

const VIEW_TITLE: Record<View, string> = {
  board: "Board",
  runs: "Runs",
  tasks: "Tasks",
  activity: "Activity",
  settings: "Settings",
  run: "Run",
};
const SHORTCUTS: Record<string, NavView> = { "1": "board", "2": "runs", "3": "tasks", "4": "activity", ",": "settings" };
const SOURCE_OPTIONS = [
  { value: "linear", label: "Linear" },
  { value: "local", label: "Local" },
];
const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

export default function App() {
  return (
    <ToastProvider>
      <Desk />
    </ToastProvider>
  );
}

function Desk() {
  const [cli, setCli] = useState<{ ok: boolean; text: string } | null>(null);
  const [keyConfigured, setKeyConfigured] = useState<boolean | null>(null);
  const [keyError, setKeyError] = useState<LinearError | null>(null);
  const [viewer, setViewer] = useState<Viewer | null>(null);
  const [viewerError, setViewerError] = useState<string | null>(null);

  const [view, setView] = useState<View>("board");
  const [runKey, setRunKey] = useState<string | null>(null);
  const [runFrom, setRunFrom] = useState<NavView>("board");
  const [runsTab, setRunsTab] = useState<RunsTab>("active");
  const [section, setSection] = useState<SettingsSection>("linear");
  const [panelIssue, setPanelIssue] = useState<string | null>(null);
  // Detalle de Linear cacheado mientras el panel sigue abierto (navegar entre
  // sub-issues no vuelve a pedirlo); se descarta al cerrarlo.
  const [detailCache] = useState(() => new IssueDetailCache());
  const [paletteOpen, setPaletteOpen] = useState(false);

  const [teams, setTeams] = useState<Team[]>([]);
  const [teamFilter, setTeamFilter] = useState<string>(() => localGet(TEAM_FILTER_KEY) ?? "");
  const [board, setBoard] = useState<Board | null>(null);
  const [boardError, setBoardError] = useState<LinearError | null>(null);
  const [loading, setLoading] = useState(false);
  const [lastSync, setLastSync] = useState<number | null>(null);

  const [config, setConfig] = useState<AppConfig>(DEFAULT_CONFIG);
  // Hasta que get_config responda bien, Settings no deja guardar (pisaría el archivo).
  const [configLoaded, setConfigLoaded] = useState(false);
  const [configError, setConfigError] = useState<string | null>(null);

  const [query, setQuery] = useState("");
  const [projectFilter, setProjectFilter] = useState<string | null>(null);
  const [assigneeFilter, setAssigneeFilter] = useState<string | null>(null);
  const [hasRepoFilter, setHasRepoFilter] = useState(false);
  const [sourceFilter, setSourceFilter] = useState<Source | null>(null);
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [tasksRepo, setTasksRepo] = useState<string | null>(null);
  const [activityRepo, setActivityRepo] = useState<string | null>(() => localGet(ACTIVITY_REPO_KEY));
  const [panelTask, setPanelTask] = useState<string | null>(null);
  const [taskDialog, setTaskDialog] = useState<{ task: Task | null } | null>(null);
  const [taskPlanVersion, setTaskPlanVersion] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [picks, setPicks] = useState<Record<string, string>>({});
  const [lastWorkflow, setLastWorkflow] = useState(readLastWorkflow);

  const runs = useRuns(keyConfigured === true);
  const catalogs = useWorkflows(config.repos.map((r) => r.path));
  const launcher = useLauncher(config, catalogs, lastWorkflow, runs);
  const actions = useRunActions(runs);
  // Comparte la lista de `claude agents` de useRuns: un solo `list_runs` por intervalo.
  const tasks = useTasks(null, keyConfigured === true, runs.loaded ? runs.runs : undefined);
  const taskActions = useTaskActions(tasks.refresh);
  const mappedRepos = useMemo(() => [...new Set(config.repos.map((r) => r.path))], [config.repos]);
  const activity = useActivitySummary(mappedRepos, keyConfigured === true);
  const repoOptions = useMemo(() => mappedRepos.map((r) => ({ value: r, label: basename(r) })), [mappedRepos]);
  const finishOf = useCallback(
    (repo: string): FinishMode | undefined => config.repos.find((r) => samePath(r.path, repo))?.finish,
    [config.repos],
  );
  const pickFile = useCallback(async (repo: string) => {
    const picked = await openDialog({ filters: [{ name: "Markdown", extensions: ["md"] }], defaultPath: repo });
    return typeof picked === "string" ? picked : null;
  }, []);

  const checkCli = useCallback(
    () =>
      invoke<string>("claude_version")
        .then((text) => setCli({ ok: true, text }))
        .catch((err) => setCli({ ok: false, text: String(err) })),
    [],
  );

  useEffect(() => {
    void checkCli();
    configApi
      .get()
      .then((c) => {
        setConfig(c);
        setConfigLoaded(true);
      })
      .catch((e) => setConfigError(String(e)));
  }, [checkCli]);

  const checkKey = useCallback(() => {
    setKeyError(null);
    linearApi
      .keyStatus()
      .then((s) => setKeyConfigured(s.configured))
      .catch((e) => setKeyError(toLinearError(e)));
  }, []);
  useEffect(checkKey, [checkKey]);

  const loadViewer = useCallback(
    () =>
      linearApi
        .viewer()
        .then((v) => {
          setViewer(v);
          setViewerError(null);
        })
        .catch((e) => setViewerError(toLinearError(e).message)),
    [],
  );
  useEffect(() => {
    if (keyConfigured) void loadViewer();
    else setViewer(null);
  }, [keyConfigured, loadViewer]);

  // Descarta respuestas viejas si se cambia el filtro con un fetch en vuelo.
  const requestId = useRef(0);
  const loadBoard = useCallback(async () => {
    const id = ++requestId.current;
    setLoading(true);
    setBoardError(null);
    try {
      // allSettled: si falla el board (p. ej. filtro viejo) igual queremos la lista de
      // teams para poder cambiar el filtro.
      const [ts, b] = await Promise.allSettled([linearApi.teams(), linearApi.board(teamFilter ? [teamFilter] : null)]);
      if (id !== requestId.current) return;
      if (ts.status === "fulfilled") setTeams(ts.value);
      if (b.status === "fulfilled") {
        setBoard(b.value);
        setLastSync(Date.now());
      }
      const failed = b.status === "rejected" ? b.reason : ts.status === "rejected" ? ts.reason : null;
      if (failed !== null) {
        const err = toLinearError(failed);
        if (err.kind === "missingKey") setKeyConfigured(false);
        setBoardError(err);
      }
    } finally {
      if (id === requestId.current) setLoading(false);
    }
  }, [teamFilter]);

  useEffect(() => {
    if (keyConfigured) void loadBoard();
  }, [keyConfigured, loadBoard]);

  // El board cargó, así que la key funciona: si el viewer había fallado (p. ej. sin
  // red), reintentarlo para que el sidebar no siga marcando Linear caído.
  useEffect(() => {
    if (lastSync && viewerError) void loadViewer();
    // Sólo tras cada sync exitoso.
  }, [lastSync]);

  // Si el team guardado ya no existe (key de otro workspace), volver a "todos".
  useEffect(() => {
    if (teamFilter && teams.length > 0 && !teams.some((t) => t.id === teamFilter)) {
      setTeamFilter("");
      localSet(TEAM_FILTER_KEY, "");
    }
  }, [teams, teamFilter]);

  function onKeyCleared() {
    requestId.current++;
    setLoading(false);
    setBoard(null);
    setTeams([]);
    setBoardError(null);
    setViewer(null);
    setViewerError(null);
    setSelected(new Set());
    setPanelIssue(null);
    setPanelTask(null);
    setRunKey(null);
    setView("board");
    setKeyConfigured(false);
  }

  // ---- Navegación ----
  const go = useCallback((v: NavView) => {
    setView(v);
    setPanelIssue(null);
    setPanelTask(null);
    setPaletteOpen(false);
  }, []);
  const openRun = useCallback(
    (key: string) => {
      setRunFrom((prev) => (view === "run" ? prev : view));
      setRunKey(key);
      setView("run");
      setPanelIssue(null);
      setPanelTask(null);
    },
    [view],
  );
  const openRunView = useCallback((v: RunView) => openRun(v.key), [openRun]);
  // Un solo drawer a la vez.
  const openIssue = useCallback((id: string) => {
    setPanelTask(null);
    setPanelIssue(id);
  }, []);
  const openTask = useCallback((id: string) => {
    setPanelIssue(null);
    setPanelTask(id);
  }, []);
  const openTaskDialog = useCallback((task: Task | null) => {
    setPaletteOpen(false);
    setTaskDialog({ task });
  }, []);
  const openSettings = (s: SettingsSection) => {
    setSection(s);
    go("settings");
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || keyConfigured !== true) return;
      if (e.metaKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      } else if (e.metaKey && e.key in SHORTCUTS) {
        e.preventDefault();
        go(SHORTCUTS[e.key]!);
      } else if (e.key === "Escape" && view === "run" && !panelIssue && !panelTask && !taskDialog && !paletteOpen) {
        setView(runFrom);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [keyConfigured, go, view, panelIssue, panelTask, taskDialog, paletteOpen, runFrom]);

  // ---- Board: filtros y selección ----
  const allIssues = useMemo(() => board?.issues ?? [], [board]);
  const issueById = useMemo(() => new Map(allIssues.map((i) => [i.id, i])), [allIssues]);
  // El panel también se cierra si la issue sale del board tras un refresh.
  const panelOpen = panelIssue !== null && issueById.has(panelIssue);
  useEffect(() => {
    if (!panelOpen) detailCache.clear();
  }, [panelOpen, detailCache]);
  const repoOf = useCallback((i: Issue) => resolveRepo(config, i.team.id, i.project?.id), [config]);
  const q = query.trim().toLowerCase();
  const visible = useMemo(
    () =>
      allIssues.filter(
        (i) =>
          sourceFilter !== "local" &&
          (!repoFilter || (repoOf(i) !== null && samePath(repoOf(i)!, repoFilter))) &&
          (!projectFilter || i.project?.id === projectFilter) &&
          (!assigneeFilter || (assigneeFilter === UNASSIGNED ? !i.assignee : i.assignee?.id === assigneeFilter)) &&
          (!hasRepoFilter || repoOf(i) !== null) &&
          (!q || `${i.identifier} ${i.title}`.toLowerCase().includes(q)),
      ),
    [allIssues, sourceFilter, repoFilter, projectFilter, assigneeFilter, hasRepoFilter, q, repoOf],
  );
  // Tareas locales en el board: los filtros propios de Linear (team, project, assignee)
  // no las afectan; sí la fuente, el repo y el texto.
  const boardTasks = useMemo((): BoardTask[] => {
    if (sourceFilter === "linear") return [];
    const since = Date.now() - DONE_TASK_WINDOW_MS;
    return tasks.tasks
      .filter(
        (t) =>
          (t.status !== "done" || (t.doneAt ?? t.createdAt) >= since) &&
          (!repoFilter || samePath(t.repoPath, repoFilter)) &&
          (!q || t.title.toLowerCase().includes(q)),
      )
      .map((task) => ({ task, view: tasks.current.get(task.id) }));
  }, [tasks.tasks, tasks.current, sourceFilter, repoFilter, q]);
  const projectOptions = useMemo(() => {
    const m = new Map<string, string>();
    for (const i of allIssues) if (i.project) m.set(i.project.id, i.project.name);
    return [...m].map(([value, label]) => ({ value, label })).sort((a, b) => a.label.localeCompare(b.label));
  }, [allIssues]);
  const assigneeOptions = useMemo(() => {
    const m = new Map<string, string>();
    for (const i of allIssues) if (i.assignee) m.set(i.assignee.id, i.assignee.displayName || i.assignee.name);
    const list = [...m].map(([value, label]) => ({ value, label })).sort((a, b) => a.label.localeCompare(b.label));
    return [...list, { value: UNASSIGNED, label: "Unassigned" }];
  }, [allIssues]);
  const hasFilters = !!(q || projectFilter || assigneeFilter || hasRepoFilter || teamFilter || sourceFilter || repoFilter);
  const setTeam = (v: string | null) => {
    setTeamFilter(v ?? "");
    localSet(TEAM_FILTER_KEY, v ?? "");
  };
  const clearFilters = () => {
    setQuery("");
    setProjectFilter(null);
    setAssigneeFilter(null);
    setHasRepoFilter(false);
    setSourceFilter(null);
    setRepoFilter(null);
    setTeam(null);
  };

  const activeRuns = runs.views.filter((v) => v.kind === "running" || v.kind === "starting");
  const queuedCount = runs.views.filter((v) => v.ir?.status === "queued").length;
  // Igual que el backend: un run `launching` ya ocupa slot; la cola sale en orden.
  const launchingCount = runs.views.filter((v) => v.ir?.status === "launching").length;
  const freeSlots = queuedCount > 0 ? 0 : Math.max(0, config.concurrency - activeRuns.length - launchingCount);

  const canLaunch = (i: Issue) =>
    repoOf(i) !== null && !runs.currentView.get(i.id)?.active && !launcher.pending.has(i.id);
  // Sólo cuenta lo seleccionado que sigue visible (los filtros pueden ocultar cards elegidas).
  const selectedIssues = visible.filter((i) => selected.has(i.id));
  const launchable = selectedIssues.filter(canLaunch);
  const skipped = selectedIssues.length - launchable.length;
  const willQueue = Math.max(0, launchable.length - freeSlots);
  const launchSelected = () => {
    if (!launchable.length) return;
    // Cada issue con el workflow que muestra su card.
    void launcher.launch(launchable, (i) => picks[i.id]);
    setSelected(new Set());
  };

  const pickWorkflow = (issueId: string, wf: string) => {
    setPicks((p) => ({ ...p, [issueId]: wf }));
    setLastWorkflow(wf);
    writeLastWorkflow(wf);
  };

  // ---- Runs de tareas y sesiones sueltas ----
  const taskOfRunKey = (key: string): Task | undefined => {
    // `t:<taskId>:<queuedAt>`
    const id = key.startsWith("t:") ? key.slice(2, key.lastIndexOf(":")) : null;
    return id ? tasks.tasks.find((t) => t.id === id) : undefined;
  };
  /** `i:`/`s:` de useRuns, `t:` de tareas, o cualquier sesión que liste `claude agents`. */
  const resolveRunView = (key: string): RunView | undefined => {
    const found = runs.views.find((v) => v.key === key);
    if (found) return found;
    const task = taskOfRunKey(key);
    if (task) return tasks.historyOf(task.id).find((v) => v.key === key);
    if (key.startsWith("s:")) {
      const sid = key.slice(2);
      const r = runs.runs.find((x) => x.sessionId === sid);
      if (r) return viewOfSession(r, runs.details[sid]);
    }
    return undefined;
  };
  /** Una sesión de la vista Activity: si es de una issue o tarea, abre ese run. */
  const openSession = (sid: string) => {
    const own =
      runs.views.find((v) => v.run?.sessionId === sid) ??
      [...tasks.current.values()].find((v) => v.run?.sessionId === sid);
    openRun(own?.key ?? sessionKey(sid));
  };

  // ---- Paleta ----
  const paletteActions: PaletteItem[] = [
    { id: "go-board", kind: "Action", label: "Go to Board", sub: "⌘1", run: () => go("board") },
    { id: "go-runs", kind: "Action", label: "Go to Runs", sub: "⌘2", run: () => go("runs") },
    { id: "go-tasks", kind: "Action", label: "Go to Tasks", sub: "⌘3", run: () => go("tasks") },
    { id: "go-activity", kind: "Action", label: "Go to Activity", sub: "⌘4", run: () => go("activity") },
    { id: "new-task", kind: "Action", label: "New task…", sub: "Local plan → /plan-task", run: () => openTaskDialog(null) },
    { id: "go-settings", kind: "Action", label: "Open Settings", sub: "⌘,", run: () => go("settings") },
    { id: "refresh", kind: "Action", label: "Refresh board", run: () => void loadBoard() },
    ...(launchable.length
      ? [
          {
            id: "launch-sel",
            kind: "Action" as const,
            label: `Launch ${launchable.length} selected issue${launchable.length === 1 ? "" : "s"}`,
            run: launchSelected,
          },
        ]
      : []),
    ...activeRuns.slice(0, 3).map((v) => ({
      id: `open-${v.key}`,
      kind: "Action" as const,
      label: `Open run ${v.identifier ?? v.runId ?? ""}`,
      sub: v.label,
      run: () => openRun(v.key),
    })),
  ];
  const paletteSearch = (pq: string): PaletteItem[] => {
    const matches = allIssues.filter((i) => !pq || `${i.identifier} ${i.title}`.toLowerCase().includes(pq));
    const runItems = pq
      ? matches
          .filter(canLaunch)
          .slice(0, 3)
          .map((i) => ({
            id: `run-${i.id}`,
            kind: "Run" as const,
            label: `Run ${i.identifier} — ${i.title}`,
            sub: launcher.workflowFor(i, picks[i.id]),
            run: () => void launcher.launch([i], picks[i.id]),
          }))
      : [];
    const issueItems = matches.slice(0, pq ? 6 : 4).map((i) => ({
      id: `issue-${i.id}`,
      kind: "Issue" as const,
      label: `${i.identifier}  ${i.title}`,
      sub: i.team.key,
      run: () => {
        setView("board");
        openIssue(i.id);
      },
    }));
    const taskItems = tasks.tasks
      .filter((t) => !pq || t.title.toLowerCase().includes(pq))
      .slice(0, pq ? 4 : 2)
      .map((t) => ({
        id: `task-${t.id}`,
        kind: "Task" as const,
        label: t.title,
        sub: basename(t.repoPath),
        run: () => {
          // La vista Tasks tiene su propio drawer: se abre desde el board para no apilar dos.
          if (view === "tasks") setView("board");
          openTask(t.id);
        },
      }));
    return [...runItems, ...issueItems, ...taskItems];
  };

  // ---- Pantallas fuera del shell ----
  if (keyError) {
    return (
      <div className="desk">
        <main className="main">
          <ErrorState error={keyError} onRetry={checkKey} />
        </main>
      </div>
    );
  }
  if (keyConfigured === null) return <div className="loading-screen">Loading…</div>;
  if (!keyConfigured) {
    return (
      <Onboarding
        onSaved={(v) => {
          setViewer(v);
          setKeyConfigured(true);
        }}
      />
    );
  }

  // ---- Contenido principal ----
  let content;
  if (view === "settings") {
    content = (
      <SettingsView
        section={section}
        onSection={setSection}
        teams={teams}
        issues={allIssues}
        config={config}
        configLoaded={configLoaded}
        configError={configError}
        viewer={viewer}
        viewerError={viewerError}
        diagnostics={{ cli, workflows: catalogs[""]?.length ?? null }}
        onRecheck={async () => {
          await Promise.all([checkCli(), loadViewer()]);
        }}
        onConfigSaved={(c) => {
          setConfig(c);
          setConfigLoaded(true);
          setConfigError(null);
        }}
        onKeySaved={(v) => {
          setViewer(v);
          setViewerError(null);
          void loadBoard();
        }}
        onKeyCleared={onKeyCleared}
      />
    );
  } else if (view === "runs") {
    content = (
      <RunsView
        runs={runs}
        issues={issueById}
        config={config}
        actions={actions}
        tab={runsTab}
        onTab={setRunsTab}
        onOpenRun={openRun}
      />
    );
  } else if (view === "tasks") {
    content = (
      <TasksView
        repoPath={tasksRepo}
        repos={mappedRepos}
        pickFile={pickFile}
        onOpenRun={openRunView}
        state={tasks}
        actions={taskActions}
        finishOf={finishOf}
      />
    );
  } else if (view === "activity") {
    const repo = activityRepo && mappedRepos.includes(activityRepo) ? activityRepo : (mappedRepos[0] ?? null);
    content = repo ? (
      <div className="activity-view">
        <RepoActivityPanel key={repo} repoPath={repo} onOpenSession={(s) => openSession(s.sessionId)} />
      </div>
    ) : (
      <EmptyState
        title="No repositories mapped"
        text="Map a repository in Settings to see what Claude Code is doing in it."
        action={{ label: "Map a repo", onClick: () => openSettings("repos") }}
      />
    );
  } else if (view === "run") {
    const rv = runKey ? resolveRunView(runKey) : undefined;
    const issue = rv?.issueId ? issueById.get(rv.issueId) : undefined;
    const task = rv && !rv.issueId ? taskOfRunKey(rv.key) : undefined;
    const taskRunAgain =
      task && task.status !== "done" && !tasks.current.get(task.id)?.active && !taskActions.pending.has(task.id)
        ? () => void taskActions.run(task)
        : undefined;
    content = rv ? (
      <RunDetailView
        key={rv.key}
        view={rv}
        issue={issue}
        backLabel={VIEW_TITLE[runFrom]}
        actions={actions}
        onBack={() => setView(runFrom)}
        onOpenIssue={openIssue}
        onRunAgain={issue && canLaunch(issue) ? () => void launcher.launch([issue], rv.workflow) : taskRunAgain}
      />
    ) : (
      <EmptyState
        title="This run is no longer available"
        text="It was removed from the history or the queue."
        action={{ label: `Back to ${VIEW_TITLE[runFrom]}`, onClick: () => setView(runFrom) }}
      />
    );
  } else if (!board && boardError) {
    content = (
      <ErrorState
        error={boardError}
        lastSync={lastSync}
        onRetry={() => void loadBoard()}
        retrying={loading}
        onSettings={() => openSettings("linear")}
      />
    );
  } else if (!board) {
    content = <BoardSkeleton />;
  } else if (visible.length === 0 && boardTasks.length === 0) {
    content =
      allIssues.length > 0 || tasks.tasks.length > 0 || hasFilters ? (
        <EmptyState
          title="Nothing matches these filters"
          text="Try another team or project, or clear filters to see everything pulled from Linear."
          action={{ label: "Clear filters", onClick: clearFilters }}
        />
      ) : (
        <EmptyState
          title="No issues"
          text="Nothing open in Linear, nothing closed in the last 14 days, and no local tasks."
          action={{ label: "New task", onClick: () => openTaskDialog(null) }}
        />
      );
  } else {
    content = (
      <BoardView
        issues={visible}
        tasks={boardTasks}
        taskHandlers={{
          isBusy: (id) => taskActions.pending.has(id),
          selectedId: panelTask,
          onOpen: (t) => openTask(t.id),
          onRun: (t) => void taskActions.run(t),
          onToggleDone: (t) => void taskActions.toggleDone(t),
          onOpenRun: openRunView,
        }}
        config={config}
        currentView={runs.currentView}
        launcher={launcher}
        picks={picks}
        onPick={pickWorkflow}
        selected={selected}
        onToggle={(id) =>
          setSelected((s) => {
            const n = new Set(s);
            if (n.has(id)) n.delete(id);
            else n.add(id);
            return n;
          })
        }
        onOpenIssue={openIssue}
        onOpenRun={openRun}
        onMapRepo={() => openSettings("repos")}
      />
    );
  }

  const panel = panelIssue ? issueById.get(panelIssue) : undefined;
  const taskPanel = panelTask ? tasks.tasks.find((t) => t.id === panelTask) : undefined;
  const navCurrent: NavView = view === "run" ? runFrom : view;
  const working = activity ? activity.sessions + activity.agents : 0;
  const activityLabel = activity
    ? `${activity.sessions} session${activity.sessions === 1 ? "" : "s"} and ${activity.agents} agent${activity.agents === 1 ? "" : "s"} working`
    : "";
  const setActivity = (v: string | null) => {
    setActivityRepo(v);
    localSet(ACTIVITY_REPO_KEY, v ?? "");
  };

  return (
    <div className="desk">
      <Sidebar
        current={navCurrent}
        counts={{
          board: board ? allIssues.length : undefined,
          runs: activeRuns.length,
          tasks: tasks.loading ? undefined : tasks.tasks.filter((t) => t.status === "todo").length,
          activity: working || undefined,
        }}
        live={working ? { activity: activityLabel } : undefined}
        activeRuns={activeRuns}
        viewer={viewer}
        linear={
          viewerError || (boardError && !board)
            ? { ok: false, label: "Linear unreachable" }
            : { ok: true, label: "Linear connected" }
        }
        onNav={go}
        onOpenRun={openRun}
        onOpenPalette={() => setPaletteOpen(true)}
      />
      <main className="main">
        <header className="topbar">
          <h1 className="topbar-title">{VIEW_TITLE[view]}</h1>
          {view === "board" && (
            <div className="topbar-filters">
              <input
                className="filter-input"
                type="search"
                placeholder="Filter issues"
                aria-label="Filter issues"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                spellCheck={false}
              />
              <FilterChip
                label="Team"
                value={teamFilter || null}
                options={teams.map((t) => ({ value: t.id, label: t.name }))}
                onChange={setTeam}
              />
              <FilterChip label="Project" value={projectFilter} options={projectOptions} onChange={setProjectFilter} />
              <FilterChip label="Assignee" value={assigneeFilter} options={assigneeOptions} onChange={setAssigneeFilter} />
              <button
                type="button"
                className={`chip ${hasRepoFilter ? "chip-on" : ""}`}
                aria-pressed={hasRepoFilter}
                onClick={() => setHasRepoFilter(!hasRepoFilter)}
              >
                Has repo
              </button>
              <FilterChip
                label="Source"
                value={sourceFilter}
                options={SOURCE_OPTIONS}
                anyLabel="All"
                onChange={(v) => setSourceFilter(v as Source | null)}
              />
              <FilterChip label="Repo" value={repoFilter} options={repoOptions} onChange={setRepoFilter} />
              {hasFilters && (
                <button type="button" className="btn btn-ghost btn-sm" onClick={clearFilters}>
                  Clear
                </button>
              )}
            </div>
          )}
          {view === "tasks" && (
            <div className="topbar-filters">
              <FilterChip label="Repo" value={tasksRepo} options={repoOptions} onChange={setTasksRepo} />
            </div>
          )}
          {view === "activity" && mappedRepos.length > 0 && (
            <div className="topbar-filters">
              <FilterChip
                label="Repo"
                value={activityRepo && mappedRepos.includes(activityRepo) ? activityRepo : (mappedRepos[0] ?? null)}
                options={repoOptions}
                anyLabel={null}
                onChange={setActivity}
              />
            </div>
          )}
          <div className="topbar-spacer" />
          {view === "board" && (
            <>
              <button type="button" className="btn topbar-btn" onClick={() => openTaskDialog(null)}>
                New task
              </button>
              <button type="button" className="btn topbar-btn" onClick={() => void loadBoard()} disabled={loading}>
                {loading ? "Syncing…" : "Refresh"}
              </button>
            </>
          )}
          <button type="button" className="runs-pill" onClick={() => go("runs")} aria-label="Open runs">
            <span className="runs-pill-running">
              <span className={`dot ${activeRuns.length ? "pulse" : ""}`} aria-hidden />
              {activeRuns.length} running
            </span>
            <span className="runs-pill-queued">{queuedCount} queued</span>
          </button>
          <button type="button" className="btn topbar-btn" onClick={() => go("settings")}>
            Settings
          </button>
        </header>

        {cli && !cli.ok && (
          <div className="banner banner-error" role="alert">
            Claude Code CLI not found — runs can't launch. {cli.text}
          </div>
        )}
        {configError && (
          <div className="banner banner-error" role="alert">
            Couldn't read the config: {configError}
          </div>
        )}
        {view === "board" && board && boardError && (
          <div className="banner banner-error" role="alert">
            Couldn't refresh from Linear: {boardError.message}
            <button type="button" className="btn btn-sm" onClick={() => void loadBoard()} disabled={loading}>
              Retry
            </button>
          </div>
        )}
        {view === "board" && board?.truncated && (
          <div className="banner banner-warn">Showing the first 2,000 issues. Filter by team to see the rest.</div>
        )}

        <div className="content">{content}</div>

        {view === "board" && tasks.error && (
          <div className="banner banner-error" role="alert">
            {tasks.error}
          </div>
        )}

        {view === "board" && selectedIssues.length > 0 && (
          <div className="selbar" role="region" aria-label="Selection">
            <span className="selbar-label">{selectedIssues.length} selected</span>
            {(skipped > 0 || willQueue > 0) && (
              <span className="selbar-note">
                {[skipped ? `${skipped} skipped (running or no repo)` : "", willQueue ? `${willQueue} will queue` : ""]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            )}
            <button type="button" className="btn btn-ghost" onClick={() => setSelected(new Set())}>
              Clear
            </button>
            <button type="button" className="btn btn-primary" disabled={!launchable.length} onClick={launchSelected}>
              Launch {launchable.length} issue{launchable.length === 1 ? "" : "s"}
            </button>
          </div>
        )}

        {panel && (
          <IssuePanel
            key={panel.id}
            issue={panel}
            repo={repoOf(panel)}
            history={runs.views.filter((v) => v.issueId === panel.id)}
            current={runs.currentView.get(panel.id)}
            catalog={launcher.catalogFor(panel)}
            defaultWorkflow={launcher.workflowFor(panel, picks[panel.id])}
            launching={launcher.pending.has(panel.id)}
            freeSlots={freeSlots}
            actions={actions}
            onLaunch={(wf) => {
              pickWorkflow(panel.id, wf);
              void launcher.launch([panel], wf);
            }}
            onOpenRun={openRun}
            onMapRepo={() => openSettings("repos")}
            onViewQueue={() => {
              setRunsTab("queued");
              go("runs");
            }}
            detailCache={detailCache}
            isOnBoard={(id) => issueById.has(id)}
            onOpenIssue={openIssue}
            onClose={() => setPanelIssue(null)}
          />
        )}
        {taskPanel && view !== "tasks" && (
          <TaskPanel
            key={taskPanel.id}
            task={taskPanel}
            current={tasks.current.get(taskPanel.id)}
            history={tasks.historyOf(taskPanel.id)}
            actions={taskActions}
            planVersion={taskPlanVersion}
            defaultFinish={finishOf(taskPanel.repoPath)}
            onClose={() => setPanelTask(null)}
            onEdit={(t) => openTaskDialog(t)}
            onOpenRun={openRunView}
          />
        )}
        {taskDialog && (
          <NewTaskDialog
            repos={mappedRepos}
            defaultRepo={taskDialog.task ? null : view === "tasks" ? tasksRepo : repoFilter}
            task={taskDialog.task}
            pickFile={pickFile}
            onClose={() => setTaskDialog(null)}
            onSaved={(t) => {
              setTaskDialog(null);
              setTaskPlanVersion((v) => v + 1);
              // En la vista Tasks el drawer propio lo muestra la lista; en el resto, el del shell.
              if (view !== "tasks") openTask(t.id);
              void tasks.refresh();
            }}
          />
        )}
      </main>

      {paletteOpen && (
        <CommandPalette actions={paletteActions} search={paletteSearch} onClose={() => setPaletteOpen(false)} />
      )}
    </div>
  );
}
