import { viewOfIssueRun, type RunView } from "../runs/status";
import type { IssueRun, RunDetail, RunSummary } from "../runs/types";
import type { TaskRun } from "./types";

export const TASK_WORKFLOW = "plan-task";

export const taskRunKey = (tr: TaskRun) => `t:${tr.taskId}:${tr.queuedAt}`;

/**
 * Un `TaskRun` tiene la misma forma de cola que un `IssueRun`: se adapta para reusar
 * toda la derivación de estado de `runs/status.ts` (badges, fases, resultado).
 */
export function asIssueRun(tr: TaskRun, title: string): IssueRun {
  return {
    issueId: tr.taskId,
    identifier: title,
    workflow: TASK_WORKFLOW,
    runId: tr.runId,
    sessionId: tr.sessionId,
    cwd: tr.cwd,
    queuedAt: tr.queuedAt,
    launchedAt: tr.launchedAt,
    status: tr.status,
    error: tr.error,
  };
}

/** `ir`/`issueId` quedan en null: las acciones de issue (cancel) no aplican a tareas. */
export function viewOfTaskRun(
  tr: TaskRun,
  title: string,
  run: RunSummary | undefined,
  detail: RunDetail | null | undefined,
  queuePos: number | null,
  current: boolean,
): RunView {
  const v = viewOfIssueRun(asIssueRun(tr, title), run, detail, queuePos, current);
  return { ...v, key: taskRunKey(tr), ir: null, issueId: null, identifier: title };
}
