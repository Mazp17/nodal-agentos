// `runs/**` commands that read Claude Code's state (not Nodal's database). The queue and
// database ones (`list_task_runs`, `cancel_run`, `run_diff`...) are in `src/domain/api.ts`.

import { invoke, type Channel } from "@tauri-apps/api/core";
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

/**
 * Starts `claude attach <runId>` in a pseudo-terminal of `cols`×`rows` and returns its session
 * id. The PTY's raw output arrives on `onData`; `onExit` fires once, with the exit code (or
 * `null`), when the attach process ends. Closing only detaches: the run keeps going.
 */
export const ptyAttach = (
  runId: string,
  cwd: string | null,
  cols: number,
  rows: number,
  onData: Channel<ArrayBuffer>,
  onExit: Channel<number | null>,
) => invoke<number>("pty_attach", { runId, cwd, cols, rows, onData, onExit });

/** Starts an interactive `claude` in `dir` in a PTY (to accept its trust dialog in-app). Same contract as `ptyAttach`. */
export const ptyOpenClaude = (
  dir: string,
  cols: number,
  rows: number,
  onData: Channel<ArrayBuffer>,
  onExit: Channel<number | null>,
) => invoke<number>("pty_open_claude", { dir, cols, rows, onData, onExit });

/** Writes raw bytes (keystrokes, pastes, mouse reports) to the PTY. */
export const ptyWrite = (session: number, data: Uint8Array) => invoke<void>("pty_write", { session, data: Array.from(data) });

/** Resizes the PTY; the TUI redraws for the new size. */
export const ptyResize = (session: number, cols: number, rows: number) => invoke<void>("pty_resize", { session, cols, rows });

/** Detaches (kills `claude attach`, not the run). Idempotent. */
export const ptyClose = (session: number) => invoke<void>("pty_close", { session });

/** Root of the git repo containing `path`; `null` if it isn't in a repo. */
export const resolveGitRoot = (path: string) => invoke<string | null>("resolve_git_root", { path });
