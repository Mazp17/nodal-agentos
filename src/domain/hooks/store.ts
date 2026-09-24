// Capa de datos compartida. Un store por recurso (projects, repos, tasks, settings; los runs
// viven en `runs.ts` con el mismo contrato) con UN timer por recurso: se lee la lista global y
// cada vista filtra lo suyo, así board, panel, sidebar y paleta comparten una sola consulta.
// El polling corre mientras haya alguien suscripto y se pausa con la ventana oculta; tras una
// mutación, `invalidate(...)` relee al instante (y devuelve la promesa de esa relectura).
// Las consultas por clave sin polling (relaciones, plan, ejecutores) usan el mismo motor con
// intervalo 0 y se releen al invalidar su recurso.
// El backend avisa con `nodal://changed` (ver `onChanged`): cada aviso invalida el recurso
// que corresponde, así que el polling queda solo de respaldo (más lento).

import { useCallback, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from "react";
import {
  getSettings,
  onChanged,
  listExecutors,
  listProjects,
  listRepos,
  listTaskRelations,
  listTasks,
  readTaskPlan,
  type ChangedKind,
  type ExecutorInfo,
} from "../api";
import type { Project, Repo, Settings, Task, TaskRelation } from "../types";

export type Resource = "projects" | "repos" | "tasks" | "runs" | "settings" | "sources";

/**
 * Intervalos de polling (ms), de respaldo: los cambios llegan por `nodal://changed`. `live`
 * es para lo que depende de sesiones de Claude Code (`claude agents`), que no avisan.
 */
export const POLL = { tasks: 20_000, live: 5000, slow: 60_000 } as const;

export interface Loadable<T> {
  data: T | undefined;
  error: string | null;
  loading: boolean;
  refresh: () => Promise<void>;
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
  /** Borrado diferido de la entrada sin suscriptores (se cancela si alguien vuelve). */
  evict: number | undefined;
  seq: number;
  inflight: Promise<void> | null;
}

/** Tiempo que se conserva en caché una clave sin nadie montado. */
const EVICT_MS = 60_000;

const store = new Map<string, Entry>();
const EMPTY: Snapshot = { value: undefined, hasValue: false, error: null };

function notify(e: Entry) {
  e.listeners.forEach((l) => l());
}

function load(key: string): Promise<void> {
  const e = store.get(key);
  if (!e) return Promise.resolve();
  const mine = ++e.seq;
  const p = e
    .fetcher()
    .then((value) => {
      if (mine !== e.seq) return;
      e.snap = { value, hasValue: true, error: null };
      notify(e);
    })
    .catch((err: unknown) => {
      if (mine !== e.seq) return;
      e.snap = { ...e.snap, error: String(err) };
      notify(e);
    })
    .finally(() => {
      if (e.inflight === p) e.inflight = null;
    });
  e.inflight = p;
  return p;
}

/** Recursos con store propio (runs): se releen con su función al invalidar. */
const externals = new Map<Resource, () => Promise<void>>();

export function registerResource(resource: Resource, reload: () => Promise<void>) {
  externals.set(resource, reload);
}

/**
 * Relee ya las claves de esos recursos. Las que no tienen a nadie montado se descartan de la
 * caché (al volver se cargan de cero en vez de mostrar datos viejos).
 */
export function invalidate(...resources: Resource[]): Promise<void> {
  const jobs: Promise<void>[] = [];
  for (const [k, e] of [...store.entries()]) {
    if (!e.resources.some((r) => resources.includes(r))) continue;
    if (e.listeners.size > 0) jobs.push(load(k));
    else {
      if (e.evict !== undefined) window.clearTimeout(e.evict);
      store.delete(k);
    }
  }
  for (const r of resources) {
    const ext = externals.get(r);
    if (ext) jobs.push(ext());
  }
  return Promise.all(jobs).then(() => undefined);
}

/** Cambio optimista del valor en caché de `key` (la relectura posterior lo corrige). */
export function setData<T>(key: string, update: (prev: T) => T) {
  const e = store.get(key);
  if (!e || !e.snap.hasValue) return;
  // Descarta la respuesta de una lectura ya en curso (traería el valor anterior al cambio).
  e.seq++;
  e.snap = { ...e.snap, value: update(e.snap.value as T) };
  notify(e);
}

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) return;
    for (const [k, e] of store) if (e.listeners.size > 0 && e.interval > 0) void load(k);
  });
}

// ---------- Avisos del backend ----------

/** Qué recursos relee cada `kind` de `nodal://changed`. */
const CHANGED_RESOURCES: Record<ChangedKind, Resource[]> = {
  tasks: ["tasks"],
  runs: ["runs"],
  queue: ["runs"],
  sources: ["sources"],
  // Repos y settings también avisan como `projects`.
  projects: ["projects", "repos", "settings"],
};

/** Los avisos llegan por `kind` (runs, queue y tasks juntos): se agrupan en una sola relectura. */
const CHANGE_BATCH_MS = 50;
let pendingChanges = new Set<Resource>();
let changeTimer: number | undefined;

function flushChanges() {
  changeTimer = undefined;
  // Ventana oculta: se acumulan y se releen al volver.
  if (document.hidden || !pendingChanges.size) return;
  const rs = [...pendingChanges];
  pendingChanges = new Set();
  void invalidate(...rs);
}

