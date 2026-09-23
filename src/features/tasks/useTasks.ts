import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getRunDetail, listRuns } from "../runs/api";
import type { RunView } from "../runs/status";
import { isInProgress, type RunDetail, type RunSummary } from "../runs/types";
import { findRun } from "../runs/useRuns";
import { listTaskRuns, listTasks } from "./api";
import { asIssueRun, viewOfTaskRun } from "./status";
import type { Task, TaskRun } from "./types";

const POLL_MS = 3000;
/** Polls que se sigue pidiendo el detalle de un run terminado sin resumen final. */
const SETTLE_TRIES = 3;

export interface TasksState {
  tasks: Task[];
  /** Vista del run vigente (el último encolado) por id de tarea. */
  current: Map<string, RunView>;
  /** Historial de runs de una tarea, más recientes primero. */
  historyOf: (taskId: string) => RunView[];
  error: string | null;
  loading: boolean;
  refresh: () => Promise<void>;
}

/**
 * Polling de tareas + runs de tareas + `claude agents` (y el detalle de los runs
 * vigentes) mientras `enabled`. `repoPath = null` trae las de todos los repos.
 * `sharedRuns`: la lista de `claude agents` que ya pollea otro hook (`useRuns`), una vez
 * cargada (`undefined` mientras no); con ella no se vuelve a llamar a `list_runs`.
 */
export function useTasks(repoPath: string | null, enabled = true, sharedRuns?: RunSummary[]): TasksState {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [taskRuns, setTaskRuns] = useState<TaskRun[]>([]);
  const [ownRuns, setRuns] = useState<RunSummary[]>([]);
  const runs = sharedRuns ?? ownRuns;
  const sharedRef = useRef(sharedRuns);
  sharedRef.current = sharedRuns;
  const [details, setDetails] = useState<Record<string, RunDetail | null>>({});
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const settled = useRef(new Set<string>());
  const tries = useRef(new Map<string, number>());
  const inFlight = useRef<Promise<void> | null>(null);
  const again = useRef(false);
  const repoRef = useRef(repoPath);
  repoRef.current = repoPath;

  const pollOnce = useCallback(async () => {
    const repo = repoRef.current;
    const shared = sharedRef.current;
    const [t, tr, r] = await Promise.allSettled([
      listTasks(repo),
      listTaskRuns(null),
      shared ? Promise.resolve(shared) : listRuns(),
    ]);
    // Cambió el repo mientras tanto: esta respuesta es del anterior.
    if (repo !== repoRef.current) return;
    const errors: string[] = [];
    if (t.status === "fulfilled") setTasks(t.value);
    else errors.push(`Couldn't list tasks: ${String(t.reason)}`);
    if (tr.status === "fulfilled") setTaskRuns(tr.value);
    else errors.push(`Couldn't list task runs: ${String(tr.reason)}`);
    if (r.status === "fulfilled") {
      if (!shared) setRuns(r.value);
    }
    else errors.push(`Couldn't list runs: ${String(r.reason)}`);
    setError(errors.length ? errors.join("\n") : null);
    setLoading(false);
    if (r.status === "rejected" || tr.status === "rejected") return;

    // Detalle (fase, resultado) de los runs vigentes: los `working` en cada poll, los
    // terminados hasta que aparezca el resumen final.
    const latest = latestByTask(tr.value);
    const pending = [...latest.values()]
      .map((x) => findRun(r.value, asIssueRun(x, "")))
      .filter((x): x is RunSummary => x !== undefined)
      .filter((x) => isInProgress(x) || !settled.current.has(x.sessionId));
    const fetched = await Promise.all(
      pending.map(async (x) => {
        try {
          const d = await getRunDetail(x.sessionId, x.cwd ?? "");
          if (!isInProgress(x)) {
            const n = (tries.current.get(x.sessionId) ?? 0) + 1;
            tries.current.set(x.sessionId, n);
            if (d === null || d.source === "final" || n >= SETTLE_TRIES) settled.current.add(x.sessionId);
          }
          return [x.sessionId, d] as const;
        } catch {
          return null;
        }
      }),
    );
    const updates = fetched.filter((x) => x !== null);
    if (updates.length) setDetails((prev) => ({ ...prev, ...Object.fromEntries(updates) }));
  }, []);

  const refresh = useCallback((): Promise<void> => {
    if (inFlight.current) {
      again.current = true;
      return inFlight.current;
    }
    const run = (async () => {
      try {
        do {
          again.current = false;
          await pollOnce();
        } while (again.current);
      } finally {
        inFlight.current = null;
      }
    })();
    inFlight.current = run;
    return run;
  }, [pollOnce]);

  useEffect(() => {
    if (!enabled) return;
    let timer: ReturnType<typeof setTimeout>;
    let cancelled = false;
    const loop = async () => {
      await refresh();
      if (!cancelled) timer = setTimeout(loop, POLL_MS);
    };
    void loop();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [enabled, refresh, repoPath]);

  const { current, byTask } = useMemo(() => {
    const titles = new Map(tasks.map((t) => [t.id, t.title]));
    const latest = latestByTask(taskRuns);
    const queued = taskRuns.filter((x) => x.status === "queued").sort((a, b) => a.queuedAt - b.queuedAt);
    const byTask = new Map<string, RunView[]>();
    const current = new Map<string, RunView>();
    for (const tr of [...taskRuns].sort((a, b) => b.queuedAt - a.queuedAt)) {
      const title = titles.get(tr.taskId);
      if (title === undefined) continue;
      const run = findRun(runs, asIssueRun(tr, title));
      const pos = queued.indexOf(tr);
      const isCurrent = latest.get(tr.taskId) === tr;
      const v = viewOfTaskRun(tr, title, run, run ? details[run.sessionId] : undefined, pos >= 0 ? pos + 1 : null, isCurrent);
      byTask.set(tr.taskId, [...(byTask.get(tr.taskId) ?? []), v]);
      if (isCurrent) current.set(tr.taskId, v);
    }
    return { current, byTask };
  }, [tasks, taskRuns, runs, details]);

  const historyOf = useCallback((taskId: string) => byTask.get(taskId) ?? [], [byTask]);

  return { tasks, current, historyOf, error, loading, refresh };
}

/** Misma ruta salvo barras finales (las de tareas vienen canonicalizadas, las de config no). */
export const samePath = (a: string, b: string) => a.replace(/\/+$/, "") === b.replace(/\/+$/, "");

function latestByTask(all: TaskRun[]): Map<string, TaskRun> {
  const latest = new Map<string, TaskRun>();
  for (const tr of all) {
    const prev = latest.get(tr.taskId);
    if (!prev || tr.queuedAt > prev.queuedAt) latest.set(tr.taskId, tr);
  }
  return latest;
}
