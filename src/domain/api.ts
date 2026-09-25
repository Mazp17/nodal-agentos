// Signatures for the F1 commands. The backend doesn't expose them yet: each one carries a
// `TODO(F1-X)` with the agent that implements it (A: migration and rebrand, B: queue, CRUD and
// runs, C: providers). Don't use them from the UI until they exist.
//
// All of them reject with an English string ready to display. Tauri maps the camelCase
// arguments to the command's snake_case parameters.
//
// The DTOs in this file (inputs and reports) are a proposed contract: the agent that
// implements the command creates the equivalent Rust struct with serde camelCase.
//
// Patches: absent field = leave as is; `null` = clear. In Rust that requires telling
// "missing" from "null" apart (`Option<Option<T>>` with `#[serde(default, deserialize_with = ...)]`
// or `serde_with::double_option`): a bare `Option<Option<T>>` reads `null` as "missing".

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentSource,
  ExtKind,
  Executor,
  ExternalState,
  Finish,
  Isolation,
  LaunchOptions,
  Priority,
  Project,
  RelationKind,
  Repo,
  RepoRule,
  Run,
  RunLight,
  ScopeRef,
  Settings,
  SourceLink,
  StateMap,
  Task,
  TaskRelation,
  TaskStatus,
} from "./types";
import type { Transcript } from "../features/runs/types";

// ---------- DTOs ----------

/**
 * Claude Code's trust in a folder (read-only from `~/.claude.json`):
 * `repo` = entry for the repo's canonical root (the main one, for worktrees); `parent` =
 * the folder or a parent inside the repo; `notTrusted` = it will show the dialog; `unknown`
 * (with `trusted: null`) = no readable config.
 */
export interface RepoTrust {
  trusted: boolean | null;
  source: "repo" | "parent" | "notTrusted" | "unknown";
  matchedPath: string | null;
}

/**
 * `running`/`capacity`: occupied slots of the global concurrency (the pump's rule). `needYou`:
 * Blocked tasks or tasks moved to another project in the provider + unconfirmed migrated runs + sessions waiting for permission/input, without counting
 * the same task twice. `queued`: ready to go. `pumpError`: error from the queue's last
 * pass (e.g. `claude agents` fails on every tick), or `null`.
 */
export interface WorkSummary {
  running: number;
  capacity: number;
  needYou: number;
  queued: number;
  pumpError: string | null;
}

/** Live sessions working/waiting and active subagents (same criteria as `activity_summary`). */
export interface RepoActivityCount {
  repoId: string;
  repoPath: string;
  sessions: number;
  agents: number;
}

/** Totals without double-counting what falls in nested repos. */
export interface ProjectActivity {
  projectId: string;
  repos: RepoActivityCount[];
  sessions: number;
  agents: number;
  generatedAt: number;
}

/** `exists`: the folder is a live worktree. `ahead`: commits outside the base; `unpushed`: also outside every remote. */
export interface WorktreeStatus {
  exists: boolean;
  branch: string | null;
  base: string | null;
  ahead: number;
  unpushed: number;
  dirty: boolean;
}

/**
 * `key`: 2-6 uppercase letters/digits, unique. `color`: if missing, one from the palette is assigned.
 * Changing the key renumbers the visible ids (`PAY-1` → `WEB-1`) but doesn't rename branches or
 * worktrees already created.
 */
export interface NewProject {
  name: string;
  key: string;
  color?: string;
  description?: string | null;
}

/** Only the present fields change; `null` clears the optional ones. */
export type ProjectPatch = Partial<Pick<Project, "name" | "key" | "color" | "description" | "defaultExecutor" | "reviewer">> & {
  archived?: boolean;
};

/** `path` can be any folder inside the repo: it resolves to the git root. */
export interface NewRepo extends LaunchOptions {
  path: string;
  name?: string;
  defaultExecutor?: Executor | null;
  defaultIsolation?: Isolation;
  defaultFinish?: Finish;
  defaultReview?: boolean;
  reviewer?: string | null;
}

export type RepoPatch = Partial<Omit<NewRepo, "path" | "model" | "effort" | "permissionMode">> & {
  model?: string | null;
  effort?: string | null;
  permissionMode?: string | null;
  position?: number;
};

/** What the UI sends for the plan; the backend stores it as `PlanRef`. */
export type PlanInput = { kind: "text"; text: string } | { kind: "file"; path: string };

