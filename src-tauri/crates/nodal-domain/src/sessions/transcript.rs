//! Validators and text helpers for reading Claude Code session/subagent transcripts. The
//! parsing that depends on the on-disk format lives in nodal-host.

use crate::model::claude::TranscriptItem;

/// A sessionId comes from the frontend and ends up in a path: only `[A-Za-z0-9-]`.
pub fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Ids that end up in a path (`wf_...`, agent id): only `[A-Za-z0-9_-]`.
pub fn is_valid_path_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub const TRANSCRIPT_DEFAULT_LIMIT: u32 = 200;
pub const TRANSCRIPT_MAX_LIMIT: u32 = 2000;
pub const TEXT_MAX: usize = 4000;

/// Clips to `max` characters (with `…`) and reports whether it clipped.
pub fn clip(s: &str, max: usize) -> (String, bool) {
    match s.char_indices().nth(max) {
        None => (s.to_string(), false),
        Some((cut, _)) => {
            let mut out = s[..cut].to_string();
            out.push('…');
            (out, true)
        }
    }
}

/// A user text as a transcript item, clipped like the rest.
pub fn user_item(s: &str, uuid: Option<String>) -> TranscriptItem {
    let (text, truncated) = clip(s, TEXT_MAX);
    TranscriptItem::User { text, truncated, uuid }
}

/// Decoded size of standard, padded base64; `None` if it isn't valid base64.
pub fn base64_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(4) {
        return None;
    }
    let pad = b.iter().rev().take_while(|&&c| c == b'=').count();
    let ok = pad <= 2 && b[..b.len() - pad].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'+' || *c == b'/');
    ok.then(|| b.len() / 4 * 3 - pad)
}

/// A message uuid reaches a CLI argument (`--resume-session-at`, `rewind_files`): only hex and dashes.
pub fn is_valid_message_uuid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

#[cfg(test)]
mod tests;
