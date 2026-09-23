// Espejo de src-tauri/src/tasks/store.rs (serde rename_all = camelCase).

export type TaskStatus = "todo" | "done";

/** El texto vive en `<app_data_dir>/tasks/<id>/plan.md`; se lee con `readTaskPlan`. */
export type PlanRef = { kind: "text" } | { kind: "file"; path: string };

/** Lo que se manda al crear/editar. `path` absoluta o relativa al repo. */
export type PlanInput = { kind: "text"; text: string } | { kind: "file"; path: string };

export interface Task {
  id: string;
  /** Canonicalizada por el backend. */
  repoPath: string;
  title: string;
  plan: PlanRef;
  status: TaskStatus;
  /** Epoch ms. */
  createdAt: number;
  doneAt: number | null;
}

export type TaskRunStatus = "queued" | "launching" | "launched" | "failed";

export type FinishMode = "pr" | "branch";

export interface TaskRun {
  taskId: string;
  /** `/plan-task {"plan":…,"title":…,"finish":…}`. */
  prompt: string;
  finish: FinishMode;
  runId: string | null;
  sessionId: string | null;
  cwd: string;
  queuedAt: number;
  launchedAt: number | null;
  status: TaskRunStatus;
  error: string | null;
}
