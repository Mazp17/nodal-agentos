// Estado global de trabajo para el shell: contadores del sidebar y búsqueda de la paleta.
// Solo deriva de los stores compartidos (tareas y runs): no consulta nada por su cuenta. La
// píldora del topbar usa `useQueueSummary` de runs.

import { useMemo } from "react";
import { isRunActive, useAllRuns } from "../domain/hooks/runs";
import { useTaskList } from "../domain/hooks/store";
import type { Run, Task } from "../domain/types";

const RUNNING: ReadonlySet<Run["status"]> = new Set(["launching", "launched"]);
const NONE: never[] = [];

export interface WorkStatus {
  /** Error de la última lectura de tareas o runs. */
  error: string | null;
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
  const runsQ = useAllRuns();
  const tasks = tasksQ.data ?? NONE;
  const runs = runsQ.data ?? NONE;
  const error = tasksQ.error ?? runsQ.error;

  return useMemo((): WorkStatus => {
    const taskById = new Map(tasks.map((t) => [t.id, t]));
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
      const p = r.taskId ? taskById.get(r.taskId)?.projectId : undefined;
      if (p) activeByProject.set(p, (activeByProject.get(p) ?? 0) + 1);
    }
    return {
      error,
      tasks,
      blocked: tasks.filter((t) => t.status === "blocked"),
      openTotal,
      activeTotal,
      activeByProject,
      activeTaskIds,
    };
  }, [tasks, runs, error]);
}
