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

import { invoke, type Channel } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentSource,
  Chat,
  ExtKind,
  Executor,
  ExternalState,
  HiddenExecutor,
  LaunchOptions,
  Priority,
  Project,
  RelationKind,
  Repo,
  RepoRule,
  ScopeRef,
  Settings,
  SourceLink,
  StateMap,
  Task,
  TaskRelation,
  TaskStatus,
} from "./types";
import type { SubagentRef, ToolResultInfo, Transcript, TranscriptItem } from "../features/chats/types";

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

/** `exists`: the folder is a live worktree. `ahead`: commits outside the base; `unpushed`: also outside every remote. */
export interface WorktreeStatus {
  exists: boolean;
  branch: string | null;
  base: string | null;
  ahead: number;
  unpushed: number;
  dirty: boolean;
  hasRemote: boolean;
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
  /** Must be an existing folder; it's stored canonical. */
  rootPath?: string | null;
}

/** Only the present fields change; `null` clears the optional ones. */
export type ProjectPatch = Partial<Pick<Project, "name" | "key" | "color" | "description" | "rootPath">> & {
  archived?: boolean;
};

/** `path` can be any folder inside the repo: it resolves to the git root. */
export interface NewRepo extends LaunchOptions {
  path: string;
  name?: string;
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
}

/**
 * `repoId` moves the task to another repo of the same project (no runs in progress and no worktree; a
 * `.md` plan from the old repo requires sending another `plan`).
 * Imported tasks only accept `repoId`, `plan` (becomes `planOverridden`), `status`,
 * `acceptance`.
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
  /** Agents: user/repo/plugin; workflows: user/repo; Claude: `null`. */
  source: AgentSource | null;
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
/** Canonical git roots up to 3 levels below `root` (no hidden folders, `node_modules` or nested repos). */
export const scanGitRepos = (root: string) => invoke<string[]>("scan_git_repos", { root });
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
/** Opens Terminal.app at `path`; with `runClaude`, starts `claude` there. */
export const openTerminalAt = (path: string, runClaude = false) => invoke<void>("open_terminal_at", { path, runClaude });
/** `git version 2.x.y`; rejects if git isn't on the PATH. */
export const gitVersion = () => invoke<string>("git_version");
/** Squash defaults to true in the backend; `push` is opt-in; `cleanup` deletes the worktree and branch after merging. */
export interface MergeInput {
  squash: boolean;
  push: boolean;
  cleanup: boolean;
}

/** `commit`: the base's new tip. `moved`: the base had moved and was merged into the worktree first. */
export type MergeOutcome =
  | { kind: "merged"; commit: string; commits: number; squashed: boolean; moved: boolean }
  /** The merge was aborted: nothing is left mid-merge. */
  | { kind: "conflict"; files: string[] };

export interface MergeReport {
  outcome: MergeOutcome;
  /** Done after a merge; unchanged on a conflict. */
  task: Task;
  pushedTo: string | null;
  pushError: string | null;
  /** The merge went through but the worktree couldn't be cleaned up. */
  cleanupError: string | null;
}

/**
 * "Merge into <base> & done": commits uncommitted changes, lands the task branch on its base
 * by fast-forward (merging a moved base into the worktree first) and marks the task Done.
 * Rejects without touching anything if the base is checked out with uncommitted changes.
 */
export const mergeWorktree = (taskId: string, input: MergeInput) =>
  invoke<MergeReport>("merge_worktree", { taskId, input });
/** Everything zero/false/null if the task has no worktree. */
export const worktreeStatus = (taskId: string) => invoke<WorktreeStatus>("worktree_status", { taskId });

// ---------- Chat agents ----------

/** Agents the project hides from its chat pickers (`listChatAgents` stays unfiltered). */
export const listHiddenExecutors = (projectId: string) =>
  invoke<HiddenExecutor[]>("list_hidden_executors", { projectId });
/** Hides or shows one agent in the project; resolves with the project's new set. */
export const setExecutorHidden = (projectId: string, key: HiddenExecutor, hidden: boolean) =>
  invoke<HiddenExecutor[]>("set_executor_hidden", { projectId, key, hidden });

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

// ---------- Chats ----------

export interface NewChat extends LaunchOptions {
  /** The Repo pill; absent → the whole project. */
  repoId?: string | null;
  title?: string | null;
  /** Agent to talk to; absent → plain Claude. */
  agent?: string | null;
  /** One of `repoId`'s worktrees (its path) to run in; absent → the repo's own checkout. */
  worktree?: string | null;
}