function onBackendChange(kind: ChangedKind) {
  for (const r of CHANGED_RESOURCES[kind] ?? []) pendingChanges.add(r);
  if (changeTimer === undefined) changeTimer = window.setTimeout(flushChanges, CHANGE_BATCH_MS);
}

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden && changeTimer === undefined) flushChanges();
  });
}

if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
  onChanged((e) => onBackendChange(e.kind)).catch((err: unknown) => console.error("nodal://changed", err));
}

function entry(key: string, fetcher: () => Promise<unknown>, resources: Resource[], interval: number): Entry {
  let e = store.get(key);
  if (!e) {
    e = {
      snap: EMPTY,
      fetcher,
      resources,
      interval,
      listeners: new Set(),
      timer: undefined,
      evict: undefined,
      seq: 0,
      inflight: null,
    };
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
    (listener: () => void) => {
      if (key === null) return () => {};
      const e = entry(key, () => fetchRef.current(), resKey.split(",").filter(Boolean) as Resource[], intervalMs);
      e.listeners.add(listener);
      if (e.evict !== undefined) {
        window.clearTimeout(e.evict);
        e.evict = undefined;
      }
      if (e.listeners.size === 1) {
        if (!e.inflight) void load(key);
        if (intervalMs > 0) {
          e.timer = window.setInterval(() => {
            if (!document.hidden && !e.inflight) void load(key);
          }, intervalMs);
        }
      }
      return () => {
        e.listeners.delete(listener);
        if (e.listeners.size > 0) return;
        if (e.timer !== undefined) {
          window.clearInterval(e.timer);
          e.timer = undefined;
        }
        e.evict = window.setTimeout(() => {
          if (store.get(key) === e && e.listeners.size === 0) store.delete(key);
        }, EVICT_MS);
      };
    },
    [key, intervalMs, resKey],
  );

  const snap = useSyncExternalStore(subscribe, () => (key === null ? EMPTY : (store.get(key)?.snap ?? EMPTY)));
  const refresh = useCallback(() => (key !== null ? load(key) : Promise.resolve()), [key]);

  return {
    data: snap.hasValue ? (snap.value as T) : undefined,
    error: snap.error,
    loading: key !== null && !snap.hasValue && snap.error === null,
    refresh,
  };
}

/** Deriva un `Loadable` filtrado/transformado de otro sin volver a consultar. */
function useSelect<T, U>(src: Loadable<T>, pick: (v: T) => U, deps: unknown[]): Loadable<U> {
  const { data, error, loading, refresh } = src;
  const out = useMemo(() => (data === undefined ? undefined : pick(data)), [data, ...deps]);
  return useMemo(() => ({ data: out, error, loading, refresh }), [out, error, loading, refresh]);
}

// ---------- Recursos base (una clave y un timer cada uno) ----------

export const KEYS = { projects: "projects", repos: "repos", tasks: "tasks", settings: "settings" } as const;

export const useProjectList = () =>
  usePolled<Project[]>(KEYS.projects, () => listProjects(false), ["projects"], POLL.slow);

/** Todos los repos, ordenados por posición. */
export const useRepoList = () =>
  usePolled<Repo[]>(
    KEYS.repos,
    () => listRepos(null).then((rs) => [...rs].sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))),
    ["repos", "projects"],
    POLL.slow,
  );

export const useTaskList = () => usePolled<Task[]>(KEYS.tasks, () => listTasks(null), ["tasks"], POLL.tasks);

export const useSettings = () => usePolled<Settings>(KEYS.settings, getSettings, ["settings"], POLL.slow);

// ---------- Selecciones ----------

/** `null`: repos de todos los proyectos. */
export function useRepos(projectId: string | null): Loadable<Repo[]> {
  return useSelect(useRepoList(), (rs) => (projectId ? rs.filter((r) => r.projectId === projectId) : rs), [projectId]);
}

/** `null`: tareas de todos los proyectos. */
export function useTasks(projectId: string | null): Loadable<Task[]> {
  return useSelect(useTaskList(), (ts) => (projectId ? ts.filter((t) => t.projectId === projectId) : ts), [projectId]);
}

/** Una tarea de la lista compartida; error si la lista cargó y no está (borrada). */
export function useTask(taskId: string | null): Loadable<Task> {
  const all = useTaskList();
  const task = taskId && all.data ? all.data.find((t) => t.id === taskId) : undefined;
  const missing = taskId !== null && all.data !== undefined && task === undefined;
  return useMemo(
    () => ({
      data: task,
      error: missing ? (all.error ?? "This task no longer exists.") : all.error,
      loading: taskId !== null && all.data === undefined && all.error === null,
      refresh: all.refresh,
    }),
    [task, missing, all.error, all.data, taskId, all.refresh],
  );
}

// ---------- Por clave, sin polling ----------

export const useTaskRelations = (taskId: string | null) =>
  usePolled<TaskRelation[]>(taskId ? `rels:${taskId}` : null, () => listTaskRelations(taskId as string), ["tasks"], 0);

/** Se relee al invalidar tareas (o con `refresh` cuando cambia `updatedAt`). */
export const useTaskPlan = (taskId: string | null) =>
  usePolled<string>(taskId ? `plan:${taskId}` : null, () => readTaskPlan(taskId as string), ["tasks"], 0);

/** Catálogo de ejecutores; lee disco, así que no se repite (solo al invalidar). */
export const useExecutors = (repoId: string | null) =>
  usePolled<ExecutorInfo[]>(`executors:${repoId ?? "*"}`, () => listExecutors(repoId), ["repos"], 0);
