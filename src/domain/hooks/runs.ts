// Store compartido de runs: UN solo polling para toda la app. Cada vista se suscribe con
// `useRuns`/`useRun`/`useQueueSummary`; el polling corre mientras haya al menos un
// suscriptor y se detiene con el último. Las mutaciones llaman a `refreshRuns()`.
//
// En cada vuelta se leen de la base los runs (`list_task_runs`), proyectos, repos, tareas y
// settings, y de Claude Code las sesiones en background (`claude agents`). El detalle del
// workflow (`get_run_detail`) se pide solo para runs de workflow: los activos en cada
// vuelta, los terminados hasta que se asienta (y después nunca más).

import { useEffect, useMemo, useSyncExternalStore } from "react";
import { getSettings, listProjects, listRepos, listTaskRuns, listTasks } from "../api";
import type { Project, Repo, Run, Settings, Task } from "../types";
import { getRunDetail, listRuns } from "../../features/runs/api";
import { deriveRunView, LAUNCH_GRACE_MS, type RunView } from "../../features/runs/status";
import { isInProgress, type RunDetail, type RunSummary } from "../../features/runs/types";

const POLL_MS = 3000;
/** Polls que se sigue pidiendo el detalle de un run terminado sin resumen final. */
const SETTLE_TRIES = 3;
/** Runs terminados cuyo detalle se pide sin que nadie lo pida explícitamente. */
const MAX_HISTORY_DETAILS = 40;

export interface RunsSnapshot {
  runs: Run[];
  live: RunSummary[];
  details: Record<string, RunDetail | null>;
  projects: Project[];
  repos: Repo[];
  tasks: Task[];
  settings: Settings | null;
  /** Errores de la última vuelta (en inglés, listos para mostrar). */
  error: string | null;
  /** La base respondió al menos una vez (`runs` vacío ya significa "no hay"). */
  loaded: boolean;
  /** `claude agents` respondió al menos una vez. */
  liveLoaded: boolean;
  now: number;
}

const EMPTY: RunsSnapshot = {
  runs: [],
  live: [],
  details: {},
  projects: [],
  repos: [],
  tasks: [],
  settings: null,
  error: null,
  loaded: false,
  liveLoaded: false,
  now: Date.now(),
};

let snapshot: RunsSnapshot = EMPTY;
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setTimeout> | null = null;
/** Hay un bucle de polling vivo (esperando el timer o con una vuelta en curso). */
let looping = false;
let inFlight: Promise<void> | null = null;
let again = false;
/** Detalles que ya no cambian: no se vuelven a pedir. */
const settled = new Set<string>();
const settleTries = new Map<string, number>();
/** Runs cuyo detalle pidió una vista (p. ej. RunDetailView de un run viejo). */
const wanted = new Set<string>();

function emit(next: Partial<RunsSnapshot>) {
  snapshot = { ...snapshot, ...next };
  for (const l of listeners) l();
}

function needsDetail(run: Run, live: RunSummary | null, historyIds: Set<string>): boolean {
  if (run.executor.kind !== "workflow" || !run.sessionId) return false;
  if (run.status === "launched" || isInProgress(live)) return true;
  if (settled.has(run.id)) return false;
  return historyIds.has(run.id) || wanted.has(run.id);
}

async function pollOnce() {
  const [runsR, liveR, projR, repoR, taskR, setR] = await Promise.allSettled([
    listTaskRuns(null),
    listRuns(),
    listProjects(),
    listRepos(null),
    listTasks(null),
    getSettings(),
  ]);
  const errors: string[] = [];
  const next: Partial<RunsSnapshot> = { now: Date.now() };
  if (runsR.status === "fulfilled") {
    next.runs = runsR.value;
    next.loaded = true;
  } else errors.push(`Couldn't list runs: ${String(runsR.reason)}`);
  if (liveR.status === "fulfilled") {
    next.live = liveR.value;
    next.liveLoaded = true;
  } else errors.push(`Couldn't read Claude sessions: ${String(liveR.reason)}`);
  if (projR.status === "fulfilled") next.projects = projR.value;
  if (repoR.status === "fulfilled") next.repos = repoR.value;
  if (taskR.status === "fulfilled") next.tasks = taskR.value;
  if (setR.status === "fulfilled") next.settings = setR.value;
  next.error = errors.length ? errors.join("\n") : null;
  emit(next);

  const runs = snapshot.runs;
  const live = snapshot.live;
  const historyIds = new Set(
    runs
      .filter((r) => r.executor.kind === "workflow" && r.sessionId && r.status !== "launched")
      .slice(0, MAX_HISTORY_DETAILS)
      .map((r) => r.id),
  );
  const pending = runs.filter((r) => {
    const l = live.find((s) => s.id === r.claudeRunId || s.sessionId === r.sessionId) ?? null;
    return needsDetail(r, l, historyIds);
  });
  if (!pending.length) return;
  const fetched = await Promise.all(
    pending.map(async (r) => {
      try {
        const d = await getRunDetail(r.sessionId!, r.cwd);
        if (r.status !== "launched") {
          // Al terminar, el resumen final puede tardar en aparecer: se reintenta un par de veces.
          const n = (settleTries.get(r.id) ?? 0) + 1;
          settleTries.set(r.id, n);
          if (d === null || d.source === "final" || n >= SETTLE_TRIES) settled.add(r.id);
        }
        return [r.id, d] as const;
      } catch {
        return null; // se reintenta en la próxima vuelta
      }
    }),
  );
  const updates = fetched.filter((x) => x !== null);
  if (updates.length) emit({ details: { ...snapshot.details, ...Object.fromEntries(updates) } });
}