export type ChatPatch = { [K in "title" | "repoId" | "agent" | "model" | "effort" | "permissionMode"]?: string | null };

/** A permission prompt waiting for `respondChatPermission`. */
export interface PermissionRequest {
  requestId: string;
  toolName: string;
  /** The `toolUse` item it belongs to. */
  toolUseId: string | null;
  /** What the tool is about to do, as Claude Code words it. */
  description: string | null;
  /** One line: the main argument (path, command…). */
  summary: string | null;
  input: unknown;
}

export type ChatRunState = "stopped" | "idle" | "busy";

export interface ChatLive {
  state: ChatRunState;
  /** Oldest first. */
  pending: PermissionRequest[];
}

/** The events a subagent streams. */
export type SubagentEvent =
  | { type: "item"; item: TranscriptItem }
  | { type: "toolResult"; toolUseId: string; result: ToolResultInfo };

/**
 * What a chat streams. `textDelta`/`thinkingDelta` preview text that a later `item` carries
 * in full; `item` also brings back each user message, in order.
 */
export type ChatEvent =
  | { type: "init"; sessionId: string; model: string | null; cwd: string | null; permissionMode: string | null }
  | { type: "textDelta"; text: string }
  | { type: "thinkingDelta"; text: string }
  | { type: "item"; item: TranscriptItem }
  | { type: "toolResult"; toolUseId: string; result: ToolResultInfo }
  /** What a subagent does inside: its own items and tool results, under the `Agent` call that launched it. */
  | { type: "subagent"; parentToolUseId: string; event: SubagentEvent }
  | ({ type: "permissionRequest" } & PermissionRequest)
  /** Denied without asking (the permission mode or a rule decided). */
  | { type: "permissionDenied"; toolName: string; toolUseId: string | null; message: string | null }
  /** Tokens in the context after the last response. */
  | { type: "usage"; contextTokens: number }
  /** `ok` is false for errors and interrupts. */
  | {
      type: "turnEnd";
      ok: boolean;
      subtype: string;
      sessionId: string | null;
      result: string | null;
      costUsd: number | null;
      contextWindow: number | null;
    }
  | { type: "state"; state: ChatRunState }
  | { type: "permissionResolved"; requestId: string; allowed: boolean }
  | { type: "error"; message: string };

/** Payload of `nodal://chat`. */
export interface ChatEventEnvelope {
  chatId: string;
  event: ChatEvent;
}

/** How a chat sees the `propose_task` tool of `nodal-mcp --chat` (`chats::context` in Rust). */
export const PROPOSE_TASK_TOOL = "mcp__nodal__propose_task";

/**
 * What `propose_task` returns (its input has the `create_task` MCP fields). Nothing is created:
 * accepting the card calls `createTask(newTask)`.
 */
export interface TaskProposal {
  newTask: NewTask;
  projectKey: string;
  repoName: string;
  created: false;
  note: string;
}

/** What Claude Code uses when a chat leaves model or effort unset, from its settings files. */
export interface ClaudeDefaults {
  model: string | null;
  effort: string | null;
}
/** Agents a chat can talk to in the folder it runs in for `repoId` (`null`: the whole project). */
export const listChatAgents = (projectId: string, repoId: string | null) =>
  invoke<ExecutorInfo[]>("list_chat_agents", { projectId, repoId });
/** Creates (or returns the existing) chat that works the task in its own worktree; the task moves to In progress. */
/** `prompt`: the task's first message, still unsent (`null` when the chat already existed). */
export const startTaskChat = (taskId: string) => invoke<{ chat: Chat; prompt: string | null }>("start_task_chat", { taskId });
export const getClaudeDefaults = (repoId: string | null) => invoke<ClaudeDefaults>("get_claude_defaults", { repoId });
/** A project's chats (with `null`, every project's), most recently used first. */
export const listChats = (projectId: string | null) => invoke<Chat[]>("list_chats", { projectId });
export const createChat = (projectId: string, input: NewChat = {}) => invoke<Chat>("create_chat", { projectId, input });
/** New settings apply from the next message. */
export const updateChat = (id: string, patch: ChatPatch) => invoke<Chat>("update_chat", { id, patch });
export const deleteChat = (id: string) => invoke<void>("delete_chat", { id });
/** Starts or resumes the chat's `claude` process if needed; the answer streams as `nodal://chat`. */
export const sendChatMessage = (id: string, text: string) => invoke<Chat>("send_chat_message", { id, text });
/** `message` is what Claude reads on a denial. */
export const respondChatPermission = (id: string, requestId: string, allow: boolean, message: string | null = null) =>
  invoke<void>("respond_chat_permission", { id, requestId, allow, message });
