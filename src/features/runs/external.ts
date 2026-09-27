// Claude Code sessions started outside Nodal, listed on Runs next to Nodal's runs.
// Types mirror src-tauri/src/activity/mod.rs (serde rename_all = camelCase).

import { invoke } from "@tauri-apps/api/core";
import { POLL, usePolled } from "../../domain/hooks/store";

export type SessionKind = "interactive" | "background" | "unlisted" | (string & {});

export interface SessionActivity {
  sessionId: string;
  /** Short id (background only). */
  id: string | null;
  /** "unlisted": active transcript that `claude agents` doesn't list (`claude -p`, SDK…). */
  kind: SessionKind;
  name: string | null;
  cwd: string | null;
  /** "busy" | "idle" | "waiting" (live processes). */
  status: string | null;
  /** "working" | "blocked" | "done" | "stopped" (background). */
  state: string | null;
  waitingFor: string | null;
  startedAt: number | null;
  pid: number | null;
  alive: boolean;
  isAppRun: boolean;
  lastActivityAt: number | null;
  lastTool: string | null;
  lastToolSummary: string | null;
  entrypoint: string | null;
}

export interface SubagentActivity {
  sessionId: string;
  agentId: string;
  active: boolean;
}

/** Each session appears once, in the deepest repo that contains it. */
export interface RepoSessions {
  repoId: string;
  sessions: SessionActivity[];
  subagents: SubagentActivity[];
}

export interface ExternalSessions {
  repos: RepoSessions[];
  generatedAt: number;
}

/** Sessions not launched by Nodal in the project's repos (`null`: every project), from a single `claude agents`. */
export const externalSessions = (projectId: string | null) => invoke<ExternalSessions>("external_sessions", { projectId });

export function useExternalSessions(projectId: string | null) {
  return usePolled<ExternalSessions>(`external-sessions:${projectId ?? "all"}`, () => externalSessions(projectId), ["repos"], POLL.live);
}