/** Vuelve a leer todo ya (sin solapar: si hay una vuelta en curso, se repite al terminar). */
export function refreshRuns(): Promise<void> {
  if (inFlight) {
    again = true;
    return inFlight;
  }
  const p = (async () => {
    try {
      do {
        again = false;
        await pollOnce();
      } while (again);
    } finally {
      inFlight = null;
    }
  })();
  inFlight = p;
  return p;
}

function loop() {
  timer = null;
  void refreshRuns().finally(() => {
    if (listeners.size) timer = setTimeout(loop, POLL_MS);
    else looping = false;
  });
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  // Un solo bucle: si el último suscriptor se fue con una vuelta en curso y llega otro
  // antes de que termine, el mismo bucle sigue (no se arranca un segundo).
  if (!looping) {
    looping = true;
    loop();
  }
  return () => {
    listeners.delete(listener);
    if (!listeners.size && timer !== null) {
      clearTimeout(timer);
      timer = null;
      looping = false;
    }
  };
}

const getSnapshot = () => snapshot;

/** Snapshot crudo del store (se re-renderiza en cada vuelta del polling). */
export function useRunsSnapshot(): RunsSnapshot {
  return useSyncExternalStore(subscribe, getSnapshot);
}

/** Proyecto de un run: por su tarea o, si no tiene, por su repo. */
export function projectIdOf(run: Run, tasks: Map<string, Task>, repos: Map<string, Repo>): string | null {
  const t = run.taskId ? tasks.get(run.taskId) : undefined;
  if (t) return t.projectId;
  const r = run.repoId ? repos.get(run.repoId) : undefined;
  return r?.projectId ?? null;
}

export interface RunsFilter {
  /** `null`/ausente: todos los proyectos. */
  projectId?: string | null;
  taskId?: string | null;
}

export interface RunsState {
  /** Runs filtrados, más recientes primero. */
  views: RunView[];
  /** Todos los runs (sin filtrar), por id. */
  byId: Map<string, RunView>;
  /** Run de trabajo más reciente de cada tarea (sin filtrar). */
  latestByTask: Map<string, RunView>;
  /** Cola global (sin filtrar), en orden de salida. */
  queue: RunView[];
  projects: Map<string, Project>;
  repos: Map<string, Repo>;
  tasks: Map<string, Task>;
  settings: Settings | null;
  error: string | null;
  loaded: boolean;
  liveLoaded: boolean;
  now: number;
  refresh: () => Promise<void>;
}

interface Derived {
  projects: Map<string, Project>;
  repos: Map<string, Repo>;
  tasks: Map<string, Task>;
  all: RunView[];
  byId: Map<string, RunView>;
  latestByTask: Map<string, RunView>;
  queue: RunView[];
}

// Derivado una vez por snapshot, compartido por todos los suscriptores.
let derivedFor: RunsSnapshot | null = null;
let derived: Derived | null = null;

