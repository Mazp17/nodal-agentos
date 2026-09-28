//! Why a background session never got to start its workflow: Claude Code rejected the
//! `Workflow` tool because nobody approved it (there's no `/workflows` prompt in `--bg`).

use std::fs;
use std::io::Read;
use std::path::Path;

use serde_json::Value;

use super::transcript::{read_transcript_text, tool_result_text};

/// Text Claude Code (2.1.281) uses to reject the `Workflow` tool when the workflow is new
/// or changed and nobody approved it in `/workflows`. In a `--bg` session there's nobody to
/// ask: it stays as a `tool_result` with `is_error` and the session ends without a workflow.
pub const WORKFLOW_REVIEW_TEXT: &str = "Review dynamic workflow before running";
/// How much of the transcript's head is read: the `Workflow` call comes early on.
const BLOCKER_SCAN_BYTES: u64 = 4 * 1024 * 1024;

/// Cap on the returned last message (the final JSON block goes at the end).
const LAST_TEXT_MAX: usize = 64 * 1024;

/// Text of the last assistant message in the lines of a session transcript: the `text`
/// blocks of the last turn with text, joined. Ignores sidechains (subagents).
pub fn last_assistant_text_in(text: &str) -> Option<String> {
    let mut last: Option<String> = None;
    for line in text.lines() {
        if !line.contains("\"assistant\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("type").and_then(Value::as_str) != Some("assistant")
            || v.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(Value::Array(blocks)) = v.get("message").and_then(|m| m.get("content")) else { continue };
        let joined: Vec<&str> = blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !joined.is_empty() {
            last = Some(joined.join("\n\n"));
        }
    }
    last.map(|s| {
        if s.len() <= LAST_TEXT_MAX {
            return s;
        }
        // Keep the end, which is where the JSON block goes.
        let mut start = s.len() - LAST_TEXT_MAX;
        while !s.is_char_boundary(start) {
            start += 1;
        }
        s[start..].to_string()
    })
}

/// Last assistant message of the main transcript (`<sid>.jsonl`). Reads at most the
/// file's tail (see `super::transcript::read_transcript_text`).
pub fn read_last_assistant_text(path: &Path) -> Option<String> {
    let (text, _, _) = read_transcript_text(path).ok()?;
    last_assistant_text_in(&text)
}

/// Searches the lines of a session transcript for the `Workflow` call rejected for lack
/// of approval. `Some(name)` if found (the name may be missing).
pub fn find_workflow_review_denial(text: &str) -> Option<Option<String>> {
    // tool_use id → name of the requested workflow.
    let mut calls: Vec<(String, Option<String>)> = Vec::new();
    for line in text.lines() {
        let has_call = line.contains("\"Workflow\"");
        let has_denial = line.contains(WORKFLOW_REVIEW_TEXT);
        if !has_call && !has_denial {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else { continue };
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") if b.get("name").and_then(Value::as_str) == Some("Workflow") => {
                    if let Some(id) = b.get("id").and_then(Value::as_str) {
                        let name = b.pointer("/input/name").and_then(Value::as_str).map(str::to_string);
                        calls.push((id.to_string(), name));
                    }
                }
                Some("tool_result") if tool_result_text(b).contains(WORKFLOW_REVIEW_TEXT) => {
                    let id = b.get("tool_use_id").and_then(Value::as_str);
                    let name = calls.iter().find(|(c, _)| Some(c.as_str()) == id).and_then(|(_, n)| n.clone());
                    return Some(name);
                }
                _ => {}
            }
        }
    }
    None
}

/// Reads the head of the transcript and looks for the rejection (a cut-off last line
/// doesn't parse as JSON and is ignored).
pub fn read_workflow_review_denial(path: &Path) -> Option<Option<String>> {
    let file = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(BLOCKER_SCAN_BYTES).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    find_workflow_review_denial(&text)
}

#[cfg(test)]
mod tests;
