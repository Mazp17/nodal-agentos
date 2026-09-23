import { invoke } from "@tauri-apps/api/core";
import type { IssueRun, RunDetail, RunRef, RunSummary, WorkflowInfo } from "./types";

export const launchRun = (cwd: string, prompt: string) => invoke<RunRef>("launch_run", { cwd, prompt });

export const listRuns = () => invoke<RunSummary[]>("list_runs");

/** `null` si la sesión no tiene carpeta todavía o no lanzó ningún workflow. */
export const getRunDetail = (sessionId: string, cwd: string) =>
  invoke<RunDetail | null>("get_run_detail", { sessionId, cwd });

/** ~/.claude/workflows más <repo>/.claude/workflows si se pasa repo. */
export const listWorkflows = (repoPath: string | null) =>
  invoke<WorkflowInfo[]>("list_workflows", { repoPath });

export interface LaunchIssueArgs {
  issueId: string;
  identifier: string;
  teamId: string;
  projectId: string | null;
  workflow: string;
}
/** Encola o lanza; rechaza con un string en español. */
export const launchIssueRun = (args: LaunchIssueArgs) => invoke<IssueRun>("launch_issue_run", { ...args });

/** Historial, más recientes primero; puede haber varios por issue. */
export const listIssueRuns = () => invoke<IssueRun[]>("list_issue_runs");

export const cancelQueued = (issueId: string) => invoke<void>("cancel_queued", { issueId });

/** Abre Terminal.app con `claude attach <id>`. */
export const attachRun = (runId: string) => invoke<void>("attach_run", { runId });

export const stopRun = (runId: string) => invoke<void>("stop_run", { runId });