function derive(s: RunsSnapshot): Derived {
  if (derivedFor === s && derived) return derived;
  derived = (() => {
    const projects = new Map(s.projects.map((p) => [p.id, p]));
    const repos = new Map(s.repos.map((r) => [r.id, r]));
    const tasks = new Map(s.tasks.map((t) => [t.id, t]));
    const queued = s.runs
      .filter((r) => r.status === "queued")
      .sort((a, b) => a.queuePosition - b.queuePosition || a.queuedAt - b.queuedAt || a.id.localeCompare(b.id));
    const queuePos = new Map(queued.map((r, i) => [r.id, i + 1]));
    const reviews = new Map<string, Run>();
    for (const r of s.runs) {
      if (r.kind !== "review" || !r.parentRunId) continue;
      const prev = reviews.get(r.parentRunId);
      if (!prev || r.queuedAt > prev.queuedAt) reviews.set(r.parentRunId, r);
    }
    const ctx = { live: s.live, details: s.details, queuePos, reviews, now: s.now };
    const all = s.runs.map((r) => deriveRunView(r, ctx));
    const byId = new Map(all.map((v) => [v.run.id, v]));
    const latestByTask = new Map<string, RunView>();
    for (const v of all) {
      if (!v.run.taskId || v.run.kind !== "work") continue;
      const prev = latestByTask.get(v.run.taskId);
      if (!prev || v.run.queuedAt > prev.run.queuedAt) latestByTask.set(v.run.taskId, v);
    }
    const queue = queued.map((r) => byId.get(r.id)!);
    return { projects, repos, tasks, all, byId, latestByTask, queue };
  })();
  derivedFor = s;
  return derived;
}

/**
 * Runs con su estado derivado. Todas las vistas comparten el mismo polling, así que se
 * puede llamar desde tantos componentes como haga falta.
 */
export function useRuns(filter: RunsFilter = {}): RunsState {
  const s = useRunsSnapshot();
  const d = derive(s);
  const { projectId = null, taskId = null } = filter;
  const views = useMemo(
    () =>
      d.all.filter((v) => {
        if (taskId && v.run.taskId !== taskId) return false;
        if (projectId && projectIdOf(v.run, d.tasks, d.repos) !== projectId) return false;
        return true;
      }),
    [d, projectId, taskId],
  );
  return {
    views,
    byId: d.byId,
    latestByTask: d.latestByTask,
    queue: d.queue,
    projects: d.projects,
    repos: d.repos,
    tasks: d.tasks,
    settings: s.settings,
    error: s.error,
    loaded: s.loaded,
    liveLoaded: s.liveLoaded,
    now: s.now,
    refresh: refreshRuns,
  };
}

/** Un run por id (con su detalle de workflow pedido aunque sea viejo). */
export function useRun(runId: string | null): { view: RunView | undefined; state: RunsState } {
  const state = useRuns();
  useEffect(() => {
    if (!runId || wanted.has(runId)) return;
    wanted.add(runId);
    void refreshRuns();
  }, [runId]);
  return { view: runId ? state.byId.get(runId) : undefined, state };
}

export interface QueueSummary {
  /** Slots ocupados: sesiones en background `working` más las nuestras que están arrancando. */
  running: number;
  /** Concurrencia global (Settings). */
  capacity: number;
  /** Runs de Nodal esperando al usuario (permiso, input) o por confirmar. */
  needYou: number;
  /** Runs en cola (incluye los que esperan confirmación). */
  queued: number;
  /** "2/4 running · 1 need you · 3 queued" (partes vacías omitidas). */
  label: string;
}

/**
 * Resumen para la píldora global. `running` sigue el mismo criterio de slots que la cola
 * del backend (`work/queue.rs::occupied_slots`).
 */
export function useQueueSummary(): QueueSummary {
  const s = useRunsSnapshot();
  return useMemo(() => {
    const working = s.live.filter((l) => l.state === "working").length;
    let starting = 0;
    let needYou = 0;
    let queued = 0;
    for (const r of s.runs) {
      if (r.status === "queued") {
        queued++;
        if (r.legacyLabel !== null) needYou++;
        continue;
      }
      if (r.status === "launching") {
        starting++;
        continue;
      }
      if (r.status !== "launched") continue;
      const live = s.live.find((l) => l.id === r.claudeRunId || l.sessionId === r.sessionId);
      if (!live) {
        if (r.launchedAt != null && s.now - r.launchedAt < LAUNCH_GRACE_MS) starting++;
      } else if (live.state === "blocked") needYou++;
    }
    const running = working + starting;
    const capacity = s.settings?.concurrency ?? 0;
    const parts = [`${running}/${capacity || "?"} running`];
    if (needYou) parts.push(`${needYou} need you`);
    if (queued) parts.push(`${queued} queued`);
    return { running, capacity, needYou, queued, label: parts.join(" · ") };
  }, [s]);
}
