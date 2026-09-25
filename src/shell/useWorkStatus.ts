// Global work status for the shell: sidebar counters and palette search.
// Derived only from the shared stores (tasks and runs): it queries nothing on its own. The
// topbar pill uses `useQueueSummary` from runs.

import { useMemo } from "react";
import { isRunActive, projectIdOf, useAllRuns, useRunsSnapshot } from "../domain/hooks/runs";
import { useRepoList, useTaskList } from "../domain/hooks/store";
import type { RunLight, Task } from "../domain/types";

const RUNNING: ReadonlySet<RunLight["status"]> = new Set(["launching", "launched"]);
const NONE: never[] = [];

export interface WorkStatus {
  /** Error from the last read of tasks or runs (database). */
  error: string | null;
  /** Error from `claude agents` (CLI), shown in a separate banner. */
  cliError: string | null;
  tasks: Task[];
  /** Blocked tasks (failed review, blocked agent, stopped run). */
  blocked: Task[];
  /** Open tasks (neither Done nor Canceled). */
  openTotal: number;
  /** `launching`/`launched` runs, in total and per project. */
  activeTotal: number;
  activeByProject: Map<string, number>;
  /** Tasks with a queued or running run (the palette doesn't offer "Run X" for them). */
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
      // Runs without a task count toward their repo's project.
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
