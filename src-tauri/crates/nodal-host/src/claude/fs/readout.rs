//! Reading a session's readout for the app: workflow detail, last message and launch
//! blocker, in one pass.

use std::path::PathBuf;

use nodal_domain::model::claude::SessionReadout;
use nodal_domain::sessions::transcript::is_valid_session_id;

use super::paths::{claude_config_dir, find_session_dir, find_session_jsonl};
use super::review_denial::{read_last_assistant_text, read_workflow_review_denial};
use super::usage::read_usage_tokens;
use super::workflow_detail::read_run_detail;

pub fn projects_dir() -> Result<PathBuf, String> {
    Ok(claude_config_dir()
        .ok_or("Couldn't locate the Claude Code folder ($HOME is not set).")?
        .join("projects"))
}

pub fn read_session(session_id: &str, cwd: &str) -> SessionReadout {
    if !is_valid_session_id(session_id) {
        return SessionReadout::default();
    }
    let Ok(projects) = projects_dir() else { return SessionReadout::default() };
    let detail = find_session_dir(&projects, cwd, session_id).and_then(|d| read_run_detail(&d));
    let jsonl = find_session_jsonl(&projects, cwd, session_id);
    SessionReadout {
        detail,
        last_message: jsonl.as_deref().and_then(read_last_assistant_text),
        blocker: jsonl.as_deref().and_then(read_workflow_review_denial),
    }
}

/// Tokens of the session's main transcript (blocking). `None` if there's no file or usage.
pub fn session_tokens(session_id: &str, cwd: &str) -> Option<i64> {
    if !is_valid_session_id(session_id) {
        return None;
    }
    let projects = projects_dir().ok()?;
    read_usage_tokens(&find_session_jsonl(&projects, cwd, session_id)?)
}
