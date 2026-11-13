// Global work status for the shell: sidebar counters and palette search.
// Derived only from the shared task store: it queries nothing on its own.

import { useMemo } from "react";
import { useTaskList } from "../domain/hooks/store";
import type { Task } from "../domain/types";

const NONE: never[] = [];

export interface WorkStatus {
  /** Error from the last read of tasks (database). */
  error: string | null;
  tasks: Task[];
  /** Blocked tasks. */
  blocked: Task[];
  /** Open tasks (neither Done nor Canceled). */
  openTotal: number;
}

export function useWorkStatus(): WorkStatus {
  const tasksQ = useTaskList();
  const tasks = tasksQ.data ?? NONE;
  const error = tasksQ.error;

  return useMemo(
    (): WorkStatus => ({
      error,
      tasks,
      blocked: tasks.filter((t) => t.status === "blocked"),
      openTotal: tasks.filter((t) => t.status !== "done" && t.status !== "canceled").length,
    }),
    [tasks, error],
  );
}
