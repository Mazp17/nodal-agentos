// Exact mirror of `src-tauri/src/domain/mod.rs`. Any change goes on both sides.
// Conventions: camelCase fields, snake_case value enums, data-carrying enums
// discriminated by `kind`, dates in epoch ms. Rust's `Option<T>` arrives as `T | null`
// (except `LaunchOptions`, whose fields are omitted when empty).

// ---------- Projects and repos ----------

export interface Project {
  id: string;
  name: string;
  /** Prefix for task ids (`PAY` → `PAY-1`). */
  key: string;
  nextTaskNumber: number;
  color: string;
  description: string | null;
  /** `null` → `Settings.defaultExecutor` → Claude. */
  defaultExecutor: Executor | null;
  /** `null` → `code-reviewer`. */
  reviewer: string | null;
  createdAt: number;
  archivedAt: number | null;
}

/** Optional `claude --bg` flags. */
export interface LaunchOptions {
  model?: string;
  effort?: string;
  permissionMode?: string;
}

/** `model`, `effort` and `permissionMode` are flat (flatten in Rust). */
export interface Repo extends LaunchOptions {
  id: string;
  projectId: string;
  /** Canonical git root. */
  path: string;
  name: string;
  defaultExecutor: Executor | null;
  defaultIsolation: Isolation;
  defaultFinish: Finish;
  defaultReview: boolean;
  reviewer: string | null;
  position: number;
  createdAt: number;
}

// ---------- Executors and options ----------

export type AgentSource = "user" | "repo" | "plugin";

export type Executor =
  | { kind: "agent"; name: string; source: AgentSource }
  | { kind: "workflow"; name: string }
  | { kind: "claude" };

export type Isolation = "worktree" | "in_place";

/** `changes`: uncommitted · `commit`: commit without push · `pr`: commit, push and PR. */
export type Finish = "changes" | "commit" | "pr";

// ---------- Tasks ----------

export type TaskStatus = "backlog" | "todo" | "in_progress" | "in_review" | "blocked" | "done" | "canceled";

export const TASK_STATUSES: readonly TaskStatus[] = [
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "blocked",
  "done",
  "canceled",
];

export type Priority = "urgent" | "high" | "medium" | "low" | "none";

export type PlanRef = { kind: "text" } | { kind: "file"; path: string };

export interface WorktreeRef {
  path: string;
  branch: string;
  base: string;
}

export type ExtKind = "triage" | "backlog" | "unstarted" | "started" | "completed" | "canceled" | "unknown";

export interface ExternalState {
  id: string;
  name: string;
  kind: ExtKind;
  color: string | null;
}

export interface TaskSource {
  provider: string;
  /** To delete the link, its tasks are unlinked first. */
  linkId: string | null;
  externalId: string;
  /** The provider's human-readable id (`ENG-142`). */
  identifier: string;
  url: string;
  externalState: ExternalState | null;
  lastSyncedAt: number | null;
  syncError: string | null;
  /** The current external state isn't in the pull mapping ("unmapped external state"). */
  unmapped: boolean;
  /** Provider project (Linear project) seen in the last import/pull. */
  project: ExtProject | null;
  /** Project rule through which it reached its repo. */
  ruleId: string | null;
  /** The issue changed project in the provider: pending a decision (`resolveMovedTask`). */
  moved: MovedInfo | null;
}

export interface ExtProject {
  id: string;
  name: string;
}

export interface MovedInfo {
  fromProject: ExtProject;
  /** `null`: it was left without a project. */
  toProject: ExtProject | null;
  /** Repo it would get by the rules; `null` if none applies. */
  suggestedRepoId: string | null;
}

export interface Task {
  id: string;
  projectId: string;
  repoId: string;
  /** The visible id is `taskKey(project.key, number)`. */
  number: number;
  title: string;
  status: TaskStatus;
  priority: Priority;
  labels: string[];
  position: number;
  plan: PlanRef;
  planOverridden: boolean;
  acceptance: string[];
  /** `null` → repo default → project default → Claude. */
  assignee: Executor | null;
  isolation: Isolation | null;
  finish: Finish | null;
  review: boolean | null;
  worktree: WorktreeRef | null;
  source: TaskSource | null;
  createdAt: number;
  updatedAt: number;
  closedAt: number | null;
}

