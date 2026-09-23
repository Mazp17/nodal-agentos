import { invoke } from "@tauri-apps/api/core";
import type { FinishMode, PlanInput, Task, TaskRun } from "./types";

// Todos rechazan con un string (en inglés) listo para mostrar.

export const createTask = (repoPath: string, title: string, plan: PlanInput) =>
  invoke<Task>("create_task", { repoPath, title, plan });

/** El repo no se cambia. `null` deja el campo como está. */
export const updateTask = (id: string, title: string | null, plan: PlanInput | null) =>
  invoke<Task>("update_task", { id, title, plan });

export const deleteTask = (id: string) => invoke<void>("delete_task", { id });

/** Más recientes primero; con `repoPath`, solo las de ese repo. */
export const listTasks = (repoPath: string | null) => invoke<Task[]>("list_tasks", { repoPath });

export const setTaskDone = (id: string, done: boolean) => invoke<Task>("set_task_done", { id, done });

export const readTaskPlan = (id: string) => invoke<string>("read_task_plan", { id });

/** Encola (o lanza, si hay slot) `/plan-task` para la tarea. */
export const launchTaskRun = (id: string, finish: FinishMode | null) =>
  invoke<TaskRun>("launch_task_run", { id, finish });

/** Historial, más recientes primero. */
export const listTaskRuns = (taskId: string | null) => invoke<TaskRun[]>("list_task_runs", { taskId });

export const cancelTaskRun = (taskId: string) => invoke<void>("cancel_task_run", { taskId });