export interface NewTask {
  projectId: string;
  repoId: string;
  title: string;
  plan: PlanInput;
  status?: TaskStatus;
  priority?: Priority;
  labels?: string[];
  acceptance?: string[];
  assignee?: Executor | null;
  isolation?: Isolation | null;
  finish?: Finish | null;
  review?: boolean | null;
}

/**
 * `repoId` moves the task to another repo of the same project (no runs in progress and no worktree; a
 * `.md` plan from the old repo requires sending another `plan`).
 * Imported tasks only accept `repoId`, `plan` (becomes `planOverridden`), `status`,
 * `acceptance` and the execution options.
 */
export type TaskPatch = Partial<Omit<NewTask, "projectId">>;

export interface ExecutorInfo {
  executor: Executor;
  /** From the frontmatter (`description`) or the workflow's `meta`. */
  description: string | null;
  /** Agents: `tools` from the frontmatter. */
  tools: string[] | null;
  /** Workflows: they sync the provider on their own (`meta.managesSource`). */
  managesSource: string | null;
  /** Workflows: they review on their own (`meta.reviews`); they skip the gate. */
  reviews: boolean;
  /** Definition file, for agents and workflows. */
  path: string | null;
  source: AgentSource | null;
}

/** Overrides for this run; whatever is missing comes from the task → repo → project. */
export interface LaunchInput {
  executor?: Executor;
  isolation?: Isolation;
  finish?: Finish;
  review?: boolean;
  extraInstructions?: string;
  options?: LaunchOptions;
}

export type DiffFileStatus = "added" | "modified" | "deleted" | "renamed";

export interface DiffLine {
  kind: "context" | "add" | "del";
  text: string;
  oldNo: number | null;
  newNo: number | null;
}

export interface DiffHunk {
  header: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: DiffLine[];
}

export interface FileDiff {
  path: string;
  oldPath: string | null;
  status: DiffFileStatus;
  additions: number;
  deletions: number;
  binary: boolean;
  hunks: DiffHunk[];
}

export interface CommitInfo {
  sha: string;
  shortSha: string;
  subject: string;
  author: string;
  /** Author date, epoch ms. */
  at: number;
}

export interface RunDiff {
  /** Base ref (`git diff <base>...HEAD`). */
  base: string;
  /** The run's branch; `null` with a detached HEAD. */
  branch: string | null;
  /** Commits on the branch that aren't in `base`, newest to oldest (up to 200). */
  commits: CommitInfo[];
  /** The run is still active: the diff may change. */
  live: boolean;
  cwd: string;
  /** Includes uncommitted changes (the run is still active). */
  includesWorkingTree: boolean;
  files: FileDiff[];
  /** Full patch, for "Copy patch". */
  patch: string;
}

export interface ProviderStatus {
  provider: string;
  hasKey: boolean;
  /** The user's name if the key is valid. */
  viewer: string | null;
  error: string | null;
  /** Last 4 characters of the saved key (never the key); `null` without a key or if it's short. */
  keyHint: string | null;
  /** Epoch ms until which sync is paused (rate limit or rejected key); `null` otherwise. */
  pausedUntil: number | null;
  pauseReason: string | null;
}

export interface ImportableItem {
  externalId: string;
  identifier: string;
  title: string;
  url: string;
  state: ExternalState;
  labels: string[];
  /** From `repoRules` or `defaultRepoId`. */
  suggestedRepoId: string | null;
  /** If already imported, its task. */
  taskId: string | null;
}

export interface ImportResult {
  imported: Task[];
  skipped: { externalId: string; reason: string }[];
}

/** Backfill of a project rule (`previewRuleImport`). */
export interface RulePreview {
  /** Items that would be imported into the rule's repo (open + closed in the last 14 days). */
  count: number;
  /** Already imported into the rule's repo. */
  alreadyImported: number;
  /** Already imported into another repo: they stay where they are. */
  inOtherRepos: number;
}

export type MovedAction = "move" | "keep";

export interface SyncReport {
  /** Tasks refreshed from the provider. */
  pulled: number;
  /** Writes to the provider (states and comments). */
  pushed: number;
  /** New tasks from auto-import. */
  imported: number;
  errors: string[];
  /** Notices: new/vanished states, items without a repo for auto-import, discarded pushes. */
  notices: string[];
}