export const taskKey = (projectKey: string, number: number) => `${projectKey}-${number}`;

/** `blocks`: `taskId` blocks `otherId`. */
export type RelationKind = "related" | "blocks";

export interface TaskRelation {
  taskId: string;
  otherId: string;
  kind: RelationKind;
}

// ---------- Runs ----------

export type RunKind = "work" | "review";

export type RunStatus = "queued" | "launching" | "launched" | "finished" | "failed" | "canceled";

export type RunOutcome = "green" | "yellow" | "red" | "stopped" | "unknown";

export interface Verdict {
  pass: boolean;
  unmet: string[];
  nits: string[];
  summary: string | null;
}

export interface Run {
  id: string;
  taskId: string | null;
  repoId: string | null;
  cwd: string;
  executor: Executor;
  kind: RunKind;
  parentRunId: string | null;
  prompt: string;
  extraInstructions: string | null;
  options: LaunchOptions;
  finish: Finish;
  /** Resolved when queued; `null` for workflows. */
  isolation: Isolation | null;
  /** Resolved when queued: when it finishes, the reviewer is queued. */
  review: boolean;
  verdict: Verdict | null;
  status: RunStatus;
  queuePosition: number;
  claudeRunId: string | null;
  sessionId: string | null;
  queuedAt: number;
  launchedAt: number | null;
  finishedAt: number | null;
  outcome: RunOutcome | null;
  summary: string | null;
  prUrl: string | null;
  branch: string | null;
  error: string | null;
  legacyLabel: string | null;
  /** Transcript tokens (agent/Claude/reviewer), summed on close. */
  tokens: number | null;
}

/** `Run` without `prompt` or `extraInstructions`, for lists. */
export type RunLight = Omit<Run, "prompt" | "extraInstructions">;

// ---------- External sources ----------

export interface ScopeRef {
  kind: string;
  id: string;
  name: string;
}

export type RuleKind = "label" | "project";

/**
 * Routing on import. Precedence: project rule > label rule > `defaultRepoId`.
 * On save, a rule without `id` (or with a different `kind`/`value`) is new: the backend gives it `id`
 * and `createdAt` (it auto-imports from then on; earlier items are brought in by `importRule`).
 */
export interface RepoRule {
  /** `""` on a new rule. */
  id: string;
  kind: RuleKind;
  /** Label, or the provider project's id. */
  value: string;
  /** Display name (the project's; for label rules, the label). */
  name: string;
  repoId: string;
  /** Set by the backend. */
  createdAt: number;
}

export interface StateMap {
  /** `externalState.id` → Nodal status. Absent = unmapped. */
  pull: Record<string, TaskStatus>;
  /** Nodal status → `externalState.id`; `null` = "Don't sync". */
  push: Partial<Record<TaskStatus, string | null>>;
  /** `null` = mapping pending (no push is done). */
  confirmedAt: number | null;
  knownStates: ExternalState[];
}

/** Provider states that changed against `knownStates`; cleared by `saveStateMap`. */
export interface StateChanges {
  added: ExternalState[];
  removed: ExternalState[];
}

export interface SourceLink {
  id: string;
  projectId: string;
  provider: string;
  scope: ScopeRef;
  defaultRepoId: string | null;
  repoRules: RepoRule[];
  stateMap: StateMap;
  autoImport: boolean;
  createdAt: number;
  lastSyncedAt: number | null;
  lastSyncError: string | null;
  /** `null` = nothing pending. */
  pendingStateChanges: StateChanges | null;
}

export type OutboxPayload = { kind: "set_state"; stateId: string } | { kind: "comment"; body: string };

export interface OutboxItem {
  id: number;
  taskId: string;
  provider: string;
  payload: OutboxPayload;
  attempts: number;
  nextAttemptAt: number;
  lastError: string | null;
  createdAt: number;
}

// ---------- Settings ----------

export interface Settings {
  concurrency: number;
  /** `code`, `cursor`... `null` → the system's. */
  editor: string | null;
  reviewer: string;
  /** Last executor fallback: repo → project → this → Claude. */
  defaultExecutor: Executor | null;
}
