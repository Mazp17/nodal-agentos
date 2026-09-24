// Recurso "runs" de la capa de datos (ver `store.ts`): UN solo polling para toda la app.
// Cada vista se suscribe con `useRuns`/`useRun`/`useAllRuns`; el polling corre mientras haya
// al menos un suscriptor, se pausa con la ventana oculta y se detiene con el último. Las
// mutaciones y los avisos `nodal://changed` llaman a `invalidate("runs")` (o `refreshRuns()`).
//
// En cada vuelta se leen de la base los runs livianos (`list_runs_light`, hasta 500) y el
// último de cada tarea (`latest_runs_by_task`, sin límite, para los badges), y de Claude Code
// las sesiones en background (`claude agents`), que no avisan: por eso el polling sigue. Los
// runs que abre una vista (`useRun`) se piden completos con `get_run`. El detalle del workflow
// (`get_run_detail`) se pide solo para runs de workflow: los activos en cada vuelta, los
// terminados hasta que se asienta. El resumen de la píldora sale de `work_summary`.

import { useEffect, useMemo, useSyncExternalStore } from "react";
import { getRun, latestRunsByTask, listRunsLight, workSummary, type WorkSummary } from "../api";
import type { Project, Repo, Run, RunLight, Settings, Task } from "../types";
import {
  POLL,
  registerResource,
  usePolled,
  useProjectList,
  useRepoList,
  useSettings,
  useTaskList,
  type Loadable,
} from "./store";
import { getRunDetail, listRuns } from "../../features/runs/api";
import { deriveRunView, type RunView } from "../../features/runs/status";
import { isInProgress, type RunDetail, type RunSummary } from "../../features/runs/types";

const POLL_MS = POLL.live;
/** Polls que se sigue pidiendo el detalle de un run terminado sin resumen final. */
const SETTLE_TRIES = 3;
/** Runs terminados cuyo detalle se pide sin que nadie lo pida explícitamente. */
const MAX_HISTORY_DETAILS = 40;

export interface RunsSnapshot {
  /** Hasta 500 runs, más recientes primero. */
  runs: RunLight[];
  /** Último run de cada tarea (cualquier tipo), sin límite de historial. */
  latest: RunLight[];
  /** Runs completos (con `prompt`) que pidió una vista con `useRun`; `null`: no existe o falló. */
  full: Record<string, Run | null>;
  live: RunSummary[];
  details: Record<string, RunDetail | null>;
  /** Errores de la base en la última vuelta (en inglés, listos para mostrar). */
  error: string | null;
  /** Error de `claude agents` (CLI), aparte: no afecta a los datos de la base. */
  liveError: string | null;
  /** La base respondió al menos una vez (`runs` vacío ya significa "no hay"). */
  loaded: boolean;
  /** `claude agents` respondió al menos una vez. */
  liveLoaded: boolean;
  now: number;
}

