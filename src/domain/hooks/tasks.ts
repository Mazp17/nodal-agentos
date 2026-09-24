// Hooks de datos para board, tareas y ejecutores. Un store compartido por clave: varios
// componentes que piden lo mismo (board, panel, diálogo) comparten una sola consulta y un
// solo timer. Polling pausado con la ventana oculta, más invalidación explícita: tras una
// mutación se llama `invalidate(...)` y las claves de ese recurso refrescan al instante.

import { useCallback, useLayoutEffect, useRef, useSyncExternalStore } from "react";
import {
  getSettings,
  getTask,
  listExecutors,
  listProjects,
  listRepos,
  listTaskRelations,
  listTaskRuns,
  listTasks,
  readTaskPlan,
  type ExecutorInfo,
} from "../api";
import type { Project, Repo, Run, Settings, Task, TaskRelation } from "../types";

export type Resource = "projects" | "repos" | "tasks" | "runs" | "settings";

/** Intervalos de polling (ms). Las tareas cambian por el pump y el sync; los runs, más seguido. */
export const POLL = { tasks: 5000, runs: 3000, slow: 30000 } as const;

export interface Loadable<T> {
  data: T | undefined;
  error: string | null;
  loading: boolean;
  refresh: () => void;
}

interface Snapshot {
  value: unknown;
  hasValue: boolean;
  error: string | null;
}

interface Entry {
  snap: Snapshot;
  fetcher: () => Promise<unknown>;
  resources: Resource[];
  interval: number;
  listeners: Set<() => void>;
  timer: number | undefined;
  seq: number;
}

const store = new Map<string, Entry>();
const EMPTY: Snapshot = { value: undefined, hasValue: false, error: null };

function load(key: string) {
  const e = store.get(key);
  if (!e) return;
  const mine = ++e.seq;
  e.fetcher()
    .then((value) => {
      if (mine !== e.seq) return;
      e.snap = { value, hasValue: true, error: null };
      e.listeners.forEach((l) => l());
    })
    .catch((err: unknown) => {
      if (mine !== e.seq) return;
      e.snap = { ...e.snap, error: String(err) };
      e.listeners.forEach((l) => l());
    });
}

function active(): string[] {
  return [...store.entries()].filter(([, e]) => e.listeners.size > 0).map(([k]) => k);
}

/** Pide a las claves montadas de esos recursos que refresquen ya. */
export function invalidate(...resources: Resource[]) {
  for (const k of active()) {
    const e = store.get(k);
    if (e && e.resources.some((r) => resources.includes(r))) load(k);
  }
}

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) active().forEach(load);
  });
}

function entry(key: string, fetcher: () => Promise<unknown>, resources: Resource[], interval: number): Entry {
  let e = store.get(key);
  if (!e) {
    e = { snap: EMPTY, fetcher, resources, interval, listeners: new Set(), timer: undefined, seq: 0 };
    store.set(key, e);
  }
  return e;
}

/**
 * Carga `fetcher` para `key` (compartido entre componentes), lo repite cada `intervalMs`
 * (0 = nunca) mientras haya alguien montado y cuando se invalida uno de `resources`.
 * La clave tiene que codificar los argumentos del fetcher.
 */
export function usePolled<T>(
  key: string | null,
  fetcher: () => Promise<T>,
  resources: Resource[],
  intervalMs: number,
): Loadable<T> {
  const fetchRef = useRef(fetcher);
  useLayoutEffect(() => {
    fetchRef.current = fetcher;
  });
  const resKey = resources.join(",");

  const subscribe = useCallback(
    (notify: () => void) => {
      if (key === null) return () => {};
      const e = entry(key, () => fetchRef.current(), resKey.split(",").filter(Boolean) as Resource[], intervalMs);
      e.listeners.add(notify);
      if (e.listeners.size === 1) {
        load(key);
        if (intervalMs > 0) {
          e.timer = window.setInterval(() => {
            if (!document.hidden) load(key);
          }, intervalMs);
        }
      }
      return () => {
        e.listeners.delete(notify);
        if (e.listeners.size === 0 && e.timer !== undefined) {
          window.clearInterval(e.timer);
          e.timer = undefined;
        }
      };
    },
    [key, intervalMs, resKey],
  );

  const snap = useSyncExternalStore(subscribe, () => (key === null ? EMPTY : (store.get(key)?.snap ?? EMPTY)));
  const refresh = useCallback(() => {
    if (key !== null) load(key);
  }, [key]);

  return {
    data: snap.hasValue ? (snap.value as T) : undefined,
    error: snap.error,
    loading: key !== null && !snap.hasValue && snap.error === null,
    refresh,
  };
}

// ---------- Recursos ----------

export const useProjects = () => usePolled<Project[]>("projects", () => listProjects(false), ["projects"], POLL.slow);

/** `null`: repos de todos los proyectos. */
export const useRepos = (projectId: string | null) =>
  usePolled<Repo[]>(`repos:${projectId ?? "*"}`, () => listRepos(projectId), ["repos", "projects"], POLL.slow);

/** `null`: tareas de todos los proyectos. */
export const useTasks = (projectId: string | null) =>
  usePolled<Task[]>(`tasks:${projectId ?? "*"}`, () => listTasks(projectId), ["tasks"], POLL.tasks);

/**
 * Todos los runs (más recientes primero). El backend no filtra por proyecto, así que la vista
 * filtra por las tareas que muestra.
 */
export const useAllRuns = () => usePolled<Run[]>("runs:*", () => listTaskRuns(null), ["runs"], POLL.runs);

export const useTaskRuns = (taskId: string | null) =>
  usePolled<Run[]>(taskId ? `runs:${taskId}` : null, () => listTaskRuns(taskId), ["runs"], POLL.runs);

export const useTask = (taskId: string | null) =>
  usePolled<Task>(taskId ? `task:${taskId}` : null, () => getTask(taskId as string), ["tasks"], POLL.tasks);

export const useTaskRelations = (taskId: string | null) =>
  usePolled<TaskRelation[]>(
    taskId ? `rels:${taskId}` : null,
    () => listTaskRelations(taskId as string),
    ["tasks"],
    0,
  );

/** El plan se relee cuando cambia la tarea (`updatedAt`). */
export const useTaskPlan = (taskId: string | null, version: number | null) =>
  usePolled<string>(
    taskId && version !== null ? `plan:${taskId}:${version}` : null,
    () => readTaskPlan(taskId as string),
    [],
    0,
  );

/** Catálogo de ejecutores; lee disco, así que no se repite (solo al invalidar). */
export const useExecutors = (repoId: string | null) =>
  usePolled<ExecutorInfo[]>(`executors:${repoId ?? "*"}`, () => listExecutors(repoId), ["repos"], 0);

export const useSettings = () => usePolled<Settings>("settings", getSettings, ["settings"], POLL.slow);

// ---------- Derivados de runs ----------

export const isRunActive = (r: Run) => r.status === "queued" || r.status === "launching" || r.status === "launched";

/** Último run de cada tarea (la lista viene de más reciente a más viejo). */
export function latestRunByTask(runs: Run[] | undefined): Map<string, Run> {
  const map = new Map<string, Run>();
  for (const r of runs ?? []) {
    if (r.taskId && !map.has(r.taskId)) map.set(r.taskId, r);
  }
  return map;
}

/** Posición 1-based en la cola global de un run `queued`. */
export function queuePositions(runs: Run[] | undefined): Map<string, number> {
  const queued = (runs ?? []).filter((r) => r.status === "queued").sort((a, b) => a.queuePosition - b.queuePosition);
  return new Map(queued.map((r, i) => [r.id, i + 1]));
}
