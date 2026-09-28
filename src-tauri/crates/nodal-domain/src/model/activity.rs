//! DTOs for Claude Code activity in a repo (sessions and subagents). Parsing of Claude
//! Code's internal format, and the assembly of these from disk, live in nodal-host.

use std::collections::HashSet;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActivity {
    pub session_id: String,
    /// Short id (background only).
    pub id: Option<String>,
    /// "interactive" | "background" | "unlisted" (an active transcript that `claude agents`
    /// doesn't list: `claude -p`, SDK, …).
    pub kind: String,
    pub name: Option<String>,
    pub cwd: Option<String>,
    /// "busy" | "idle" | "waiting" (live processes).
    pub status: Option<String>,
    /// "working" | "blocked" | "done" | "stopped" (background).
    pub state: Option<String>,
    pub waiting_for: Option<String>,
    pub started_at: Option<i64>,
    pub pid: Option<u32>,
    pub alive: bool,
    /// Launched from the app (issue or task).
    pub is_app_run: bool,
    /// The transcript's mtime.
    pub last_activity_at: Option<i64>,
    pub last_tool: Option<String>,
    pub last_tool_summary: Option<String>,
    /// "cli", "sdk-cli", … (from the transcript).
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentActivity {
    pub session_id: String,
    pub agent_id: String,
    pub description: Option<String>,
    pub agent_type: Option<String>,
    /// Own or inherited worktree; otherwise, the transcript's last cwd.
    pub cwd: Option<String>,
    pub worktree: Option<String>,
    pub parent_agent_id: Option<String>,
    pub workflow_id: Option<String>,
    pub workflow_phase: Option<String>,
    pub model: Option<String>,
    pub active: bool,
    /// Finished (last message with `end_turn`). `active = false && !finished` = no
    /// activity for over 10 min, or its session is no longer running.
    pub finished: bool,
    pub last_activity_at: Option<i64>,
    pub last_tool: Option<String>,
    pub last_tool_summary: Option<String>,
    /// Owning session.
    pub session_name: Option<String>,
    pub session_kind: String,
    pub session_cwd: Option<String>,
    pub session_is_app_run: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoActivity {
    pub repo_path: String,
    pub sessions: Vec<SessionActivity>,
    pub subagents: Vec<SubagentActivity>,
    /// Epoch ms of the computation.
    pub generated_at: i64,
}

/// Runs launched by the app: short ids and sessionIds.
#[derive(Debug, Default, Clone)]
pub struct AppRuns {
    pub run_ids: HashSet<String>,
    pub session_ids: HashSet<String>,
}

impl AppRuns {
    pub fn contains(&self, id: Option<&str>, session_id: &str) -> bool {
        self.session_ids.contains(session_id) || id.is_some_and(|i| self.run_ids.contains(i))
    }
    pub fn add(&mut self, run_id: Option<String>, session_id: Option<String>) {
        if let Some(r) = run_id {
            self.run_ids.insert(r);
        }
        if let Some(s) = session_id {
            self.session_ids.insert(s);
        }
    }
}

/// Sessions started outside the app in one repo.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSessions {
    pub repo_id: String,
    pub sessions: Vec<SessionActivity>,
    pub subagents: Vec<SubagentActivity>,
}

/// Sessions started outside the app, per repo (only repos with something to show).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSessions {
    pub repos: Vec<RepoSessions>,
    pub generated_at: i64,
}

/// Deserializes leniently: an unexpected shape becomes `None` instead of failing the whole
/// document. `pub` (beyond `AgentSession`'s own needs) because `claude_sessions::SubagentMeta`,
/// still in the shell, also derives with it.
pub fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

/// A session as reported by `claude agents` (interactive or background).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    #[serde(default, deserialize_with = "lenient")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub session_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub cwd: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub pid: Option<u32>,
    #[serde(default, deserialize_with = "lenient")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub state: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub waiting_for: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub started_at: Option<f64>,
}

impl AgentSession {
    /// Live process (`pid`), or a background session that `claude agents` reports as active.
    pub fn alive(&self) -> bool {
        self.pid.is_some() || matches!(self.state.as_deref(), Some("working") | Some("blocked"))
    }
}
