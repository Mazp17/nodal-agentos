// Espejo de src-tauri/src/runs/types.rs (serde rename_all = camelCase).

export interface RunRef {
  id: string;
  cwd: string;
}

export interface RunSummary {
  id: string;
  sessionId: string;
  cwd: string | null;
  name: string | null;
  /** Epoch en ms. */
  startedAt: number | null;
  pid: number | null;
  /** "busy" | "idle" (solo sesiones vivas). */
  status: string | null;
  /** "working" | "done" | "stopped". */
  state: string | null;
}

export type DetailSource = "final" | "live";

export interface PhaseInfo {
  title: string;
  detail: string | null;
}

export type AgentState = "queued" | "running" | "done" | "failed" | "unknown";

export interface AgentInfo {
  agentId: string | null;
  label: string;
  phase: string | null;
  model: string | null;
  state: AgentState;
  tokens: number | null;
  toolCalls: number | null;
  durationMs: number | null;
  lastToolName: string | null;
  lastToolSummary: string | null;
  resultPreview: string | null;
}

export interface RunDetail {
  workflowId: string;
  workflowName: string | null;
  source: DetailSource;
  /** "completed", ... `null` en modo live: cruzar con RunSummary.state. */
  status: string | null;
  phases: PhaseInfo[];
  currentPhase: string | null;
  /** 1-based dentro de `phases`. */
  currentPhaseIndex: number | null;
  agents: AgentInfo[];
  agentCount: number;
  totalTokens: number | null;
  totalToolCalls: number | null;
  durationMs: number | null;
  /** "green" | "yellow" | "red". */
  resultStatus: string | null;
  workflowCount: number;
}

export interface WorkflowInfo {
  name: string;
  description: string | null;
  whenToUse: string | null;
  source: "user" | "repo";
  path: string;
}

export type IssueRunStatus = "queued" | "launching" | "launched" | "failed";

export interface IssueRun {
  issueId: string;
  identifier: string;
  workflow: string;
  /** Id corto de `claude --bg`; `null` mientras está en cola. */
  runId: string | null;
  /** Se completa recién cuando la sesión aparece en `list_runs`. */
  sessionId: string | null;
  cwd: string;
  /** Epoch en ms. */
  queuedAt: number;
  /** Epoch en ms. */
  launchedAt: number | null;
  status: IssueRunStatus;
  /** Motivo si `status === "failed"`. */
  error: string | null;
}
