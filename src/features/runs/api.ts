// `runs/**` commands that read Claude Code's state (not Nodal's database). The queue and
// database ones (`list_task_runs`, `cancel_run`, `run_diff`...) are in `src/domain/api.ts`.

import { invoke } from "@tauri-apps/api/core";
import type { LaunchBlocker, RunDetail, RunSummary, Transcript } from "./types";

/** Background sessions from `claude agents` (ours and others'). */
export const listRuns = () => invoke<RunSummary[]>("list_runs");

/** `null` if the session has no folder yet or hasn't launched any workflow. */
export const getRunDetail = (sessionId: string, cwd: string) =>
  invoke<RunDetail | null>("get_run_detail", { sessionId, cwd });

/**
 * Transcript of a workflow subagent. `workflowId` is the `wf_...` id. Returns at most
 * the last `limit` items (default 200). `null` if the agent has no file yet.
 */
export const getAgentTranscript = (sessionId: string, cwd: string, workflowId: string, agentId: string, limit?: number) =>
  invoke<Transcript | null>("get_agent_transcript", { sessionId, cwd, runId: workflowId, agentId, limit: limit ?? null });

/** `null` unless the session ended without running its workflow for a known reason. */
export const getLaunchBlocker = (sessionId: string, cwd: string) =>
  invoke<LaunchBlocker | null>("get_launch_blocker", { sessionId, cwd });

/** Opens Terminal.app at `path`; with `runClaude`, starts `claude` there. */
export const openTerminalAt = (path: string, runClaude = false) => invoke<void>("open_terminal_at", { path, runClaude });

/** Opens Terminal.app with `claude attach <id>` (`Run.claudeRunId`). */
export const attachRun = (claudeRunId: string) => invoke<void>("attach_run", { runId: claudeRunId });

/** Root of the git repo containing `path`; `null` if it isn't in a repo. */
export const resolveGitRoot = (path: string) => invoke<string | null>("resolve_git_root", { path });
