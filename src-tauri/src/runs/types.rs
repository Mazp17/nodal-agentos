//! Types the `runs` module exposes to the frontend. Mirrored in `src/features/runs/types.ts`.
//! They're ours, not Claude Code's: parsing of the internal format lives in `claude_fs.rs`.

use serde::Serialize;

/// Moved to `domain`; re-exported so current uses don't break.
pub use crate::domain::LaunchOptions;

/// What `launch_run` returns: the short id that `claude --bg` prints.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRef {
    pub id: String,
    pub cwd: String,
}

/// Why a background session ended without getting to run its workflow.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LaunchBlocker {
    /// The `Workflow` tool was rejected with "Review dynamic workflow before running".
    WorkflowReview { workflow: Option<String> },
}

/// A background session as reported by `claude agents --json --all`.
/// Stopped sessions have no `pid` or `status`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub session_id: String,
    pub cwd: Option<String>,
    pub name: Option<String>,
    /// Epoch in ms.
    pub started_at: Option<i64>,
    pub pid: Option<u32>,
    /// "busy" | "idle" | "waiting" (live sessions only).
    pub status: Option<String>,
    /// "working" | "blocked" | "done" | "failed" | "stopped".
    pub state: Option<String>,
    /// With `status == "waiting"`: "permission prompt", "input needed", "sandbox request",
    /// "worker request", "dialog open" (values from the agent view docs; the first one verified).
    pub waiting_for: Option<String>,
}

impl RunSummary {
    /// The session is still in progress: working, or blocked waiting on the user (permission,
    /// input). Either way it takes a slot and the issue can't be relaunched.
    pub fn is_in_progress(&self) -> bool {
        matches!(self.state.as_deref(), Some("working" | "blocked"))
    }
}

/// Where the detail came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailSource {
    /// `workflows/wf_*.json`: the workflow finished (or at least wrote its summary).
    Final,
    /// Rebuilt from `journal.jsonl`: in progress, or cut off without a summary.
    Live,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseInfo {
    pub title: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Queued,
    Running,
    Done,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub agent_id: Option<String>,
    pub label: String,
    pub phase: Option<String>,
    pub model: Option<String>,
    pub state: AgentState,
    pub tokens: Option<u64>,
    pub tool_calls: Option<u64>,
    pub duration_ms: Option<u64>,
    pub last_tool_name: Option<String>,
    pub last_tool_summary: Option<String>,
    pub result_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    /// "wf_2450b7a8-254".
    pub workflow_id: String,
    pub workflow_name: Option<String>,
    pub source: DetailSource,
    /// Workflow status according to Claude Code ("completed", ...). `None` in `live` mode:
    /// the journal doesn't tell "in progress" from "cut off"; cross-check with `RunSummary.state`.
    pub status: Option<String>,
    pub phases: Vec<PhaseInfo>,
    pub current_phase: Option<String>,
    /// 1-based within `phases`.
    pub current_phase_index: Option<u32>,
    pub agents: Vec<AgentInfo>,
    pub agent_count: u32,
    pub total_tokens: Option<u64>,
    pub total_tool_calls: Option<u64>,
    pub duration_ms: Option<u64>,
    /// The workflow's `result.status` if it's "green" | "yellow" | "red".
    pub result_status: Option<String>,
    /// Known fields of the workflow's `result` (only in `final` mode).
    pub result: Option<RunResult>,
    /// How many workflows the session has (only the most recent one is returned).
    pub workflow_count: u32,
}

/// Known fields of the `result` a workflow returns (e.g. `linear-issue`).
/// `result` is free-form: each field is optional and ignored if it lacks the expected type.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub issue: Option<String>,
    /// PR URL (http/https only).
    pub pr: Option<String>,
    pub branch: Option<String>,
    pub workdir: Option<String>,
    /// Where it stopped (linear-issue in Blocked).
    #[serde(rename = "where")]
    pub where_: Option<String>,
    pub unmet_acceptance: Option<Vec<String>>,
    pub nits: Option<Vec<String>>,
    /// Full `result` as indented JSON (clipped), to show what isn't known.
    pub raw: Option<String>,
}

/// An entry in a subagent's conversation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TranscriptItem {
    /// Assistant text.
    #[serde(rename_all = "camelCase")]
    Text { text: String, truncated: bool },
    /// Assistant reasoning (only when in plain text; almost always empty).
    #[serde(rename_all = "camelCase")]
    Thinking { text: String, truncated: bool },
    /// User-side message that isn't a tool result.
    #[serde(rename_all = "camelCase")]
    User { text: String, truncated: bool },
    #[serde(rename_all = "camelCase")]
    ToolUse {
        id: Option<String>,
        name: String,
        /// One line: the main argument (command, path, pattern...).
        summary: Option<String>,
        /// Full input as indented JSON, clipped.
        input: Option<String>,
        result: Option<ToolResultInfo>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultInfo {
    pub text: String,
    pub is_error: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub agent_id: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub phase: Option<String>,
    /// Task the agent received (text computed by the workflow).
    pub prompt: Option<String>,
    pub items: Vec<TranscriptItem>,
    /// Total items in what was read; the first `omitted` aren't returned.
    pub total_items: u32,
    pub omitted: u32,
    /// Final output: the `StructuredOutput` input or the last assistant text.
    pub final_output: Option<String>,
    /// The file was too large and only its head and tail were read.
    pub partial: bool,
    pub bytes: u64,
}
