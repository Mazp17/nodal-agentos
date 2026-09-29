//! Executor and workflow catalog DTOs, plus their name validators. Reading the catalog from
//! disk (`~/.claude/agents`, `~/.claude/workflows`, plugins) lives in nodal-host.

use serde::Serialize;

use crate::model::{AgentSource, Executor};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutorInfo {
    pub executor: Executor,
    pub description: Option<String>,
    pub tools: Option<Vec<String>>,
    pub manages_source: Option<String>,
    pub reviews: bool,
    pub path: Option<String>,
    pub source: Option<AgentSource>,
}

/// Name usable in `--agent <name>` (with `plugin:agent` for plugin agents).
pub fn is_valid_agent_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentDef {
    /// Name for `--agent` (`plugin:agent` for plugin agents).
    pub name: String,
    pub source: AgentSource,
    pub description: Option<String>,
    pub tools: Option<Vec<String>>,
    pub path: std::path::PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowSource {
    User,
    Repo,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInfo {
    pub name: String,
    pub description: Option<String>,
    pub when_to_use: Option<String>,
    /// Provider the workflow syncs on its own (`"linear"`): the app doesn't push status to it
    /// or comment.
    pub manages_source: Option<String>,
    /// The workflow already reviews (its result is the verdict): Nodal's gate is skipped.
    pub reviews: bool,
    pub source: WorkflowSource,
    pub path: String,
}

/// Name usable in `/<workflow> ...`: no spaces or anything `claude` would read as an option.
pub fn is_valid_workflow_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

#[cfg(test)]
mod tests;
