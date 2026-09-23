// Espejo de src-tauri/src/activity/mod.rs (serde rename_all = camelCase).

export type SessionKind = "interactive" | "background" | "unlisted" | (string & {});

export interface SessionActivity {
  sessionId: string;
  /** Id corto (solo background). */
  id: string | null;
  /** "unlisted": transcript activo que `claude agents` no lista (`claude -p`, SDK…). */
  kind: SessionKind;
  name: string | null;
  cwd: string | null;
  /** "busy" | "idle" | "waiting" (procesos vivos). */
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
  description: string | null;
  agentType: string | null;
  cwd: string | null;
  worktree: string | null;
  parentAgentId: string | null;
  workflowId: string | null;
  workflowPhase: string | null;
  model: string | null;
  active: boolean;
  /** Terminó con end_turn. `!active && !finished` = sin actividad reciente. */
  finished: boolean;
  lastActivityAt: number | null;
  lastTool: string | null;
  lastToolSummary: string | null;
  sessionName: string | null;
  sessionKind: SessionKind;
  sessionCwd: string | null;
  sessionIsAppRun: boolean;
}

export interface RepoActivity {
  repoPath: string;
  sessions: SessionActivity[];
  subagents: SubagentActivity[];
  generatedAt: number;
}

export interface ActivitySummary {
  /** Sesiones vivas trabajando o esperando al usuario. */
  sessions: number;
  /** Subagentes activos. */
  agents: number;
  generatedAt: number;
}
