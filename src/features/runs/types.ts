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
  /** "busy" | "idle" | "waiting" (solo sesiones vivas). */
  status: string | null;
  /** "working" | "blocked" | "done" | "failed" | "stopped". */
  state: string | null;
  /** Con status "waiting": "permission prompt" | "input needed" | "sandbox request" | ... */
  waitingFor: string | null;
}

/** Trabajando o bloqueada esperando al usuario: la sesión sigue viva. */
export const isInProgress = (r: RunSummary | undefined) => r?.state === "working" || r?.state === "blocked";

/** Flags de `claude --bg` por repo. */
export interface LaunchOptions {
  model?: string;
  effort?: string;
  permissionMode?: string;
}

/** Campos conocidos del `result` del workflow (forma de linear-issue); todos opcionales. */
export interface RunResult {
  issue: string | null;
  /** URL http(s) del PR. */
  pr: string | null;
  branch: string | null;
  workdir: string | null;
  where: string | null;
  unmetAcceptance: string[] | null;
  nits: string[] | null;
  /** `result` completo como JSON indentado (recortado). */
  raw: string | null;
}

export interface ToolResultInfo {
  text: string;
  isError: boolean;
  truncated: boolean;
}

export type TranscriptItem =
  | { kind: "text"; text: string; truncated: boolean }
  | { kind: "thinking"; text: string; truncated: boolean }
  | { kind: "user"; text: string; truncated: boolean }
  | {
      kind: "toolUse";
      id: string | null;
      name: string;
      summary: string | null;
      input: string | null;
      result: ToolResultInfo | null;
    };

export interface Transcript {
  agentId: string;
  label: string | null;
  model: string | null;
  phase: string | null;
  prompt: string | null;
  items: TranscriptItem[];
  totalItems: number;
  /** Items más viejos que no se devolvieron. */
  omitted: number;
  finalOutput: string | null;
  /** Archivo muy grande: se leyó solo principio y cola. */
  partial: boolean;
  bytes: number;
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
  /** Solo en modo final. */
  result: RunResult | null;
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
  /** Flags del repo fijados al encolar (ausente si no hay ninguno). */
  options?: LaunchOptions;
}

/** Por qué una sesión en background terminó sin correr su workflow (`get_launch_blocker`). */
export type LaunchBlocker = { kind: "workflowReview"; workflow: string | null };
