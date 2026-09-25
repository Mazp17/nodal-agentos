// "runs" resource of the data layer (see `store.ts`): ONE single polling loop for the whole app.
// Each view subscribes with `useRuns`/`useRun`/`useAllRuns`; polling runs while there is
// at least one subscriber, pauses while the window is hidden and stops with the last one.
// Mutations and `nodal://changed` notices call `invalidate("runs")` (or `refreshRuns()`).
//
// Each pass reads from the database the light runs (`list_runs_light`, up to 500) and the
// latest of each task (`latest_runs_by_task`, unlimited, for the badges), and from Claude Code
// the background sessions (`claude agents`), which don't notify: that's why polling continues.
// Runs opened by a view (`useRun`) are fetched in full with `get_run`. The workflow detail
// (`get_run_detail`) is fetched only for workflow runs: active ones on every pass, finished
// ones until they settle. The pill's summary comes from `work_summary`.

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
/** Polls during which the detail of a finished run without a final summary keeps being fetched. */
const SETTLE_TRIES = 3;
/** Finished runs whose detail is fetched without anyone explicitly asking for it. */
const MAX_HISTORY_DETAILS = 40;

export interface RunsSnapshot {
  /** Up to 500 runs, most recent first. */
  runs: RunLight[];
  /** Latest run of each task (any kind), with no history limit. */
  latest: RunLight[];
  /** Full runs (with `prompt`) requested by a view with `useRun`; `null`: doesn't exist or failed. */
  full: Record<string, Run | null>;
  live: RunSummary[];
  details: Record<string, RunDetail | null>;
  /** Database errors from the last pass (in English, ready to display). */
  error: string | null;
  /** `claude agents` (CLI) error, kept separate: it doesn't affect the database data. */
  liveError: string | null;
  /** The database answered at least once (an empty `runs` now means "there are none"). */
  loaded: boolean;
  /** `claude agents` answered at least once. */
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
/** There's a live polling loop (waiting on the timer or with a pass in progress). */
let looping = false;
let inFlight: Promise<void> | null = null;
let again = false;
/** Details that no longer change: they aren't fetched again. Pruned to the runs still visible. */
const settled = new Set<string>();
const settleTries = new Map<string, number>();
/** Runs opened by a view (`useRun`), with the number of views using them. */
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

/** Keeps only the keys of `keep` in `map`. */
function prune<K>(set: { keys(): Iterable<K>; delete(k: K): unknown }, keep: Set<K>) {
  for (const k of [...set.keys()]) if (!keep.has(k)) set.delete(k);
}

async function pollOnce() {
  const wantedIds = [...wanted.keys()];
  const [runsR, latestR, liveR, fullR] = await Promise.allSettled([
    listRunsLight(null),
    latestRunsByTask(null),
    listRuns(),
    // Only "doesn't exist" is stored as `null`; a transient error keeps the previous value.
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
  // Whatever is no longer in any list isn't looked at again: its state is forgotten.
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
          // After finishing, the final summary may take a while to appear: retry a couple of times.
          const n = (settleTries.get(r.id) ?? 0) + 1;
          settleTries.set(r.id, n);
          if (d === null || d.source === "final" || n >= SETTLE_TRIES) settled.add(r.id);
        }
        return [r.id, d] as const;
      } catch {
        return null; // retried on the next pass
      }
    }),
  );
  const updates = fetched.filter((x) => x !== null);
  if (updates.length) emit({ details: { ...snapshot.details, ...Object.fromEntries(updates) } });
}

/** Every known run, without duplicates: the list, the latest per task and the opened ones. */
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

/** Re-reads everything now (without overlapping: if a pass is in progress, it repeats when done). */
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
  // Hidden window: no querying; on return, `visibilitychange` re-reads immediately.
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
  // A single loop: if the last subscriber left with a pass in progress and another one arrives
  // before it finishes, the same loop continues (a second one isn't started).
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

/** Raw store snapshot (re-renders on every polling pass). */
export function useRunsSnapshot(): RunsSnapshot {
  return useSyncExternalStore(subscribe, getSnapshot);
}

/** A run's project: through its task or, if it has none, through its repo. */
export function projectIdOf(run: RunLight, tasks: Map<string, Task>, repos: Map<string, Repo>): string | null {
  const t = run.taskId ? tasks.get(run.taskId) : undefined;
  if (t) return t.projectId;
  const r = run.repoId ? repos.get(run.repoId) : undefined;
  return r?.projectId ?? null;
}

export interface RunsFilter {
  /** `null`/absent: all projects. */
  projectId?: string | null;
  taskId?: string | null;
}

export interface RunsState {
  /** Filtered runs, most recent first. */
  views: RunView[];
  /** Every run (unfiltered), by id. */
  byId: Map<string, RunView>;
  /** Most recent run of each task, of any kind (unfiltered and with no history limit). */
  latestByTask: Map<string, RunView>;
  /** Global queue (unfiltered), in launch order. */
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

// Derived once per snapshot, shared by every subscriber.
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
    // No response from `latest_runs_by_task` yet: whatever is in the list.
    if (!s.latest.length) {
      for (const v of all) if (v.run.taskId && !latestByTask.has(v.run.taskId)) latestByTask.set(v.run.taskId, v);
    }
    const queue = queued.map((r) => byId.get(r.id)!);
    return { all, byId, latestByTask, queue };
  })();
  derivedFor = s;
  return derived;
}

/** By-id map of a store list, cached by identity (once per response). */
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
 * Runs with their derived state. Every view shares the same polling, so it can be
 * called from as many components as needed.
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

/** Raw list of light runs (most recent first, up to 500) with the `Loadable` contract. */
export function useAllRuns(): Loadable<RunLight[]> {
  const s = useRunsSnapshot();
  return useMemo(
    () => ({ data: s.loaded ? s.runs : undefined, error: s.error, loading: !s.loaded && !s.error, refresh: refreshRuns }),
    [s.loaded, s.runs, s.error],
  );
}

/** Latest run of each task (any kind), with no history limit: for badges. */
export function useLatestRunByTask(): Map<string, RunLight> {
  const latest = useRunsSnapshot().latest;
  return useMemo(() => new Map(latest.flatMap((r) => (r.taskId ? [[r.taskId, r] as const] : []))), [latest]);
}

/** A task's runs (most recent first), without the global list's cap. */
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
 * A run by id, even if it's older than the list: fetched in full (`get_run`, with the
 * prompt) while the view is mounted, along with its workflow detail.
 */
export function useRun(runId: string | null): {
  view: RunView | undefined;
  /** Full, with `prompt`; `undefined` while fetching, `null` if it doesn't exist. */
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
  /** Occupied slots of the global concurrency (same criteria as the backend queue). */
  running: number;
  /** Global concurrency (Settings). */
  capacity: number;
  /** Blocked tasks, unconfirmed migrated runs and sessions waiting for permission/input (no duplicates). */
  needYou: number;
  /** Runs ready to go. */
  queued: number;
  /** Error from the queue's last pass, or `null`. */
  pumpError: string | null;
  /** "2/4 running · 1 need you · 3 queued" (empty parts omitted). */
  label: string;
  loaded: boolean;
}

/**
 * Work summary (`work_summary`): single source for the pill, the palette and the Dock
 * badge. `null`: global (includes Claude Code sessions outside the app).
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
