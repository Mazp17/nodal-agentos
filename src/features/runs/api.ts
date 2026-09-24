// Comandos de `runs/**` que leen el estado de Claude Code (no la base de Nodal). Los de la
// cola y la base (`list_task_runs`, `cancel_run`, `run_diff`...) están en `src/domain/api.ts`.

import { invoke } from "@tauri-apps/api/core";
import type { LaunchBlocker, RunDetail, RunSummary, Transcript } from "./types";

/** Sesiones en background de `claude agents` (nuestras y ajenas). */
export const listRuns = () => invoke<RunSummary[]>("list_runs");

/** `null` si la sesión no tiene carpeta todavía o no lanzó ningún workflow. */
export const getRunDetail = (sessionId: string, cwd: string) =>
  invoke<RunDetail | null>("get_run_detail", { sessionId, cwd });

/**
 * Transcript de un subagente de workflow. `workflowId` es el id `wf_...`. Devuelve como
 * mucho los últimos `limit` items (default 200). `null` si el agente aún no tiene archivo.
 */
export const getAgentTranscript = (sessionId: string, cwd: string, workflowId: string, agentId: string, limit?: number) =>
  invoke<Transcript | null>("get_agent_transcript", { sessionId, cwd, runId: workflowId, agentId, limit: limit ?? null });

/** `null` salvo que la sesión haya terminado sin correr su workflow por un motivo conocido. */
export const getLaunchBlocker = (sessionId: string, cwd: string) =>
  invoke<LaunchBlocker | null>("get_launch_blocker", { sessionId, cwd });

/** Abre Terminal.app en `path`; con `runClaude`, arranca `claude` ahí. */
export const openTerminalAt = (path: string, runClaude = false) => invoke<void>("open_terminal_at", { path, runClaude });

/** Abre Terminal.app con `claude attach <id>` (`Run.claudeRunId`). */
export const attachRun = (claudeRunId: string) => invoke<void>("attach_run", { runId: claudeRunId });

/** Raíz del repo git que contiene `path`; `null` si no está en un repo. */
export const resolveGitRoot = (path: string) => invoke<string | null>("resolve_git_root", { path });
