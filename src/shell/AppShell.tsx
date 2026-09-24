import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { launchTask } from "../domain/api";
import { useProjects } from "../domain/hooks/projects";
import { useProviderStatus, useSourceLinks } from "../domain/hooks/providers";
import { useQueueSummary } from "../domain/hooks/runs";
import { invalidate } from "../domain/hooks/store";
import { taskKey, type Project } from "../domain/types";
import { BoardView } from "../features/board";
import { Onboarding } from "../features/onboarding/Onboarding";
import { CreateProjectDialog } from "../features/projects/CreateProjectDialog";
import { ProjectSettings } from "../features/projects/ProjectSettings";
import { ImportDialog } from "../features/providers";
import { ActivityView, RunDetailView, RunDiffDrawer, RunsView } from "../features/runs";
import { useLegacyImport } from "../features/settings/legacyImport";
import { SettingsView } from "../features/settings/SettingsView";
import { NewTaskDialog, TaskPanel, TasksView } from "../features/tasks";
import { useToast } from "../ui/Toasts";
import { CommandPalette, type PaletteItem } from "./palette/CommandPalette";
import { Sidebar, type ProviderFoot } from "./Sidebar";
import { Topbar } from "./Topbar";
import { PAGE_TITLE, useNav, type Page, type ProjectPage } from "./useNav";
import { useWorkStatus } from "./useWorkStatus";
import "./shell.css";

const isEditable = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));

/** ⌘1–4, como en el diseño. */
const SHORTCUT_PAGES: Record<string, ProjectPage> = { "1": "board", "2": "tasks", "3": "runs", "4": "activity" };
const PROJECT_PAGES: ReadonlySet<Page> = new Set(["board", "tasks", "runs", "activity", "project-settings"]);