export const interruptChat = (id: string) => invoke<void>("interrupt_chat", { id });

/** What rewinding a chat to a message does (or would do, on a dry run) to the files Claude edited. */
export interface RewindResult {
  filesChanged: string[];
  insertions: number;
  deletions: number;
  /** Why the files can't be restored (e.g. a session from before checkpoints); the conversation still rewinds. */
  filesError: string | null;
}
/**
 * Rewinds a chat to just before the user message `messageUuid`: restores the files Claude edited
 * since, and the next message continues from there. `dryRun` only reports what would change.
 */
export const rewindChat = (id: string, messageUuid: string, dryRun: boolean) =>
  invoke<RewindResult>("rewind_chat", { id, messageUuid, dryRun });
export const stopChat = (id: string) => invoke<void>("stop_chat", { id });
export const getChatLive = (id: string) => invoke<ChatLive>("get_chat_live", { id });
/** History from the Claude Code session; `null` before the first message. */
export const getChatTranscript = (id: string, limit?: number) =>
  invoke<Transcript | null>("get_chat_transcript", { id, limit: limit ?? null });
/** The subagents of the chat's session, oldest first. */
export const listChatSubagents = (chatId: string) => invoke<SubagentRef[]>("list_chat_subagents", { chatId });
/** A subagent's conversation; `null` when its file isn't there. */
export const getChatSubagentTranscript = (chatId: string, agentId: string, limit?: number) =>
  invoke<Transcript | null>("get_chat_subagent_transcript", { chatId, agentId, limit: limit ?? null });
/**
 * Registers `onEvent` as `id`'s dedicated channel (P05): from then on its events arrive there,
 * batched on the Rust side, instead of the shared `nodal://chat` broadcast. Call before the
 * chat's first message; safe to call again (e.g. after a reload).
 */
export const attachChatChannel = (id: string, onEvent: Channel<ChatEventEnvelope>) =>
  invoke<void>("attach_chat_channel", { id, onEvent });
/** Stops delivering `id`'s events (called when the chat is deleted). */
export const detachChatChannel = (id: string) => invoke<void>("detach_chat_channel", { id });

// ---------- Migration ----------

/** Copies `folder` to `legacy-backup-<ts>/` and imports it in a transaction. Idempotent. */
export const importLegacyData = (folder: string) => invoke<LegacyImportReport>("import_legacy_data", { folder });

// ---------- Settings ----------

/** Output of `claude --version` (e.g. `2.1.0 (Claude Code)`); rejects if the CLI isn't found. */
export const claudeVersion = () => invoke<string>("claude_version");

export interface UsageTotals {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  /** API responses. */
  messages: number;
}
/** Tokens of every Claude Code session on this machine, read from its transcripts. */
export interface UsageSummary {
  last24h: UsageTotals;
  last7d: UsageTotals;
  last30d: UsageTotals;
  allTime: UsageTotals;
  /** Tokens per rolling day, oldest first: the last entry is the last 24 hours. */
  daily30d: number[];
  /** Last 30 days, most tokens first. */
  models30d: { model: string; totals: UsageTotals }[];
  sessions: number;
  /** Oldest message seen (epoch ms). */
  since: number | null;
}
export const claudeUsage = () => invoke<UsageSummary>("claude_usage");

/** `false` in debug builds ("Nodal Dev"): they never check for updates. */
export const updatesEnabled = () => invoke<boolean>("updates_enabled");
/** Relaunch after an update was installed. */
export const restartApp = () => invoke<void>("restart_app");

/** The app side of `nodal-mcp`: a Unix socket agents reach while Nodal is open. */
export interface McpStatus {
  /** `false` without a database: it can't start. */
  available: boolean;
  running: boolean;
  socket: string | null;
  /** Why it is off, when it didn't stop on request. */
  error: string | null;
}
export const mcpStatus = () => invoke<McpStatus>("mcp_status");
/** Starts it, or stops and starts it again. */
export const mcpRestart = () => invoke<McpStatus>("mcp_restart");
export const mcpStop = () => invoke<McpStatus>("mcp_stop");

