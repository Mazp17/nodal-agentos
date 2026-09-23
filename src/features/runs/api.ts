import { invoke } from "@tauri-apps/api/core";
import type { RepoMapping } from "../linear/api";
import type { IssueRun, LaunchBlocker, RunDetail, RunRef, RunSummary, Transcript, WorkflowInfo } from "./types";

/** Usa el model/effort/permission mode del repo si `cwd` está mapeado. */
export const launchRun = (cwd: string, prompt: string) => invoke<RunRef>("launch_run", { cwd, prompt });

export const listRuns = () => invoke<RunSummary[]>("list_runs");

/** `null` si la sesión no tiene carpeta todavía o no lanzó ningún workflow. */
export const getRunDetail = (sessionId: string, cwd: string) =>
  invoke<RunDetail | null>("get_run_detail", { sessionId, cwd });

/**
 * Transcript de un subagente. `runId` es el id del workflow (`wf_...`). Devuelve como
 * mucho los últimos `limit` items (default 200). `null` si el agente aún no tiene archivo.
 */
export const getAgentTranscript = (sessionId: string, cwd: string, runId: string, agentId: string, limit?: number) =>
  invoke<Transcript | null>("get_agent_transcript", { sessionId, cwd, runId, agentId, limit: limit ?? null });

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
/** Encola o lanza; rechaza con un string. */
export const launchIssueRun = (args: LaunchIssueArgs) => invoke<IssueRun>("launch_issue_run", { ...args });

/** Historial, más recientes primero; puede haber varios por issue. */
export const listIssueRuns = () => invoke<IssueRun[]>("list_issue_runs");

export const cancelQueued = (issueId: string) => invoke<void>("cancel_queued", { issueId });

/** `null` salvo que la sesión haya terminado sin correr su workflow por un motivo conocido. */
export const getLaunchBlocker = (sessionId: string, cwd: string) =>
  invoke<LaunchBlocker | null>("get_launch_blocker", { sessionId, cwd });

/** Abre Terminal.app en `path`; con `runClaude`, arranca `claude` ahí. */
export const openTerminalAt = (path: string, runClaude = false) =>
  invoke<void>("open_terminal_at", { path, runClaude });

/** Abre Terminal.app con `claude attach <id>`. */
export const attachRun = (runId: string) => invoke<void>("attach_run", { runId });

export const stopRun = (runId: string) => invoke<void>("stop_run", { runId });

/** Raíz del repo git que contiene `path`; `null` si no está en un repo. */
export const resolveGitRoot = (path: string) => invoke<string | null>("resolve_git_root", { path });

/** Mapeo de un repo por su ruta. */
export const resolveRepoConfig = (path: string) => invoke<RepoMapping | null>("resolve_repo_config", { path });
