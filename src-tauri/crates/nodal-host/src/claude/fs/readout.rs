//! Reading a session's readout for the app: workflow detail, last message and launch
//! blocker, in one pass.

use std::path::{Path, PathBuf};

use nodal_domain::model::claude::SessionReadout;
use nodal_domain::sessions::transcript::is_valid_session_id;

use super::paths::{claude_config_dir, find_session_dir, find_session_jsonl};
use super::review_denial::{
    find_workflow_review_denial, last_assistant_text_in, read_last_assistant_text,
    read_workflow_review_denial, BLOCKER_SCAN_BYTES,
};
use super::usage::{read_usage_tokens, usage_tokens_in};
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
    let Ok(projects) = projects_dir() else {
        return SessionReadout::default();
    };
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

/// `read_session` + `session_tokens` together, from a single read of the main transcript
/// (P12): closing a run used to open it 3 times (tokens, last message, blocker).
pub fn read_session_close(session_id: &str, cwd: &str) -> (SessionReadout, Option<i64>) {
    if !is_valid_session_id(session_id) {
        return (SessionReadout::default(), None);
    }
    let Ok(projects) = projects_dir() else {
        return (SessionReadout::default(), None);
    };
    let detail = find_session_dir(&projects, cwd, session_id).and_then(|d| read_run_detail(&d));
    let Some(jsonl) = find_session_jsonl(&projects, cwd, session_id) else {
        return (
            SessionReadout {
                detail,
                last_message: None,
                blocker: None,
            },
            None,
        );
    };
    let (last_message, blocker, tokens) = scan_jsonl(&jsonl);
    (
        SessionReadout {
            detail,
            last_message,
            blocker,
        },
        tokens,
    )
}

/// The three fields `read_session_close` needs from the main transcript, computed over the
/// same in-memory text instead of one file open per field. The blocker only looks at the
/// same head window `read_workflow_review_denial` does (the `Workflow` call comes early on):
/// scanning the rest of a large transcript for it measured slower than the 3 separate reads
/// it replaces, for no benefit (the call is never past the first few turns).
fn scan_jsonl(path: &Path) -> (Option<String>, Option<Option<String>>, Option<i64>) {
    let Ok(text) = super::read_to_string_lossy(path) else {
        return (None, None, None);
    };
    (
        last_assistant_text_in(&text),
        find_workflow_review_denial(head(&text, BLOCKER_SCAN_BYTES)),
        usage_tokens_in(text.lines()),
    )
}

/// The text's first `max` bytes, moved back to the nearest char boundary.
fn head(text: &str, max: u64) -> &str {
    let mut cap = (max as usize).min(text.len());
    while !text.is_char_boundary(cap) {
        cap -= 1;
    }
    &text[..cap]
}

#[cfg(test)]
mod tests;
