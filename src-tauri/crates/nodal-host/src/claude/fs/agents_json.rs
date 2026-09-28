//! `claude agents --json --all` output.

use serde::Deserialize;
use serde_json::Value;

use nodal_domain::model::claude::RunSummary;

use super::lenient;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAgent {
    #[serde(default, deserialize_with = "lenient")]
    id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    session_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    cwd: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    started_at: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pid: Option<u32>,
    #[serde(default, deserialize_with = "lenient")]
    status: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    state: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    waiting_for: Option<String>,
}

/// Sessions with `kind == "background"`, most recent first. Entries without `id` or
/// `sessionId` are dropped (interactive ones, for example, have no `id`).
pub fn parse_agents_json(text: &str) -> Result<Vec<RunSummary>, String> {
    let values: Vec<Value> = serde_json::from_str(text.trim())
        .map_err(|e| format!("`claude agents --json` didn't return the expected JSON list: {e}"))?;
    let mut runs: Vec<RunSummary> = values
        .into_iter()
        .filter_map(|v| serde_json::from_value::<RawAgent>(v).ok())
        .filter(|a| a.kind.as_deref() == Some("background"))
        .filter_map(|a| {
            Some(RunSummary {
                id: a.id?,
                session_id: a.session_id?,
                cwd: a.cwd,
                name: a.name,
                started_at: a.started_at.map(|n| n as i64),
                pid: a.pid,
                status: a.status,
                state: a.state,
                waiting_for: a.waiting_for,
            })
        })
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    Ok(runs)
}

#[cfg(test)]
mod tests;