export type MapOrigin = "suggested" | "confirmed" | "unmapped";

export interface SourceStatesReport {
  /** The provider's current states for the scope. */
  states: ExternalState[];
  /** Saved mapping, completed with Nodal's proposal for whatever is missing. */
  proposal: StateMap;
  pullOrigin: Record<string, MapOrigin>;
  pushOrigin: Partial<Record<TaskStatus, MapOrigin>>;
  /** Against `knownStates`: new and vanished states. */
  added: ExternalState[];
  removed: ExternalState[];
}

export interface NewSourceLink {
  projectId: string;
  provider: string;
  scope: ScopeRef;
  defaultRepoId?: string | null;
  repoRules?: RepoRule[];
  autoImport?: boolean;
}

export type SourceLinkPatch = Partial<Pick<SourceLink, "defaultRepoId" | "repoRules" | "autoImport">>;

export interface LegacyImportReport {
  /** Untouched copy of the chosen folder (`legacy-backup-<ts>/`). */
  backupDir: string;
  projects: number;
  repos: number;
  tasks: number;
  runs: number;
  /** Already imported before (idempotency). */
  alreadyImported: number;
  /** Skipped files or records, with the reason. */
  skipped: string[];
}

// ---------- Projects ----------

export const listProjects = (includeArchived = false) => invoke<Project[]>("list_projects", { includeArchived });
export const createProject = (input: NewProject) => invoke<Project>("create_project", { input });
export const updateProject = (id: string, patch: ProjectPatch) => invoke<Project>("update_project", { id, patch });
/** Cascade-deletes repos, tasks and sources. */
export const deleteProject = (id: string) => invoke<void>("delete_project", { id });

// ---------- Repos ----------

/** `null`: all projects. */
export const listRepos = (projectId: string | null) => invoke<Repo[]>("list_repos", { projectId });
/** Rejects if it's not in a git repo or if it's already in another project. */
export const addRepo = (projectId: string, input: NewRepo) => invoke<Repo>("add_repo", { projectId, input });
export const updateRepo = (id: string, patch: RepoPatch) => invoke<Repo>("update_repo", { id, patch });
/** Rejects if the repo has tasks. */
export const deleteRepo = (id: string) => invoke<void>("delete_repo", { id });

// ---------- Tasks ----------

/** `null`: all. */
export const listTasks = (projectId: string | null) => invoke<Task[]>("list_tasks", { projectId });
export const getTask = (id: string) => invoke<Task>("get_task", { id });
export const createTask = (input: NewTask) => invoke<Task>("create_task", { input });
export const updateTask = (id: string, patch: TaskPatch) => invoke<Task>("update_task", { id, patch });
export const deleteTask = (id: string) => invoke<void>("delete_task", { id });
/** Board drag: changes column and/or position. */
export const moveTask = (id: string, status: TaskStatus, position: number) =>
  invoke<Task>("move_task", { id, status, position });
/**
 * New order for a column: renumbers positions in a transaction. All tasks must
 * be in `status` and in the same project; the column's missing ones go after.
 */
export const reorderTasks = (status: TaskStatus, orderedIds: string[]) =>
  invoke<void>("reorder_tasks", { status, orderedIds });
export const readTaskPlan = (id: string) => invoke<string>("read_task_plan", { id });
export const listTaskRelations = (taskId: string) => invoke<TaskRelation[]>("list_task_relations", { taskId });
export const addTaskRelation = (taskId: string, otherId: string, kind: RelationKind) =>
  invoke<void>("add_task_relation", { taskId, otherId, kind });
export const removeTaskRelation = (taskId: string, otherId: string, kind: RelationKind) =>
  invoke<void>("remove_task_relation", { taskId, otherId, kind });
/**
 * Deletes the task's worktree and branch ("Clean up"). Without `force` it rejects if there are
 * unpublished commits (`unpushed`) or uncommitted changes (`dirty`).
 */
export const cleanupWorktree = (taskId: string, force = false) =>
  invoke<Task>("cleanup_worktree", { taskId, force });
