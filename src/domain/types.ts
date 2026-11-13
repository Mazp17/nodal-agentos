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
  /** Canonical folder holding the project's repos. Repo-less chats run there. */
  rootPath: string | null;
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
  position: number;
  createdAt: number;
}

// ---------- Chats ----------

/** A `claude -p` session scoped to a project, or narrowed to one of its repos. */
export interface Chat extends LaunchOptions {
  id: string;
  projectId: string;
  /** The Repo pill. `null` → the whole project; it runs in the project's first repo. */
  repoId: string | null;
  /** `null` until the first message names it. */
  title: string | null;
  /** Claude Code's name for the session (`/rename`, else its AI title); shown over `title`. */
  sessionTitle: string | null;
  /** Claude Code's session id, known after the first message. */
  sessionId: string | null;
  /** Agent the chat talks to (`--agent`); `null` is plain Claude. */
  agent: string | null;
  /** Task this chat works (in the task's worktree); `null` for a free chat. */
  taskId: string | null;
  /** A worktree a free chat was opened in (its path); `null` → the repo's checkout. */
  worktree: string | null;
  createdAt: number;
  /** Last message sent: the sessions list is ordered by it. */
  updatedAt: number;
}

// ---------- Executors and options ----------

export type AgentSource = "user" | "repo" | "plugin";

export type Executor =
  | { kind: "agent"; name: string; source: AgentSource }
  | { kind: "workflow"; name: string }
  | { kind: "claude" };

/** An agent or workflow a project hides from its pickers; plain Claude can't be hidden. */
export interface HiddenExecutor {
  kind: "agent" | "workflow";
  source: AgentSource;
  name: string;
  /** Only for `source: "repo"`: the same repo agent in two repos is two entries. */
  repoId: string | null;
}

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
  /** `code`, `cursor`... `null` → the system's. */
  editor: string | null;
  /** Intro of the chat agent's system prompt. `null` → the built-in one. */
  chatSystemPrompt: string | null;
}
