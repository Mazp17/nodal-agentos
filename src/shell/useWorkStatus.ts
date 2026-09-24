// Estado global de trabajo para el shell: contadores del sidebar y búsqueda de la paleta.
// Solo deriva de los stores compartidos (tareas y runs): no consulta nada por su cuenta. La
// píldora del topbar usa `useQueueSummary` de runs.

import { useMemo } from "react";
import { isRunActive, projectIdOf, useAllRuns, useRunsSnapshot } from "../domain/hooks/runs";
import { useRepoList, useTaskList } from "../domain/hooks/store";
import type { RunLight, Task } from "../domain/types";

const RUNNING: ReadonlySet<RunLight["status"]> = new Set(["launching", "launched"]);
const NONE: never[] = [];

export interface WorkStatus {
  /** Error de la última lectura de tareas o runs (base de datos). */
  error: string | null;
  /** Error de `claude agents` (CLI), en un banner aparte. */
  cliError: string | null;
  tasks: Task[];
  /** Tareas bloqueadas (revisión fallida, agente bloqueado, run detenido). */
  blocked: Task[];
  /** Tareas abiertas (ni Done ni Canceled). */
  openTotal: number;
  /** Runs `launching`/`launched` en total y por proyecto. */
  activeTotal: number;
  activeByProject: Map<string, number>;
  /** Tareas con un run en cola o en marcha (la paleta no ofrece "Run X" para ellas). */
  activeTaskIds: ReadonlySet<string>;
}

export function useWorkStatus(): WorkStatus {
  const tasksQ = useTaskList();
  const reposQ = useRepoList();
  const runsQ = useAllRuns();
  const tasks = tasksQ.data ?? NONE;
  const runs = runsQ.data ?? NONE;
  const error = tasksQ.error ?? runsQ.error;
  const cliError = useRunsSnapshot().liveError;

  return useMemo((): WorkStatus => {
    const taskById = new Map(tasks.map((t) => [t.id, t]));
    const repoById = new Map((reposQ.data ?? []).map((r) => [r.id, r]));
    let openTotal = 0;
    for (const t of tasks) if (t.status !== "done" && t.status !== "canceled") openTotal++;
    const activeByProject = new Map<string, number>();
    const activeTaskIds = new Set<string>();
    let activeTotal = 0;
    for (const r of runs) {
      if (!isRunActive(r)) continue;
      if (r.taskId) activeTaskIds.add(r.taskId);
      if (!RUNNING.has(r.status)) continue;
      activeTotal++;
      // Los runs sin tarea cuentan en el proyecto de su repo.
      const p = projectIdOf(r, taskById, repoById);
      if (p) activeByProject.set(p, (activeByProject.get(p) ?? 0) + 1);
    }
    return {
      error,
      cliError,
      tasks,
      blocked: tasks.filter((t) => t.status === "blocked"),
      openTotal,
      activeTotal,
      activeByProject,
      activeTaskIds,
    };
  }, [tasks, runs, reposQ.data, error, cliError]);
}