/** Absolute path. Doesn't launch anything or write Claude Code's config. */
export const repoTrust = (path: string) => invoke<RepoTrust>("repo_trust", { path });
/** `git version 2.x.y`; rejects if git isn't on the PATH. */
export const gitVersion = () => invoke<string>("git_version");
/** Claude Code activity per repo of the project (a single `claude agents`). */
export const projectActivity = (projectId: string) => invoke<ProjectActivity>("project_activity", { projectId });
/** Everything zero/false/null if the task has no worktree. */
export const worktreeStatus = (taskId: string) => invoke<WorktreeStatus>("worktree_status", { taskId });

// ---------- Executors and runs ----------

/** Agents, workflows and Claude; with `repoId` it adds the repo's own. */
export const listExecutors = (repoId: string | null) => invoke<ExecutorInfo[]>("list_executors", { repoId });
/** The task's runs (or all with `null`), most recent first. */
export const listTaskRuns = (taskId: string | null, projectId: string | null = null) =>
  invoke<Run[]>("list_task_runs", { taskId, projectId });
/** Like `listTaskRuns` (up to 500), without `prompt` or `extraInstructions`. A run without a task counts toward its repo's project. */
export const listRunsLight = (projectId: string | null = null, taskId: string | null = null) =>
  invoke<RunLight[]>("list_runs_light", { projectId, taskId });
/** Full run (with `prompt`). */
export const getRun = (runId: string) => invoke<Run>("get_run", { runId });
/** The latest run of each task, with no history limit. */
export const latestRunsByTask = (projectId: string | null = null) =>
  invoke<RunLight[]>("latest_runs_by_task", { projectId });
/** `null`: global (includes Claude Code sessions outside the app); with a project, only its runs and tasks. */
export const workSummary = (projectId: string | null = null) => invoke<WorkSummary>("work_summary", { projectId });
/** Global queue: `queued` runs, in launch order. */
export const listQueue = () => invoke<Run[]>("list_queue");
/** Queues a work run (or launches it if there's a slot). */
export const launchTask = (taskId: string, input: LaunchInput = {}) => invoke<Run>("launch_task", { taskId, input });
/** Next step of the chain with another executor, on the same branch/worktree. */
export const handOff = (taskId: string, executor: Executor, extraInstructions: string | null = null) =>
  invoke<Run>("hand_off", { taskId, executor, extraInstructions });
/** Launches the reviewer now. `reviewer` null → the configured one. */
export const reviewNow = (taskId: string, reviewer: string | null = null) =>
  invoke<Run>("review_now", { taskId, reviewer });
/** Removes a `queued` run from the queue, or stops a launched one (saves the partial patch). */
export const cancelRun = (runId: string) => invoke<Run>("cancel_run", { runId });
/**
 * A migrated run left in the queue (`status: "queued"` with `legacyLabel`) doesn't launch on its own:
 * this re-queues it as a new run (the old one is cancelled). To discard it, `cancelRun`.
 */
export const confirmRun = (runId: string) => invoke<Run>("confirm_run", { runId });
/** New queue order: ids of every `queued` run. */
export const reorderQueue = (runIds: string[]) => invoke<void>("reorder_queue", { runIds });
export const runDiff = (runId: string) => invoke<RunDiff>("run_diff", { runId });
/**
 * Transcript of an agent, Claude or reviewer run (main session; `agentId` = the run's id).
 * Rejects for workflows (use `getAgentTranscript`). `null` if the session has no file yet.
 * `limit`: most recent items (default 200, max 2000).
 */
export const getRunTranscript = (runId: string, limit?: number) =>
  invoke<Transcript | null>("get_run_transcript", { runId, limit: limit ?? null });
/** Opens the run's folder (or `file` inside it) in the editor from Settings. */
export const openInEditor = (runId: string, file: string | null = null) =>
  invoke<void>("open_in_editor", { runId, file });
/** Opens the run's folder in Finder. */
export const openWorktree = (runId: string) => invoke<void>("open_worktree", { runId });

// ---------- Providers ----------

export const providerStatus = (provider: string) => invoke<ProviderStatus>("provider_status", { provider });
/** Validates the key and saves it in the keychain; `null` deletes it. */
export const providerSetKey = (provider: string, key: string | null) =>
  invoke<ProviderStatus>("provider_set_key", { provider, key });
