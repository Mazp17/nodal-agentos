//! Nodal's model: projects, repos, tasks, runs and external sources.
//! Exact mirror in `src/domain/types.ts`: any change here goes there too.
//!
//! Serialization conventions:
//! - structs in camelCase;
//! - "value" enums in snake_case (`in_place`, `in_progress`);
//! - enums with data carry the discriminant in `kind`;
//! - dates in epoch ms (`i64`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Generates `as_str`/`parse` for value enums, with the same names serde uses.
/// They're used as the SQLite representation (TEXT columns).
macro_rules! str_enum {
    ($ty:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        impl $ty {
            #[cfg(test)]
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
    /// Default executor if the repo doesn't define one. `None` → the global one in Settings → Claude.
    pub default_executor: Option<Executor>,
    /// Default reviewer if the repo doesn't define one. `None` → `code-reviewer`.
    pub reviewer: Option<String>,
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
    pub default_executor: Option<Executor>,
    pub default_isolation: Isolation,
    pub default_finish: Finish,
    pub default_review: bool,
    /// Reviewer agent. `None` → the project's → `code-reviewer`.
    pub reviewer: Option<String>,
    /// Order within the project.
    pub position: i64,
    pub created_at: i64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    /// `~/.nodal/worktrees/<repo>/<task-slug>`, branch `nodal/<task-slug>`.
    Worktree,
    /// In the repo folder; the queue never launches two at once per repo.
    InPlace,
}
str_enum!(Isolation { Worktree => "worktree", InPlace => "in_place" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    /// Uncommitted changes.
    Changes,
    /// Commit on the branch, no push.
    Commit,
    /// Commit, push and PR with `gh`.
    Pr,
}
str_enum!(Finish { Changes => "changes", Commit => "commit", Pr => "pr" });

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
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
    File { path: String },
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
    /// `None` → repo default → project default → Claude.
    pub assignee: Option<Executor>,
    /// `None` → repo default.
    pub isolation: Option<Isolation>,
    pub finish: Option<Finish>,
    pub review: Option<bool>,
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

// ---------- Runs ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Work,
    Review,
}
str_enum!(RunKind { Work => "work", Review => "review" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Launching,
    Launched,
    Finished,
    Failed,
    Canceled,
}
str_enum!(RunStatus {
    Queued => "queued",
    Launching => "launching",
    Launched => "launched",
    Finished => "finished",
    Failed => "failed",
    Canceled => "canceled",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Green,
    Yellow,
    Red,
    Stopped,
    Unknown,
}
str_enum!(RunOutcome { Green => "green", Yellow => "yellow", Red => "red", Stopped => "stopped", Unknown => "unknown" });

/// The reviewer's verdict (or the structured result of a reviewing workflow).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub pass: bool,
    pub unmet: Vec<String>,
    pub nits: Vec<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    /// `None` for runs without a task (launched by hand or migrated).
    pub task_id: Option<String>,
    pub repo_id: Option<String>,
    pub cwd: String,
    pub executor: Executor,
    pub kind: RunKind,
    /// Previous step in the chain (the reviewed run, or the one before a handoff).
    pub parent_run_id: Option<String>,
    pub prompt: String,
    /// "Extra instructions for this run", already included in `prompt`.
    pub extra_instructions: Option<String>,
    pub options: LaunchOptions,
    pub finish: Finish,
    /// Resolved on enqueue (task → repo). `None` for workflows, which manage their own worktree.
    pub isolation: Option<Isolation>,
    /// Resolved on enqueue: when this work run finishes, the reviewer is enqueued.
    pub review: bool,
    pub verdict: Option<Verdict>,
    pub status: RunStatus,
    /// Order in the global queue (lower goes first). REAL to reorder without renumbering.
    pub queue_position: f64,
    /// Short id from `claude --bg`.
    pub claude_run_id: Option<String>,
    pub session_id: Option<String>,
    pub queued_at: i64,
    pub launched_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub outcome: Option<RunOutcome>,
    /// `summary` from the executor's final JSON block (passed on to the next step).
    pub summary: Option<String>,
    pub pr_url: Option<String>,
    pub branch: Option<String>,
    pub error: Option<String>,
    /// Label of a run imported from a previous version (old issue or task).
    pub legacy_label: Option<String>,
    /// Transcript tokens (input + output + cache), summed on close (v2). `None` for
    /// workflows or if it couldn't be read.
    #[serde(default)]
    pub tokens: Option<i64>,
}

/// `Run` without `prompt` or `extraInstructions`, for lists (history, board).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLight {
    pub id: String,
    pub task_id: Option<String>,
    pub repo_id: Option<String>,
    pub cwd: String,
    pub executor: Executor,
    pub kind: RunKind,
    pub parent_run_id: Option<String>,
    pub options: LaunchOptions,
    pub finish: Finish,
    pub isolation: Option<Isolation>,
    pub review: bool,
    pub verdict: Option<Verdict>,
    pub status: RunStatus,
    pub queue_position: f64,
    pub claude_run_id: Option<String>,
    pub session_id: Option<String>,
    pub queued_at: i64,
    pub launched_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub outcome: Option<RunOutcome>,
    pub summary: Option<String>,
    pub pr_url: Option<String>,
    pub branch: Option<String>,
    pub error: Option<String>,
    pub legacy_label: Option<String>,
    pub tokens: Option<i64>,
}

impl From<Run> for RunLight {
    fn from(r: Run) -> Self {
        RunLight {
            id: r.id,
            task_id: r.task_id,
            repo_id: r.repo_id,
            cwd: r.cwd,
            executor: r.executor,
            kind: r.kind,
            parent_run_id: r.parent_run_id,
            options: r.options,
            finish: r.finish,
            isolation: r.isolation,
            review: r.review,
            verdict: r.verdict,
            status: r.status,
            queue_position: r.queue_position,
            claude_run_id: r.claude_run_id,
            session_id: r.session_id,
            queued_at: r.queued_at,
            launched_at: r.launched_at,
            finished_at: r.finished_at,
            outcome: r.outcome,
            summary: r.summary,
            pr_url: r.pr_url,
            branch: r.branch,
            error: r.error,
            legacy_label: r.legacy_label,
            tokens: r.tokens,
        }
    }
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

    #[cfg(test)]
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

    #[cfg(test)]
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
    SetState { state_id: String },
    Comment { body: String },
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

pub const DEFAULT_CONCURRENCY: u32 = 3;
pub const MAX_CONCURRENCY: u32 = 16;
pub const DEFAULT_REVIEWER: &str = "code-reviewer";

/// Global settings. In the database it's one row per field (`settings.key` = camelCase name).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Concurrent runs in the global queue.
    pub concurrency: u32,
    /// Editor command for "Open in editor" (`code`, `cursor`...). `None` → the system one.
    pub editor: Option<String>,
    /// Global reviewer if neither the repo nor the project define one.
    pub reviewer: String,
    /// Last executor fallback: repo → project → this → Claude.
    pub default_executor: Option<Executor>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { concurrency: DEFAULT_CONCURRENCY, editor: None, reviewer: DEFAULT_REVIEWER.into(), default_executor: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enums_serialize_snake_case_and_match_as_str() {
        for s in TaskStatus::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
            assert_eq!(TaskStatus::parse(s.as_str()), Some(*s));
        }
        for s in Isolation::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
        for s in RunStatus::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
        for s in RunOutcome::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
        for s in Priority::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
        assert_eq!(TaskStatus::parse("nope"), None);
    }

    #[test]
    fn executor_shape() {
        let a = Executor::Agent { name: "frontend-developer".into(), source: AgentSource::User };
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            json!({"kind": "agent", "name": "frontend-developer", "source": "user"})
        );
        assert_eq!(serde_json::to_value(Executor::Claude).unwrap(), json!({"kind": "claude"}));
        assert_eq!(
            serde_json::to_value(Executor::Workflow { name: "plan-task".into() }).unwrap(),
            json!({"kind": "workflow", "name": "plan-task"})
        );
    }

    #[test]
    fn outbox_payload_shape() {
        let p = OutboxPayload::SetState { state_id: "s3".into() };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"kind": "set_state", "stateId": "s3"}));
        assert_eq!(p.kind(), "set_state");
        let c = OutboxPayload::Comment { body: "hello".into() };
        assert_eq!(serde_json::to_value(&c).unwrap(), json!({"kind": "comment", "body": "hello"}));
        assert_eq!(c.kind(), "comment");
    }

    #[test]
    fn repo_flattens_launch_options() {
        let r = Repo {
            id: "r".into(),
            project_id: "p".into(),
            path: "/x".into(),
            name: "x".into(),
            launch: LaunchOptions { model: Some("opus".into()), ..Default::default() },
            default_executor: None,
            default_isolation: Isolation::InPlace,
            default_finish: Finish::Pr,
            default_review: true,
            reviewer: None,
            position: 0,
            created_at: 1,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["defaultIsolation"], "in_place");
        assert!(v.get("effort").is_none());
        assert_eq!(serde_json::from_value::<Repo>(v).unwrap(), r);
    }

    #[test]
    fn state_map_keys_are_status_strings() {
        let mut m = StateMap::default();
        m.pull.insert("s1".into(), TaskStatus::InReview);
        m.push.insert(TaskStatus::Blocked, None);
        m.push.insert(TaskStatus::InProgress, Some("s2".into()));
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["pull"]["s1"], "in_review");
        assert_eq!(v["push"]["blocked"], serde_json::Value::Null);
        assert_eq!(v["push"]["in_progress"], "s2");
        assert_eq!(serde_json::from_value::<StateMap>(v).unwrap(), m);
    }

    #[test]
    fn settings_defaults_fill_missing_fields() {
        let s: Settings = serde_json::from_value(json!({"editor": "code"})).unwrap();
        assert_eq!(s.concurrency, DEFAULT_CONCURRENCY);
        assert_eq!(s.reviewer, DEFAULT_REVIEWER);
        assert_eq!(s.editor.as_deref(), Some("code"));
        assert_eq!(task_key("PAY", 1), "PAY-1");
    }
}
