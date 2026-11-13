//! Nodal's model: projects, repos, tasks, runs and external sources.
//! Exact mirror in `src/domain/types.ts`: any change here goes there too.
//!
//! Serialization conventions:
//! - structs in camelCase;
//! - "value" enums in snake_case (`in_place`, `in_progress`);
//! - enums with data carry the discriminant in `kind`;
//! - dates in epoch ms (`i64`).

pub mod activity;
pub mod chat;
pub mod claude;
pub mod events;
pub mod executors;
pub mod providers;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Generates `as_str`/`parse` for value enums, with the same names serde uses.
/// They're used as the SQLite representation (TEXT columns).
macro_rules! str_enum {
    ($ty:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        impl $ty {
            #[cfg(any(test, feature = "test-support"))]
            #[allow(dead_code)]
            pub const ALL: &'static [$ty] = &[$($ty::$variant),+];
            // Generated for every enum; not all of them use these outside tests.
            #[allow(dead_code)]
            pub fn as_str(self) -> &'static str {
                match self { $($ty::$variant => $s),+ }
            }
            #[allow(dead_code)]
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some($ty::$variant),)+ _ => None }
            }
        }
    };
}

// ---------- Projects and repos ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Prefix for task ids (`PAY` → `PAY-1`). Uppercase letters and digits, unique.
    pub key: String,
    /// Next `Task.number` to assign (starts at 1).
    pub next_task_number: i64,
    /// Project color in the UI (hex or `oklch(...)`).
    pub color: String,
    /// Free-form description (v2).
    #[serde(default)]
    pub description: Option<String>,
    /// Canonical folder that holds the project's repos and other material (v6). Repo-less
    /// chats run there. Never a run target.
    #[serde(default)]
    pub root_path: Option<String>,
    pub created_at: i64,
    pub archived_at: Option<i64>,
}

/// Optional `claude --bg` flags configured per repo.
/// Allowed values: see `runs::options` (checked against `claude --help` 2.1.281).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub id: String,
    pub project_id: String,
    /// Canonical git root. Unique across the whole app.
    pub path: String,
    pub name: String,
    /// `model`, `effort` and `permissionMode` are flattened in the JSON.
    #[serde(flatten)]
    pub launch: LaunchOptions,
    /// Order within the project.
    pub position: i64,
    pub created_at: i64,
}

// ---------- Chats ----------

/// A chat: a `claude -p` session scoped to a project (or narrowed to one of its repos).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub id: String,
    pub project_id: String,
    /// The Repo pill. `None` → the whole project; it runs in the project's first repo.
    pub repo_id: Option<String>,
    /// `None` until the first message names it.
    pub title: Option<String>,
    /// Claude Code's name for the session (`/rename`, else its AI title), shown over `title`.
    #[serde(default)]
    pub session_title: Option<String>,
    /// Claude Code's session id, known after the first message.
    pub session_id: Option<String>,
    /// Agent the chat talks to (`--agent <name>`, `plugin:agent` for plugin ones); `None` is
    /// plain Claude.
    #[serde(default)]
    pub agent: Option<String>,
    /// The task this chat works on: it runs in the task's worktree and moves its status.
    #[serde(default)]
    pub task_id: Option<String>,
    /// A free chat's own worktree folder (one of its repo's); a task chat uses the task's.
    #[serde(default)]
    pub worktree: Option<String>,
    /// `model`, `effort` and `permissionMode` are flattened in the JSON.
    #[serde(flatten)]
    pub launch: LaunchOptions,
    pub created_at: i64,
    /// Last message sent: the sessions list is ordered by it.
    pub updated_at: i64,
}

// ---------- Executors and options ----------

/// Where an agent's definition comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSource {
    /// `~/.claude/agents/*.md`
    User,
    /// `<repo>/.claude/agents/*.md`
    Repo,
    /// An enabled plugin.
    Plugin,
}
str_enum!(AgentSource { User => "user", Repo => "repo", Plugin => "plugin" });

/// Who a task is delegated to (its `assignee`) or who runs a run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Executor {
    /// `claude --bg --agent <name> "<prompt>"`.
    Agent { name: String, source: AgentSource },
    /// `claude --bg "/<name> <args>"`.
    Workflow { name: String },
    /// Plain session with the task as the prompt.
    Claude,
}

/// What a hidden-executor entry points at; plain Claude has no variant, so it can't be hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiddenKind {
    Agent,
    Workflow,
}
str_enum!(HiddenKind { Agent => "agent", Workflow => "workflow" });

/// An agent or workflow a project hides from its pickers (the definition file is untouched).
/// `repo_id` is set only for `source: repo`: the same repo agent in two repos is two entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HiddenExecutor {
    pub kind: HiddenKind,
    pub source: AgentSource,
    pub name: String,
    #[serde(default)]
    pub repo_id: Option<String>,
}

