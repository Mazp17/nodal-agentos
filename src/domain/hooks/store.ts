// Shared data layer. One store per resource (projects, repos, tasks, settings; runs
// live in `runs.ts` with the same contract) with ONE timer per resource: the global list is read
// and each view filters its own, so board, panel, sidebar and palette share a single query.
// Polling runs while someone is subscribed and pauses while the window is hidden; after a
// mutation, `invalidate(...)` re-reads immediately (and returns that re-read's promise).
// Keyed queries without polling (relations, plan, executors) use the same engine with
// interval 0 and are re-read when their resource is invalidated.
// The backend notifies with `nodal://changed` (see `onChanged`): each notice invalidates the
// matching resource, so polling is only a (slower) fallback.

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
  worktreeStatus,
  type ChangedKind,
  type ExecutorInfo,
  type WorktreeStatus,
} from "../api";
import type { Project, Repo, Settings, Task, TaskRelation } from "../types";

export type Resource = "projects" | "repos" | "tasks" | "runs" | "settings" | "sources";

/**
 * Fallback polling intervals (ms): changes arrive through `nodal://changed`. `live`
 * is for whatever depends on Claude Code sessions (`claude agents`), which don't notify.
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
  /** Deferred deletion of the entry with no subscribers (cancelled if someone comes back). */
  evict: number | undefined;
  seq: number;
  inflight: Promise<void> | null;
}

/** How long a key with nobody mounted stays cached. */
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

/** Resources with their own store (runs): re-read with their function on invalidation. */
const externals = new Map<Resource, () => Promise<void>>();

export function registerResource(resource: Resource, reload: () => Promise<void>) {
  externals.set(resource, reload);
}

/**
 * Re-reads those resources' keys now. Those with nobody mounted are dropped from the
 * cache (on return they load from scratch instead of showing stale data).
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

/** Optimistic change to `key`'s cached value (the subsequent re-read corrects it). */
export function setData<T>(key: string, update: (prev: T) => T) {
  const e = store.get(key);
  if (!e || !e.snap.hasValue) return;
  // Discards the response of a read already in flight (it would bring the pre-change value).
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

// ---------- Backend notices ----------

/** Which resources each `nodal://changed` `kind` re-reads. */
const CHANGED_RESOURCES: Record<ChangedKind, Resource[]> = {
  tasks: ["tasks"],
  runs: ["runs"],
  queue: ["runs"],
  sources: ["sources"],
  // Repos and settings also notify as `projects`.
  projects: ["projects", "repos", "settings"],
};

/** Notices arrive per `kind` (runs, queue and tasks together): they're batched into a single re-read. */
const CHANGE_BATCH_MS = 50;
let pendingChanges = new Set<Resource>();
let changeTimer: number | undefined;

function flushChanges() {
  changeTimer = undefined;
  // Hidden window: they accumulate and are re-read on return.
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
  const unlisten = onChanged((e) => onBackendChange(e.kind));
  unlisten.catch((err: unknown) => console.error("nodal://changed", err));
  // In dev, HMR reloads the module: without this the listeners pile up.
  import.meta.hot?.dispose(() => void unlisten.then((u) => u(), () => {}));
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
 * Loads `fetcher` for `key` (shared across components), repeats it every `intervalMs`
 * (0 = never) while someone is mounted and whenever one of `resources` is invalidated.
 * The key must encode the fetcher's arguments.
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

/** Derives a filtered/transformed `Loadable` from another without querying again. */
function useSelect<T, U>(src: Loadable<T>, pick: (v: T) => U, deps: unknown[]): Loadable<U> {
  const { data, error, loading, refresh } = src;
  const out = useMemo(() => (data === undefined ? undefined : pick(data)), [data, ...deps]);
  return useMemo(() => ({ data: out, error, loading, refresh }), [out, error, loading, refresh]);
}

// ---------- Base resources (one key and one timer each) ----------

export const KEYS = { projects: "projects", repos: "repos", tasks: "tasks", settings: "settings" } as const;

export const useProjectList = () =>
  usePolled<Project[]>(KEYS.projects, () => listProjects(false), ["projects"], POLL.slow);

/** Every repo, sorted by position. */
export const useRepoList = () =>
  usePolled<Repo[]>(
    KEYS.repos,
    () => listRepos(null).then((rs) => [...rs].sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))),
    ["repos", "projects"],
    POLL.slow,
  );

export const useTaskList = () => usePolled<Task[]>(KEYS.tasks, () => listTasks(null), ["tasks"], POLL.tasks);

export const useSettings = () => usePolled<Settings>(KEYS.settings, getSettings, ["settings"], POLL.slow);

// ---------- Selections ----------

/** `null`: repos of every project. */
export function useRepos(projectId: string | null): Loadable<Repo[]> {
  return useSelect(useRepoList(), (rs) => (projectId ? rs.filter((r) => r.projectId === projectId) : rs), [projectId]);
}

/** `null`: tasks of every project. */
export function useTasks(projectId: string | null): Loadable<Task[]> {
  return useSelect(useTaskList(), (ts) => (projectId ? ts.filter((t) => t.projectId === projectId) : ts), [projectId]);
}

/** A task from the shared list; error if the list loaded and it isn't there (deleted). */
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

// ---------- Keyed, without polling ----------

export const useTaskRelations = (taskId: string | null) =>
  usePolled<TaskRelation[]>(taskId ? `rels:${taskId}` : null, () => listTaskRelations(taskId as string), ["tasks"], 0);

/** Re-read when tasks are invalidated (or with `refresh` when `updatedAt` changes). */
export const useTaskPlan = (taskId: string | null) =>
  usePolled<string>(taskId ? `plan:${taskId}` : null, () => readTaskPlan(taskId as string), ["tasks"], 0);

/** Git status of the task's worktree (ahead/unpushed/dirty); changes while the agent works. */
export const useWorktreeStatus = (taskId: string | null) =>
  usePolled<WorktreeStatus>(taskId ? `worktree:${taskId}` : null, () => worktreeStatus(taskId as string), ["tasks", "runs"], POLL.tasks);

/** Executor catalog; it reads disk, so it isn't repeated (only on invalidation). */
export const useExecutors = (repoId: string | null) =>
  usePolled<ExecutorInfo[]>(`executors:${repoId ?? "*"}`, () => listExecutors(repoId), ["repos"], 0);
