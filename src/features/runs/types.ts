// Mirror of src-tauri/src/runs/types.rs (serde rename_all = camelCase): what Claude Code
// reports about a background session (`claude agents`), its workflow and its subagents. The
// Nodal run (`Run`) lives in `src/domain/types.ts`.

/** A background session as reported by `claude agents --json --all`. */
export interface RunSummary {
  /** Short id from `claude --bg` (`Run.claudeRunId`). */
  id: string;
  sessionId: string;
  cwd: string | null;
  name: string | null;
  /** Epoch in ms. */
  startedAt: number | null;
  pid: number | null;
  /** "busy" | "idle" | "waiting" (live sessions only). */
  status: string | null;
  /** "working" | "blocked" | "done" | "failed" | "stopped". */
  state: string | null;
  /** With status "waiting": "permission prompt" | "input needed" | "sandbox request" | ... */
  waitingFor: string | null;
}

/** Working or blocked waiting on the user: the session is still alive. */
export const isInProgress = (r: RunSummary | null | undefined) => r?.state === "working" || r?.state === "blocked";

/** Known fields of the workflow's `result`; all optional. */
export interface RunResult {
  issue: string | null;
  /** The PR's http(s) URL. */
  pr: string | null;
  branch: string | null;
  workdir: string | null;
  where: string | null;
  unmetAcceptance: string[] | null;
  nits: string[] | null;
  /** Full `result` as indented JSON (truncated). */
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
  /** Older items that weren't returned. */
  omitted: number;
  finalOutput: string | null;
  /** Very large file: only the head and tail were read. */
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
  /** "completed", ... `null` in live mode: cross-check with RunSummary.state. */
  status: string | null;
  phases: PhaseInfo[];
  currentPhase: string | null;
  /** 1-based within `phases`. */
  currentPhaseIndex: number | null;
  agents: AgentInfo[];
  agentCount: number;
  totalTokens: number | null;
  totalToolCalls: number | null;
  durationMs: number | null;
  /** "green" | "yellow" | "red". */
  resultStatus: string | null;
  /** Final mode only. */
  result: RunResult | null;
  workflowCount: number;
}

/** Why a background session ended without running its workflow (`get_launch_blocker`). */
export type LaunchBlocker = { kind: "workflowReview"; workflow: string | null };