// ---------- Tasks ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    Todo,
    InProgress,
    InReview,
    Blocked,
    Done,
    Canceled,
}
str_enum!(TaskStatus {
    Backlog => "backlog",
    Todo => "todo",
    InProgress => "in_progress",
    InReview => "in_review",
    Blocked => "blocked",
    Done => "done",
    Canceled => "canceled",
});

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Urgent,
    High,
    Medium,
    Low,
    #[default]
    None,
}
str_enum!(Priority { Urgent => "urgent", High => "high", Medium => "medium", Low => "low", None => "none" });

/// Where the plan is. The text lives in `tasks/<id>/plan.md` (not in the database).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanRef {
    Text,
    /// Absolute path of a `.md` inside the repo.
    File {
        path: String,
    },
}

/// The task's own worktree (only with `Isolation::Worktree`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRef {
    pub path: String,
    pub branch: String,
    /// Base ref the diff is computed against.
    pub base: String,
}

/// Normalized type of an external state, for the mapping heuristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtKind {
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
    /// The provider doesn't type its states (e.g. Asana sections).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalState {
    pub id: String,
    pub name: String,
    pub kind: ExtKind,
    pub color: Option<String>,
}

/// Link between an imported task and its item in the provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSource {
    /// `"linear"`, later `"asana"`, `"azure_devops"`...
    pub provider: String,
    /// SourceLink it was imported through. To delete the link, its tasks are unlinked first
    /// (`source = None`): the database won't delete it while tasks point to it.
    pub link_id: Option<String>,
    pub external_id: String,
    /// Provider's human-readable id (`ENG-142`).
    pub identifier: String,
    pub url: String,
    pub external_state: Option<ExternalState>,
    pub last_synced_at: Option<i64>,
    pub sync_error: Option<String>,
    /// The current external state is not in the pull mapping (v2): the task keeps its Nodal
    /// status and the UI shows "unmapped external state".
    #[serde(default)]
    pub unmapped: bool,
    /// Provider project seen on the last import/pull (v3).
    #[serde(default)]
    pub project: Option<ExtProject>,
    /// Project rule through which it reached the repo (v3): only those tasks warn if the
    /// issue changes project.
    #[serde(default)]
    pub rule_id: Option<String>,
    /// Project change awaiting a decision (v3).
    #[serde(default)]
    pub moved: Option<MovedInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub repo_id: String,
    /// Number within the project: the visible id is `{project.key}-{number}`.
    pub number: i64,
    pub title: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub labels: Vec<String>,
    /// Order within the column (REAL to insert between two without renumbering).
    pub position: f64,
    pub plan: PlanRef,
    /// On an imported task: the plan is no longer regenerated from the external description.
    pub plan_overridden: bool,
    /// Acceptance criteria the gate reviews against.
    pub acceptance: Vec<String>,
    pub worktree: Option<WorktreeRef>,
    pub source: Option<TaskSource>,
    pub created_at: i64,
    pub updated_at: i64,
    /// When it moved to Done or Canceled.
    pub closed_at: Option<i64>,
}

/// `{KEY}-{number}`, e.g. `PAY-1`.
pub fn task_key(project_key: &str, number: i64) -> String {
    format!("{project_key}-{number}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Related,
    /// `task_id` blocks `other_id`.
    Blocks,
}
str_enum!(RelationKind { Related => "related", Blocks => "blocks" });

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRelation {
    pub task_id: String,
    pub other_id: String,
    pub kind: RelationKind,
}

// ---------- Verdict ----------

/// The reviewer's verdict (or the structured result of a reviewing workflow).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub pass: bool,
    pub unmet: Vec<String>,
    pub nits: Vec<String>,
    pub summary: Option<String>,
}

// ---------- External sources ----------

/// Provider scope being linked (Linear team or project, Asana project...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeRef {
    /// Depends on the provider: `"team"`, `"project"`...
    pub kind: String,
    pub id: String,
    pub name: String,
}

/// What a routing rule looks at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// One of the item's labels (case-insensitive).
    #[default]
    Label,
    /// The provider project (Linear project) the item belongs to.
    Project,
    /// Type this version doesn't know: the rule routes nothing, but reading it doesn't break
    /// the link (or the whole sync).
    #[serde(other)]
    Unknown,
}

/// On import, an item with this label (or from this provider project) goes to this repo.
/// Precedence: project rule > label rule > the link's default repo.
///
/// Compatible with the v1/v2 JSON `{label, repoId}`: `kind` defaults to `label`, `value`
/// accepts `label`, and empty `id`/`created_at`/`name` are filled in on read
/// (`normalize_legacy_rules`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoRule {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: RuleKind,
    /// Label, or the provider project's id.
    #[serde(alias = "label")]
    pub value: String,
    /// Display name (the project's; for label rules, the label).
    #[serde(default)]
    pub name: String,
    pub repo_id: String,
    /// Project rules auto-import items created after this (earlier ones are brought in by
    /// the backfill when the rule is created).
    #[serde(default)]
    pub created_at: i64,
}