export const providerClearKey = (provider: string) => invoke<ProviderStatus>("provider_clear_key", { provider });
export const providerScopes = (provider: string) => invoke<ScopeRef[]>("provider_scopes", { provider });
export const listSourceLinks = (projectId: string | null) => invoke<SourceLink[]>("list_source_links", { projectId });
/** Builds the state-mapping proposal; stays "pending" until `saveStateMap`. */
export const createSourceLink = (input: NewSourceLink) => invoke<SourceLink>("create_source_link", { input });
export const updateSourceLink = (id: string, patch: SourceLinkPatch) =>
  invoke<SourceLink>("update_source_link", { id, patch });
/** Disconnect: unlinks its tasks (they become local) and deletes the link, in a transaction. */
export const deleteSourceLink = (id: string) => invoke<void>("delete_source_link", { id });
/** Unlink: the task becomes local. */
export const unlinkTask = (taskId: string) => invoke<Task>("unlink_task", { taskId });
/** Up to 100 items from the scope. `stateKinds` null or empty = open (triage, backlog, unstarted, started). */
export const providerListImportable = (linkId: string, query: string | null = null, stateKinds: ExtKind[] | null = null) =>
  invoke<ImportableItem[]>("provider_list_importable", { linkId, query, stateKinds });
export const importTasks = (projectId: string, linkId: string, items: { externalId: string; repoId: string }[]) =>
  invoke<ImportResult>("import_tasks", { projectId, linkId, items });
/** `null`: every source. */
export const syncNow = (linkId: string | null = null) => invoke<SyncReport>("sync_now", { linkId });
export const sourceStates = (linkId: string) => invoke<SourceStatesReport>("source_states", { linkId });
/** Saves and confirms the mapping (sets `confirmedAt` and `knownStates`). */
export const saveStateMap = (linkId: string, map: StateMap) => invoke<SourceLink>("save_state_map", { linkId, map });
/** Provider projects eligible as a link rule (Linear: its team's active ones). */
export const sourceRuleProjects = (linkId: string) => invoke<ScopeRef[]>("source_rule_projects", { linkId });
/** How many items the backfill of project rule `ruleId` (already saved) would bring in. */
export const previewRuleImport = (linkId: string, ruleId: string) =>
  invoke<RulePreview>("preview_rule_import", { linkId, ruleId });
/** Backfill: imports the project's items into the rule's repo; those already imported into another repo go in `skipped`. */
export const importRule = (linkId: string, ruleId: string) => invoke<ImportResult>("import_rule", { linkId, ruleId });
/** `move`: to the suggested repo (fails with an active run or worktree); `keep`: stays in its repo. */
export const resolveMovedTask = (taskId: string, action: MovedAction) =>
  invoke<Task>("resolve_moved_task", { taskId, action });

// ---------- Migration ----------

/** Copies `folder` to `legacy-backup-<ts>/` and imports it in a transaction. Idempotent. */
export const importLegacyData = (folder: string) => invoke<LegacyImportReport>("import_legacy_data", { folder });

// ---------- Settings ----------

/** Output of `claude --version` (e.g. `2.1.0 (Claude Code)`); rejects if the CLI isn't found. */
export const claudeVersion = () => invoke<string>("claude_version");

/** `false` in debug builds ("Nodal Dev"): they never check for updates. */
export const updatesEnabled = () => invoke<boolean>("updates_enabled");
/** Relaunch after an update was installed. */
export const restartApp = () => invoke<void>("restart_app");

export const getSettings = () => invoke<Settings>("get_settings");
export const setSettings = (settings: Settings) => invoke<Settings>("set_settings", { settings });

// ---------- Events ----------

export type ChangedKind = "tasks" | "runs" | "queue" | "sources" | "projects";

/** Payload of `nodal://changed`. Without `projectId`: it may affect any project. */
export interface ChangedEvent {
  kind: ChangedKind;
  projectId?: string;
}

/**
 * Notifies when something changed in the backend (queue, sync or mutating commands), debounced
 * on the Rust side. Carries no data: re-fetch whatever is displayed. Returns the unlisten.
 */
export const onChanged = (cb: (e: ChangedEvent) => void): Promise<UnlistenFn> =>
  listen<ChangedEvent>("nodal://changed", (ev) => cb(ev.payload));

/** "Check for Updates…" in the app menu (release builds only). Returns the unlisten. */
export const onCheckForUpdates = (cb: () => void): Promise<UnlistenFn> => listen("nodal://check-updates", () => cb());
