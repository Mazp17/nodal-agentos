import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
import { BoardSkeleton, BoardView, EmptyState, ErrorState } from "./features/linear/Board";
import { IssuePanel } from "./features/linear/IssuePanel";
import { Onboarding, SettingsView, type SettingsSection } from "./features/linear/Settings";
import { CommandPalette, type PaletteItem } from "./features/palette/CommandPalette";
import { useLauncher, useRunActions } from "./features/runs/actions";
import { RunDetailView } from "./features/runs/RunDetailView";
import { RunsView, type RunsTab } from "./features/runs/RunsView";
import { useRuns } from "./features/runs/useRuns";
import { readLastWorkflow, useWorkflows, writeLastWorkflow } from "./features/runs/useWorkflows";
import { localGet, localSet } from "./lib/format";
import { Sidebar, type NavView } from "./shell/Sidebar";
import { FilterChip } from "./ui/FilterChip";
import { ToastProvider } from "./ui/Toasts";
import "./shell/shell.css";

const TEAM_FILTER_KEY = "agent-desk.teamFilter";
const DEFAULT_CONFIG: AppConfig = { repos: [], concurrency: 3 };
const UNASSIGNED = "__unassigned";

type View = NavView | "run";

const VIEW_TITLE: Record<View, string> = { board: "Board", runs: "Runs", settings: "Settings", run: "Run" };

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
  const [runFrom, setRunFrom] = useState<"board" | "runs">("board");
  const [runsTab, setRunsTab] = useState<RunsTab>("active");
  const [section, setSection] = useState<SettingsSection>("linear");
  const [panelIssue, setPanelIssue] = useState<string | null>(null);
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
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [picks, setPicks] = useState<Record<string, string>>({});
  const [lastWorkflow, setLastWorkflow] = useState(readLastWorkflow);

  const runs = useRuns(keyConfigured === true);
  const catalogs = useWorkflows(config.repos.map((r) => r.path));
  const launcher = useLauncher(config, catalogs, lastWorkflow, runs);
  const actions = useRunActions(runs);

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
    setKeyConfigured(false);
  }

  // ---- Navegación ----
  const go = useCallback((v: NavView) => {
    setView(v);
    setPanelIssue(null);
    setPaletteOpen(false);
  }, []);
  const openRun = useCallback(
    (key: string) => {
      setRunFrom((prev) => (view === "run" ? prev : view === "runs" ? "runs" : "board"));
      setRunKey(key);
      setView("run");
      setPanelIssue(null);
    },
    [view],
  );
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
      } else if (e.metaKey && (e.key === "1" || e.key === "2" || e.key === ",")) {
        e.preventDefault();
        go(e.key === "1" ? "board" : e.key === "2" ? "runs" : "settings");
      } else if (e.key === "Escape" && view === "run" && !panelIssue && !paletteOpen) {
        setView(runFrom);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [keyConfigured, go, view, panelIssue, paletteOpen, runFrom]);

  // ---- Board: filtros y selección ----
  const allIssues = useMemo(() => board?.issues ?? [], [board]);
  const issueById = useMemo(() => new Map(allIssues.map((i) => [i.id, i])), [allIssues]);
  const repoOf = useCallback((i: Issue) => resolveRepo(config, i.team.id, i.project?.id), [config]);
  const q = query.trim().toLowerCase();
  const visible = useMemo(
    () =>
      allIssues.filter(
        (i) =>
          (!projectFilter || i.project?.id === projectFilter) &&
          (!assigneeFilter || (assigneeFilter === UNASSIGNED ? !i.assignee : i.assignee?.id === assigneeFilter)) &&
          (!hasRepoFilter || repoOf(i) !== null) &&
          (!q || `${i.identifier} ${i.title}`.toLowerCase().includes(q)),
      ),
    [allIssues, projectFilter, assigneeFilter, hasRepoFilter, q, repoOf],
  );
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
  const hasFilters = !!(q || projectFilter || assigneeFilter || hasRepoFilter || teamFilter);
  const setTeam = (v: string | null) => {
    setTeamFilter(v ?? "");
    localSet(TEAM_FILTER_KEY, v ?? "");
  };
  const clearFilters = () => {
    setQuery("");
    setProjectFilter(null);
    setAssigneeFilter(null);
    setHasRepoFilter(false);
    setTeam(null);
  };

  const activeRuns = runs.views.filter((v) => v.kind === "running" || v.kind === "starting");
  const queuedCount = runs.views.filter((v) => v.kind === "queued").length;
  const freeSlots = queuedCount > 0 ? 0 : Math.max(0, config.concurrency - activeRuns.length);

  const canLaunch = (i: Issue) => repoOf(i) !== null && !runs.currentView.get(i.id)?.active;
  const selectedIssues = [...selected].map((id) => issueById.get(id)).filter((i): i is Issue => i !== undefined);
  const launchable = selectedIssues.filter(canLaunch);
  const skipped = selectedIssues.length - launchable.length;
  const willQueue = Math.max(0, launchable.length - freeSlots);
  const launchSelected = () => {
    if (!launchable.length) return;
    void launcher.launch(launchable);
    setSelected(new Set());
  };

  const pickWorkflow = (issueId: string, wf: string) => {
    setPicks((p) => ({ ...p, [issueId]: wf }));
    setLastWorkflow(wf);
    writeLastWorkflow(wf);
  };

  // ---- Paleta ----
  const paletteActions: PaletteItem[] = [
    { id: "go-board", kind: "Action", label: "Go to Board", sub: "⌘1", run: () => go("board") },
    { id: "go-runs", kind: "Action", label: "Go to Runs", sub: "⌘2", run: () => go("runs") },
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
        setPanelIssue(i.id);
      },
    }));
    return [...runItems, ...issueItems];
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
  } else if (view === "run") {
    const rv = runs.views.find((v) => v.key === runKey);
    const issue = rv?.issueId ? issueById.get(rv.issueId) : undefined;
    content = rv ? (
      <RunDetailView
        key={rv.key}
        view={rv}
        issue={issue}
        backLabel={runFrom === "runs" ? "Runs" : "Board"}
        actions={actions}
        onBack={() => setView(runFrom)}
        onOpenIssue={setPanelIssue}
        onRunAgain={issue && canLaunch(issue) ? () => void launcher.launch([issue], rv.workflow) : undefined}
      />
    ) : (
      <EmptyState
        title="This run is no longer available"
        text="It was removed from the history or the queue."
        action={{ label: `Back to ${runFrom === "runs" ? "Runs" : "Board"}`, onClick: () => setView(runFrom) }}
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
  } else if (visible.length === 0) {
    content =
      allIssues.length > 0 || hasFilters ? (
        <EmptyState
          title="No issues match these filters"
          text="Try another team or project, or clear filters to see everything pulled from Linear."
          action={{ label: "Clear filters", onClick: clearFilters }}
        />
      ) : (
        <EmptyState
          title="No issues"
          text="Nothing open, and nothing closed in the last 14 days."
          action={{ label: "Refresh", onClick: () => void loadBoard() }}
        />
      );
  } else {
    content = (
      <BoardView
        issues={visible}
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
        onOpenIssue={setPanelIssue}
        onOpenRun={openRun}
        onMapRepo={() => openSettings("repos")}
      />
    );
  }

  const panel = panelIssue ? issueById.get(panelIssue) : undefined;
  const navCurrent: NavView = view === "run" ? runFrom : view;

  return (
    <div className="desk">
      <Sidebar
        current={navCurrent}
        counts={{ board: board ? allIssues.length : undefined, runs: activeRuns.length }}
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
              {hasFilters && (
                <button type="button" className="btn btn-ghost btn-sm" onClick={clearFilters}>
                  Clear
                </button>
              )}
            </div>
          )}
          <div className="topbar-spacer" />
          {view === "board" && (
            <button type="button" className="btn topbar-btn" onClick={() => void loadBoard()} disabled={loading}>
              {loading ? "Syncing…" : "Refresh"}
            </button>
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

        {view === "board" && selected.size > 0 && (
          <div className="selbar" role="region" aria-label="Selection">
            <span className="selbar-label">{selected.size} selected</span>
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
            onClose={() => setPanelIssue(null)}
          />
        )}
      </main>

      {paletteOpen && (
        <CommandPalette actions={paletteActions} search={paletteSearch} onClose={() => setPaletteOpen(false)} />
      )}
    </div>
  );
}