impl RepoRule {
    pub fn is_project(&self) -> bool {
        self.kind == RuleKind::Project
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn label(label: &str, repo_id: &str) -> Self {
        RepoRule {
            id: format!("rule-{label}"),
            kind: RuleKind::Label,
            value: label.into(),
            name: label.into(),
            repo_id: repo_id.into(),
            created_at: 1,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn project(id: &str, name: &str, repo_id: &str, created_at: i64) -> Self {
        RepoRule {
            id: format!("rule-{id}"),
            kind: RuleKind::Project,
            value: id.into(),
            name: name.into(),
            repo_id: repo_id.into(),
            created_at,
        }
    }
}

/// Fills in what the old JSON lacks: a stable id by position (`rule-{i}`, so two reads
/// yield the same id), the link's `created_at` and `name` = `value`.
pub fn normalize_legacy_rules(rules: &mut [RepoRule], link_created_at: i64) {
    for (i, r) in rules.iter_mut().enumerate() {
        if r.id.is_empty() {
            r.id = format!("rule-{i}");
        }
        if r.created_at == 0 {
            r.created_at = link_created_at;
        }
        if r.name.trim().is_empty() {
            r.name = r.value.clone();
        }
    }
}

/// Provider project an item belongs to (id and name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtProject {
    pub id: String,
    pub name: String,
}

/// The issue changed project in the provider after arriving through a project rule
/// (v3). Nodal doesn't move it to another repo: it waits for `resolve_moved_task`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovedInfo {
    pub from_project: ExtProject,
    /// `None`: the issue was left without a project.
    pub to_project: Option<ExtProject>,
    /// Repo it would now get by the rules; `None` if none applies.
    pub suggested_repo_id: Option<String>,
}

/// State mapping in both directions. Nodal proposes it; the user confirms it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMap {
    /// `external_state.id` → Nodal status. A missing id is "unmapped".
    pub pull: BTreeMap<String, TaskStatus>,
    /// Nodal status → `external_state.id`; `null` = "Don't sync" (comment only).
    pub push: BTreeMap<TaskStatus, Option<String>>,
    /// `None` = mapping pending: importing works but nothing is pushed.
    pub confirmed_at: Option<i64>,
    /// Snapshot of the provider states at confirmation, to detect additions and removals.
    pub known_states: Vec<ExternalState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLink {
    pub id: String,
    pub project_id: String,
    pub provider: String,
    pub scope: ScopeRef,
    pub default_repo_id: Option<String>,
    pub repo_rules: Vec<RepoRule>,
    pub state_map: StateMap,
    pub auto_import: bool,
    pub created_at: i64,
    /// Last sync pass over this link (v2).
    #[serde(default)]
    pub last_synced_at: Option<i64>,
    /// Error from the last pass (`None` if it went fine).
    #[serde(default)]
    pub last_sync_error: Option<String>,
    /// Unreviewed state additions/removals. `None` = nothing pending.
    #[serde(default)]
    pub pending_state_changes: Option<StateChanges>,
}

/// Provider states that changed against `StateMap.known_states` (v2). Set by the
/// sync; cleared when the mapping is saved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateChanges {
    pub added: Vec<ExternalState>,
    pub removed: Vec<ExternalState>,
}

impl StateChanges {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// What must be written to the provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutboxPayload {
    #[serde(rename_all = "camelCase")]
    SetState {
        state_id: String,
    },
    Comment {
        body: String,
    },
}

impl OutboxPayload {
    pub fn kind(&self) -> &'static str {
        match self {
            OutboxPayload::SetState { .. } => "set_state",
            OutboxPayload::Comment { .. } => "comment",
        }
    }
}

/// Pending write to the provider (state push or comment), with backoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItem {
    /// Autoincrement; 0 before insertion.
    pub id: i64,
    pub task_id: String,
    pub provider: String,
    pub payload: OutboxPayload,
    pub attempts: i64,
    pub next_attempt_at: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
}

// ---------- Settings ----------

/// Global settings. In the database it's one row per field (`settings.key` = camelCase name).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Editor command for "Open in editor" (`code`, `cursor`...). `None` → the system one.
    pub editor: Option<String>,
    /// Intro of the chat agent's system prompt. `None` → `DEFAULT_CHAT_INTRO` (nodal-app).
    pub chat_system_prompt: Option<String>,
}

/// One child of a repo directory (`list_repo_dir`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Uncommitted changes of a checkout against HEAD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
}

/// Where a chat is working now (it may have moved into a worktree) and its uncommitted changes there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatWorkspace {
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub is_worktree: bool,
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
}

/// A repo file for the read-only viewer: text of the first 1 MB, or empty when `binary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoFile {
    /// Opaque token of the file on disk; changes whenever the file does.
    pub version: String,
    pub content: String,
    pub binary: bool,
    pub truncated: bool,
    pub size: u64,
}

/// A checkout of a repo: its main folder or a linked git worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoWorktree {
    pub path: String,
    pub branch: Option<String>,
    pub is_main: bool,
}

/// Cap on entries `list_repo_dir` returns, to keep the UI responsive.
pub const DIR_ENTRIES_MAX: usize = 2000;
/// Cap on the files `list_repo_files` returns for the quick-open search.
pub const REPO_FILES_MAX: usize = 20_000;

#[cfg(test)]
mod tests;