export const getSettings = () => invoke<Settings>("get_settings");
export const setSettings = (settings: Settings) => invoke<Settings>("set_settings", { settings });
/** The built-in `chatSystemPrompt`. */
export const defaultChatSystemPrompt = () => invoke<string>("default_chat_system_prompt");

// ---------- Events ----------

export type ChangedKind = "tasks" | "sources" | "projects" | "chats";

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
/** ⌘W from the app menu (the native menu takes the key before the page sees it). */
export const onCloseTab = (cb: () => void): Promise<UnlistenFn> => listen("nodal://close-tab", () => cb());

export interface DirEntry { name: string; isDir: boolean }
/** Current branch of a repo's checkout; `null` when detached or unknown. */
export const repoBranch = (repoId: string) => invoke<string | null>("repo_branch", { repoId });
/** Direct children of `dir` (relative to the repo root, "" for the root), dirs first, git-ignored entries left out. */
export const listRepoDir = (repoId: string, dir: string, worktree: string | null = null) =>
  invoke<DirEntry[]>("list_repo_dir", { repoId, dir, worktree });

/** Where a chat is working now (it may have moved into a worktree) and its uncommitted changes there. */
export interface ChatWorkspace { cwd: string | null; branch: string | null; isWorktree: boolean; files: number; additions: number; deletions: number }
export const chatWorkspace = (chatId: string) => invoke<ChatWorkspace>("chat_workspace", { chatId });

export interface RepoFile { version: string; content: string; binary: boolean; truncated: boolean; size: number }
/** A repo file's text for the read-only viewer (first 1 MB; `binary` files come back empty). */
export const readRepoFile = (repoId: string, path: string, worktree: string | null = null) =>
  invoke<RepoFile>("read_repo_file", { repoId, path, worktree });

/** Uncommitted changes of a repo's checkout vs HEAD (untracked files included), per file with hunks. */
export const repoChanges = (repoId: string, worktree: string | null = null) =>
  invoke<FileDiff[]>("repo_changes", { repoId, worktree });
export interface CommitInfo { sha: string; shortSha: string; subject: string; author: string; at: number }
/** A checkout's commits, newest first: a worktree's since it branched off, the repo's own its latest. */
export const repoCommits = (repoId: string, worktree: string | null = null) =>
  invoke<CommitInfo[]>("repo_commits", { repoId, worktree });
/** One commit's files with their hunks. */
export const repoCommitDiff = (repoId: string, sha: string, worktree: string | null = null) =>
  invoke<FileDiff[]>("repo_commit_diff", { repoId, worktree, sha });
/** Saves an edited repo file. `version` is the one it was read at; a newer one on disk fails with "File changed on disk…" unless `force`. */
export const writeRepoFile = (repoId: string, path: string, content: string, version: string, force = false, worktree: string | null = null) =>
  invoke<RepoFile>("write_repo_file", { repoId, path, content, version, force, worktree });

export interface RepoWorktree { path: string; branch: string | null; isMain: boolean }
/** A repo's checkouts, main first; pass one's `path` as the `worktree` argument of the repo file bindings. */
/** Stages everything in the checkout and commits it; returns the new commit. */
export const repoCommit = (repoId: string, worktree: string | null, message: string, paths: string[] = []) =>
  invoke<CommitInfo>("repo_commit", { repoId, worktree, message, paths });
/** Pushes the checkout's branch and opens a PR with the GitHub CLI (or returns the open one's URL). */
export const repoOpenPr = (repoId: string, worktree: string | null, title: string, body: string) =>
  invoke<string>("repo_open_pr", { repoId, worktree, title, body });
/** Every file of a checkout (relative paths, ignored ones left out) for the quick-open search. */
export const listRepoFiles = (repoId: string, worktree: string | null = null) =>
  invoke<string[]>("list_repo_files", { repoId, worktree });
export const listRepoWorktrees = (repoId: string) => invoke<RepoWorktree[]>("list_repo_worktrees", { repoId });
/** Deletes one of the repo's worktrees and its branch. Without `force` it refuses one with uncommitted or unpushed work. */
export const removeRepoWorktree = (repoId: string, path: string, force: boolean) =>
  invoke<void>("remove_repo_worktree", { repoId, path, force });
/** Discards one file's uncommitted changes (restores it from HEAD, or deletes it if new). */
export const revertRepoFile = (repoId: string, path: string, worktree: string | null = null) =>
  invoke<void>("revert_repo_file", { repoId, path, worktree });