const EMPTY: RunsSnapshot = {
  runs: [],
  latest: [],
  full: {},
  live: [],
  details: {},
  error: null,
  liveError: null,
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
/** Detalles que ya no cambian: no se vuelven a pedir. Se podan a los runs que siguen visibles. */
const settled = new Set<string>();
const settleTries = new Map<string, number>();
/** Runs abiertos por una vista (`useRun`), con la cantidad de vistas que los usan. */
const wanted = new Map<string, number>();

function emit(next: Partial<RunsSnapshot>) {
  snapshot = { ...snapshot, ...next };
  for (const l of listeners) l();
}

function needsDetail(run: RunLight, live: RunSummary | null, historyIds: Set<string>): boolean {
  if (run.executor.kind !== "workflow" || !run.sessionId) return false;
  if (run.status === "launched" || isInProgress(live)) return true;
  if (settled.has(run.id)) return false;
  return historyIds.has(run.id) || wanted.has(run.id);
}

/** Deja en `map` solo las claves de `keep`. */
function prune<K>(set: { keys(): Iterable<K>; delete(k: K): unknown }, keep: Set<K>) {
  for (const k of [...set.keys()]) if (!keep.has(k)) set.delete(k);
}

async function pollOnce() {
  const wantedIds = [...wanted.keys()];
  const [runsR, latestR, liveR, fullR] = await Promise.allSettled([
    listRunsLight(null),
    latestRunsByTask(null),
    listRuns(),
    // Solo "no existe" se guarda como `null`; un error pasajero conserva lo anterior.
    Promise.all(
      wantedIds.map((id) =>
        getRun(id).then(
          (r) => [id, r] as const,
          (e: unknown) => [id, /no longer exists/i.test(String(e)) ? null : snapshot.full[id]] as const,
        ),
      ),
    ),
  ]);
  const errors: string[] = [];
  const next: Partial<RunsSnapshot> = { now: Date.now() };
  if (runsR.status === "fulfilled") {
    next.runs = runsR.value;
    next.loaded = true;
  } else errors.push(`Couldn't list runs: ${String(runsR.reason)}`);
  if (latestR.status === "fulfilled") next.latest = latestR.value;
  else errors.push(`Couldn't list the latest runs: ${String(latestR.reason)}`);
  if (liveR.status === "fulfilled") {
    next.live = liveR.value;
    next.liveLoaded = true;
    next.liveError = null;
  } else next.liveError = `Couldn't read Claude Code sessions: ${String(liveR.reason)}`;
  if (fullR.status === "fulfilled") {
    const full: Record<string, Run | null> = {};
    for (const [id, r] of fullR.value) if (wanted.has(id) && r !== undefined) full[id] = r;
    next.full = full;
  }
  next.error = errors.length ? errors.join("\n") : null;
  emit(next);

  const runs = candidates(snapshot);
  const live = snapshot.live;
  // Lo que ya no está en ninguna lista no se vuelve a mirar: se olvida su estado.
  const present = new Set(runs.map((r) => r.id));
  prune(settled, present);
  prune(settleTries, present);
  if (Object.keys(snapshot.details).some((id) => !present.has(id))) {
    emit({ details: Object.fromEntries(Object.entries(snapshot.details).filter(([id]) => present.has(id))) });
  }

  const historyIds = new Set(
    snapshot.runs
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

/** Todos los runs conocidos, sin repetir: la lista, los últimos por tarea y los abiertos. */
function candidates(s: RunsSnapshot): RunLight[] {
  const seen = new Set(s.runs.map((r) => r.id));
  const out: RunLight[] = [...s.runs];
  for (const r of [...s.latest, ...Object.values(s.full)]) {
    if (!r || seen.has(r.id)) continue;
    seen.add(r.id);
    out.push(r);
  }
  return out;
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

registerResource("runs", refreshRuns);

function loop() {
  timer = null;
  const next = () => {
    if (listeners.size) timer = setTimeout(loop, POLL_MS);
    else looping = false;
  };
  // Ventana oculta: no se consulta; al volver, `visibilitychange` relee al instante.
  if (typeof document !== "undefined" && document.hidden) next();
  else void refreshRuns().finally(next);
}

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && listeners.size) void refreshRuns();
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
export function projectIdOf(run: RunLight, tasks: Map<string, Task>, repos: Map<string, Repo>): string | null {
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
  /** Run más reciente de cada tarea, de cualquier tipo (sin filtrar ni límite de historial). */
  latestByTask: Map<string, RunView>;
  /** Cola global (sin filtrar), en orden de salida. */
  queue: RunView[];
  projects: Map<string, Project>;
  repos: Map<string, Repo>;
  tasks: Map<string, Task>;
  settings: Settings | null;
  error: string | null;
  liveError: string | null;
  loaded: boolean;
  liveLoaded: boolean;
  now: number;
  refresh: () => Promise<void>;
}

interface Derived {
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
    const known = candidates(s);
    const queued = s.runs
      .filter((r) => r.status === "queued")
      .sort((a, b) => a.queuePosition - b.queuePosition || a.queuedAt - b.queuedAt || a.id.localeCompare(b.id));
    const queuePos = new Map(queued.map((r, i) => [r.id, i + 1]));
    const reviews = new Map<string, RunLight>();
    for (const r of known) {
      if (r.kind !== "review" || !r.parentRunId) continue;
      const prev = reviews.get(r.parentRunId);
      if (!prev || r.queuedAt > prev.queuedAt) reviews.set(r.parentRunId, r);
    }
    const ctx = { live: s.live, details: s.details, queuePos, reviews, now: s.now };
    const byId = new Map(known.map((r) => [r.id, deriveRunView(r, ctx)]));
    const all = s.runs.map((r) => byId.get(r.id)!);
    const latestByTask = new Map<string, RunView>();
    for (const r of s.latest) if (r.taskId) latestByTask.set(r.taskId, byId.get(r.id)!);
    // Sin respuesta de `latest_runs_by_task` todavía: lo que haya en la lista.
    if (!s.latest.length) {
      for (const v of all) if (v.run.taskId && !latestByTask.has(v.run.taskId)) latestByTask.set(v.run.taskId, v);
    }
    const queue = queued.map((r) => byId.get(r.id)!);
    return { all, byId, latestByTask, queue };
  })();
  derivedFor = s;
  return derived;
}

/** Mapa por id de una lista del store, cacheado por identidad (una vez por respuesta). */
const mapCache = new WeakMap<object, Map<string, unknown>>();
const EMPTY_LIST: never[] = [];
function byIdOf<T extends { id: string }>(list: T[] | undefined): Map<string, T> {
  const l = list ?? EMPTY_LIST;
  let m = mapCache.get(l) as Map<string, T> | undefined;
  if (!m) {
    m = new Map(l.map((x) => [x.id, x]));
    mapCache.set(l, m);
  }
  return m;
}

/**
 * Runs con su estado derivado. Todas las vistas comparten el mismo polling, así que se
 * puede llamar desde tantos componentes como haga falta.
 */
export function useRuns(filter: RunsFilter = {}): RunsState {
  const s = useRunsSnapshot();
  const d = derive(s);
  const projects = byIdOf(useProjectList().data);
  const repos = byIdOf(useRepoList().data);
  const tasks = byIdOf(useTaskList().data);
  const settings = useSettings().data ?? null;
  const { projectId = null, taskId = null } = filter;
  const views = useMemo(
    () =>
      d.all.filter((v) => {
        if (taskId && v.run.taskId !== taskId) return false;
        if (projectId && projectIdOf(v.run, tasks, repos) !== projectId) return false;
        return true;
      }),
    [d, projectId, taskId, tasks, repos],
  );
  return {
    views,
    byId: d.byId,
    latestByTask: d.latestByTask,
    queue: d.queue,
    projects,
    repos,
    tasks,
    settings,
    error: s.error,
    liveError: s.liveError,
    loaded: s.loaded,
    liveLoaded: s.liveLoaded,
    now: s.now,
    refresh: refreshRuns,
  };
}

/** Lista cruda de runs livianos (más recientes primero, hasta 500) con el contrato `Loadable`. */
export function useAllRuns(): Loadable<RunLight[]> {
  const s = useRunsSnapshot();
  return useMemo(
    () => ({ data: s.loaded ? s.runs : undefined, error: s.error, loading: !s.loaded && !s.error, refresh: refreshRuns }),
    [s.loaded, s.runs, s.error],
  );
}

/** Último run de cada tarea (cualquier tipo), sin límite de historial: para badges. */
export function useLatestRunByTask(): Map<string, RunLight> {
  const latest = useRunsSnapshot().latest;
  return useMemo(() => new Map(latest.flatMap((r) => (r.taskId ? [[r.taskId, r] as const] : []))), [latest]);
}

/** Runs de una tarea (más recientes primero), sin el tope de la lista global. */
export function useTaskRuns(taskId: string | null): Loadable<RunLight[]> {
  return usePolled<RunLight[]>(
    taskId ? `runs:task:${taskId}` : null,
    () => listRunsLight(null, taskId),
    ["runs"],
    POLL.slow,
  );
}

export const isRunActive = (r: RunLight) => r.status === "queued" || r.status === "launching" || r.status === "launched";

/**
 * Un run por id, aunque sea más viejo que la lista: se pide completo (`get_run`, con el
 * prompt) mientras la vista esté montada, y con su detalle de workflow.
 */
export function useRun(runId: string | null): {
  view: RunView | undefined;
  /** Completo, con `prompt`; `undefined` mientras se pide, `null` si no existe. */
  full: Run | null | undefined;
  state: RunsState;
} {
  const state = useRuns();
  const s = useRunsSnapshot();
  useEffect(() => {
    if (!runId) return;
    wanted.set(runId, (wanted.get(runId) ?? 0) + 1);
    void refreshRuns();
    return () => {
      const n = (wanted.get(runId) ?? 1) - 1;
      if (n > 0) wanted.set(runId, n);
      else {
        wanted.delete(runId);
        if (runId in snapshot.full) {
          const { [runId]: _drop, ...rest } = snapshot.full;
          emit({ full: rest });
        }
      }
    };
  }, [runId]);
  return {
    view: runId ? state.byId.get(runId) : undefined,
    full: runId ? s.full[runId] : undefined,
    state,
  };
}

export interface QueueSummary {
  /** Slots ocupados de la concurrencia global (mismo criterio que la cola del backend). */
  running: number;
  /** Concurrencia global (Settings). */
  capacity: number;
  /** Tareas Blocked, runs migrados sin confirmar y sesiones esperando permiso/input (sin duplicar). */
  needYou: number;
  /** Runs listos para salir. */
  queued: number;
  /** Error de la última pasada de la cola, o `null`. */
  pumpError: string | null;
  /** "2/4 running · 1 need you · 3 queued" (partes vacías omitidas). */
  label: string;
  loaded: boolean;
}

/**
 * Resumen de trabajo (`work_summary`): fuente única de la píldora, la paleta y el badge del
 * Dock. `null`: global (incluye sesiones de Claude Code ajenas a la app).
 */
export function useQueueSummary(projectId: string | null = null): QueueSummary {
  const q = usePolled<WorkSummary>(`work-summary:${projectId ?? "*"}`, () => workSummary(projectId), ["runs", "tasks"], POLL.live);
  const d = q.data;
  return useMemo(() => {
    const running = d?.running ?? 0;
    const capacity = d?.capacity ?? 0;
    const needYou = d?.needYou ?? 0;
    const queued = d?.queued ?? 0;
    const pumpError = d?.pumpError ?? null;
    const parts = [`${running}/${capacity || "?"} running`];
    if (needYou) parts.push(`${needYou} need you`);
    if (queued) parts.push(`${queued} queued`);
    return { running, capacity, needYou, queued, pumpError, label: parts.join(" · "), loaded: d !== undefined };
  }, [d]);
}