export function AppShell() {
  const ctx = useProjects();
  const provider = useProviderStatus("linear");
  const nav = useNav();
  const work = useWorkStatus();
  const queue = useQueueSummary();
  const toast = useToast();
  const legacy = useLegacyImport();

  const [forceOnboarding, setForceOnboarding] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [createProject, setCreateProject] = useState(false);
  const [newTask, setNewTask] = useState<{ projectId: string | null } | null>(null);
  const [importFor, setImportFor] = useState<string | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [diffRunId, setDiffRunId] = useState<string | null>(null);

  const { route } = nav;
  const project = route.projectId ? (ctx.projectById.get(route.projectId) ?? null) : null;
  // En un run, el sidebar y el breadcrumb muestran de dónde vino.
  const visible = route.page === "run" ? nav.runFrom : route;
  const projectRepos = project ? ctx.reposOf(project.id) : [];

  // Proyecto borrado (desde acá o desde otra parte): volver a "All projects".
  const { forgetProject } = nav;
  useEffect(() => {
    if (ctx.loaded && !ctx.error && route.projectId && !ctx.projectById.has(route.projectId)) {
      forgetProject(route.projectId);
    }
  }, [ctx.loaded, ctx.error, ctx.projectById, route.projectId, forgetProject]);

  // Último proyecto visitado: destino de ⌘2/⌘4 desde una vista global.
  const lastProjectId = useRef<string | null>(null);
  useEffect(() => {
    if (route.projectId) lastProjectId.current = route.projectId;
  }, [route.projectId]);

  // Fuentes del proyecto actual con Linear conectado: habilitan "Import".
  const projectId = project?.id ?? null;
  const links = useSourceLinks(projectId);
  const canImport = projectId !== null && provider.connection === "connected" && (links.data?.length ?? 0) > 0;

  const overlayOpen = paletteOpen || createProject || newTask !== null || importFor !== null || diffRunId !== null;

  const openTask = useCallback((id: string) => {
    setPaletteOpen(false);
    setTaskId(id);
  }, []);
  const { openRun: navOpenRun, go: navGo } = nav;
  const openRun = useCallback(
    (id: string) => {
      setTaskId(null);
      setDiffRunId(null);
      setPaletteOpen(false);
      navOpenRun(id);
    },
    [navOpenRun],
  );
  const go = useCallback(
    (page: Page, pid: string | null = null) => {
      setTaskId(null);
      setDiffRunId(null);
      setPaletteOpen(false);
      navGo(page, pid);
    },
    [navGo],
  );
  const openNewTask = useCallback(() => {
    setPaletteOpen(false);
    setNewTask({ projectId: project?.id ?? null });
  }, [project]);

  const { setProjectSection, setSettingsSection } = nav;
  const openRepoSettings = useCallback(
    (pid: string) => {
      setProjectSection("repos");
      go("project-settings", pid);
    },
    [setProjectSection, go],
  );
  const openIntegrations = useCallback(() => {
    setSettingsSection("integrations");
    go("settings");
  }, [setSettingsSection, go]);

  /** Proyecto para ⌘2/⌘4 desde una vista global: el actual, el último visitado o el primero. */
  const fallbackProject = (): Project | null =>
    project ??
    (lastProjectId.current ? ctx.projectById.get(lastProjectId.current) : undefined) ??
    ctx.projects[0] ??
    null;

  const showOnboarding = ctx.loaded && (ctx.projects.length === 0 || forceOnboarding) && !(ctx.error && ctx.projects.length === 0);

  // El handler cambia en cada render; el listener se registra una vez y llama al último.
  const onKeyRef = useRef<(e: KeyboardEvent) => void>(() => {});
  useLayoutEffect(() => {
    onKeyRef.current = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      // Fuera del shell (carga, error, onboarding) no hay atajos.
      if (!ctx.loaded || showOnboarding || (ctx.error && ctx.projects.length === 0)) return;
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (createProject || newTask || importFor || diffRunId) return;
        setPaletteOpen((o) => !o);
        return;
      }
      if (mod && !e.shiftKey && !e.altKey && (e.key in SHORTCUT_PAGES || e.key === ",")) {
        e.preventDefault();
        if (overlayOpen) return;
        if (e.key === ",") {
          go("settings");
          return;
        }
        const page = SHORTCUT_PAGES[e.key]!;
        const needsProject = page === "tasks" || page === "activity";
        const pid = needsProject ? (fallbackProject()?.id ?? null) : (visible.projectId ?? null);
        if (needsProject && !pid) return;
        go(page, pid);
        return;
      }
      // Esc en cascada: la paleta y los diálogos lo atrapan antes (useFocusTrap); acá
      // quedan el drawer de la tarea y el detalle del run.
      if (e.key === "Escape") {
        if (taskId) {
          e.preventDefault();
          setTaskId(null);
        } else if (route.page === "run" && !isEditable(e.target)) {
          e.preventDefault();
          nav.back();
        }
      }
    };
  });
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => onKeyRef.current(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // ---- Paleta ----
  const paletteActions = useMemo((): PaletteItem[] => {
    const items: PaletteItem[] = [];
    const canNewTask = project ? projectRepos.length > 0 : ctx.repos.length > 0;
    if (canNewTask) items.push({ id: "new-task", kind: "Action", label: "New task", run: openNewTask });
    items.push({ id: "new-project", kind: "Action", label: "New project", run: () => setCreateProject(true) });
    if (project && canImport) {
      items.push({ id: "import", kind: "Action", label: "Import from Linear", run: () => setImportFor(project.id) });
    }
    items.push({ id: "go-runs", kind: "Action", label: "Go to Runs", run: () => go("runs") });
    items.push({ id: "settings", kind: "Action", label: "Open Settings", sub: "⌘,", run: () => go("settings") });
    items.push({ id: "onboarding", kind: "Action", label: "Show onboarding", run: () => setForceOnboarding(true) });
    for (const p of ctx.projects) {
      items.push({ id: `open-${p.id}`, kind: "Action", label: `Open ${p.name}`, keywords: p.key, run: () => go("board", p.id) });
    }
    if (queue.needYou > 0 || work.blocked.length > 0) {
      items.push({
        id: "need-you",
        kind: "Action",
        label: `Review what needs you (${queue.needYou + work.blocked.length})`,
        run: () => (work.blocked[0] ? openTask(work.blocked[0].id) : go("runs")),
      });
    }
    return items;
  }, [project, projectRepos.length, ctx.repos.length, ctx.projects, canImport, queue.needYou, work.blocked, openNewTask, go, openTask]);

  const paletteSearch = (q: string): PaletteItem[] => {
    const keyOf = (pid: string, n: number) => taskKey(ctx.projectById.get(pid)?.key ?? "", n);
    const pool = work.tasks.filter((t) => {
      if (!q) return true;
      const hay = `${keyOf(t.projectId, t.number)} ${t.title} ${t.source?.identifier ?? ""}`.toLowerCase();
      return hay.includes(q);
    });
    const runItems: PaletteItem[] = q
      ? pool
          .filter((t) => t.status !== "done" && t.status !== "canceled" && !work.activeTaskIds.has(t.id))
          .slice(0, 3)
          .map((t) => ({
            id: `run-${t.id}`,
            kind: "Run",
            label: `Run ${keyOf(t.projectId, t.number)} · ${t.title}`,
            run: () => {
              launchTask(t.id).then(
                (r) => {
                  toast(r.status === "queued" ? "Queued" : "Launching", `${keyOf(t.projectId, t.number)} · ${t.title}`, "accent");
                  void invalidate("runs", "tasks");
                },
                (err) => toast("Couldn't launch", String(err), "danger"),
              );
            },
          }))
      : [];
    const taskItems: PaletteItem[] = pool.slice(0, q ? 6 : 4).map((t) => ({
      id: `task-${t.id}`,
      kind: "Task",
      label: `${keyOf(t.projectId, t.number)} ${t.title}`,
      sub: ctx.projectById.get(t.projectId)?.name,
      run: () => openTask(t.id),
    }));
    return [...runItems, ...taskItems];
  };

  // ---- Pantallas fuera del shell ----
  if (!ctx.loaded) return <div className="loading-screen" aria-busy="true">Loading…</div>;
  if (ctx.error && ctx.projects.length === 0) {
    return (
      <div className="loading-screen">
        <div className="center-state-body">
          <div className="state-icon-error" aria-hidden>
            !
          </div>
          <div className="center-state-title">Couldn't load your projects</div>
          <div className="center-state-text">{ctx.error}</div>
          <button type="button" className="btn" onClick={() => void ctx.refresh()}>
            Retry
          </button>
        </div>
      </div>
    );
  }
  if (showOnboarding) {
    return (
      <Onboarding
        onDone={(p) => {
          setForceOnboarding(false);
          go("board", p.id);
        }}
        onCancel={ctx.projects.length > 0 ? () => setForceOnboarding(false) : undefined}
        importingLegacy={legacy.busy}
        onImportLegacy={() =>
          void legacy.run().then((r) => {
            if (r && r.projects > 0) setForceOnboarding(false);
          })
        }
      />
    );
  }

  // ---- Contenido ----
  // El board resuelve su propio estado "sin repos"; Tasks lo delega acá.
  const noRepos = project !== null && projectRepos.length === 0 && route.page === "tasks";
  let content: ReactNode;
  if (noRepos && project) {
    content = (
      <div className="center-state">
        <div className="center-state-body">
          <div className="state-icon-empty" aria-hidden />
          <div className="center-state-title">No repos in {project.name}</div>
          <div className="center-state-text">Every task runs in one repo. Add the root of a git checkout to start.</div>
          <div className="center-state-actions">
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => {
                openRepoSettings(project.id);
              }}
            >
              Add repo…
            </button>
          </div>
        </div>
      </div>
    );
  } else {
    switch (route.page) {
      case "board":
        content = (
          <BoardView
            projectId={route.projectId}
            onOpenTask={openTask}
            onOpenRun={openRun}
            onNewProject={() => setCreateProject(true)}
            onOpenProjectSettings={openRepoSettings}
          />
        );
        break;
      case "tasks":
        content = project && <TasksView projectId={project.id} onOpenTask={openTask} />;
        break;
      case "runs":
        content = <RunsView projectId={route.projectId} onOpenRun={openRun} />;
        break;
      case "activity":
        content = project && <ActivityView projectId={project.id} onOpenRun={openRun} />;
        break;
      case "project-settings":
        content = project && (
          <ProjectSettings
            project={project}
            section={nav.projectSection}
            onSection={nav.setProjectSection}
            onDeleted={() => go("board")}
            onOpenIntegrations={openIntegrations}
          />
        );
        break;
      case "settings":
        content = (
<SettingsView section={nav.settingsSection} onSection={nav.setSettingsSection} />
        );
        break;
      case "run":
        content = route.runId ? (
          <RunDetailView key={route.runId} runId={route.runId} onBack={nav.back} onOpenTask={openTask} onOpenRun={openRun} />
        ) : null;
        break;
    }
  }

  const pageTitle =
    route.page === "run" ? PAGE_TITLE.run : visible.page === "board" && !visible.projectId ? "All projects" : PAGE_TITLE[visible.page];
  const crumbProject = visible.projectId ? (ctx.projectById.get(visible.projectId) ?? null) : null;
  const showNewTask =
    (route.page === "board" || route.page === "tasks") && (project ? projectRepos.length > 0 : ctx.repos.length > 0);

  const foot: ProviderFoot =
    provider.connection === "connected"
      ? { tone: "ok", label: "Linear connected" }
      : provider.status?.hasKey
        ? { tone: "danger", label: "Linear unreachable" }
        : { tone: "muted", label: "No task manager" };

  return (
    <div className="desk">
      <Sidebar
        current={visible}
        projects={ctx.projects}
        expanded={nav.expanded}
        openTotal={work.openTotal}
        activeTotal={work.activeTotal}
        activeByProject={work.activeByProject}
        provider={foot}
        onGo={(page, pid) => go(page, pid)}
        onToggleProject={(pid) => {
          // Como en el diseño: elegir otro proyecto lo abre en la misma página; el actual se pliega.
          if (visible.projectId === pid) {
            nav.toggleProject(pid);
          } else {
            const page = PROJECT_PAGES.has(visible.page) && visible.projectId ? visible.page : "board";
            go(page, pid);
          }
        }}
        onNewProject={() => setCreateProject(true)}
        onOpenPalette={() => setPaletteOpen(true)}
      />
      <div className="main-wrap">
        <main className="main">
          <Topbar
            project={route.page === "run" ? crumbProject : project}
            page={pageTitle}
            running={queue.running}
            concurrency={queue.capacity || null}
            needYou={queue.needYou}
            queued={queue.queued}
            showImport={route.page === "board" && canImport}
            showNewTask={showNewTask}
            onOpenRuns={() => go("runs")}
            onImport={() => project && setImportFor(project.id)}
            onNewTask={openNewTask}
          />
          {work.error && (
            <div className="banner banner-error" role="alert">
              Couldn't refresh runs and tasks: {work.error}
            </div>
          )}
          <div className="content">{content}</div>

          {taskId && (
            <TaskPanel
              key={taskId}
              taskId={taskId}
              onClose={() => setTaskId(null)}
              onOpenRun={openRun}
              onOpenDiff={setDiffRunId}
              onOpenTask={openTask}
            />
          )}
        </main>
      </div>

      {newTask && (
        <NewTaskDialog
          projectId={newTask.projectId}
          onClose={() => setNewTask(null)}
          onSaved={(t) => {
            setNewTask(null);
            openTask(t.id);
          }}
          onAddRepo={(pid) => {
            setNewTask(null);
            openRepoSettings(pid);
          }}
        />
      )}
      {importFor && (
        <ImportDialog
          projectId={importFor}
          projectName={ctx.projectById.get(importFor)?.name}
          onClose={() => setImportFor(null)}
          onImported={() => void invalidate("tasks", "runs")}
          onOpenSources={() => {
            const pid = importFor;
            setImportFor(null);
            nav.setProjectSection("sources");
            go("project-settings", pid);
          }}
        />
      )}
      {diffRunId && <RunDiffDrawer key={diffRunId} runId={diffRunId} onClose={() => setDiffRunId(null)} />}
      {createProject && (
        <CreateProjectDialog
          onClose={() => setCreateProject(false)}
          onCreated={(p) => {
            setCreateProject(false);
            go("board", p.id);
          }}
        />
      )}
      {paletteOpen && (
        <CommandPalette actions={paletteActions} search={paletteSearch} onClose={() => setPaletteOpen(false)} />
      )}
    </div>
  );
}
