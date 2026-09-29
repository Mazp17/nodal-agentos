//! Parsing of the final JSON block the prompt asks executors for:
//! - agent/Claude: `{"status":"done|blocked","summary":…,"pr":…,"branch":…}`;
//! - reviewer: `{"verdict":"pass|fail","unmet":[…],"nits":[…],"summary":…}`.
//!
//! It looks for the last JSON object in the message that has the expected key (inside a
//! ```json block or loose). Anything unknown is ignored.

use serde_json::Value;

use crate::model::Verdict;
use crate::util::clip_chars;

const MAX_CANDIDATES: usize = 400;
const SUMMARY_MAX: usize = 4000;
const ITEM_MAX: usize = 1000;
const LIST_MAX: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportStatus {
    Done,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReport {
    pub status: ReportStatus,
    pub summary: Option<String>,
    pub pr: Option<String>,
    pub branch: Option<String>,
}

/// Last JSON object in `text` with the key `key`.
pub fn last_json_with_key(text: &str, key: &str) -> Option<serde_json::Map<String, Value>> {
    let starts: Vec<usize> = text.match_indices('{').map(|(i, _)| i).collect();
    for &i in starts.iter().rev().take(MAX_CANDIDATES) {
        let mut it = serde_json::Deserializer::from_str(&text[i..]).into_iter::<Value>();
        if let Some(Ok(Value::Object(map))) = it.next() {
            if map.contains_key(key) {
                return Some(map);
            }
        }
    }
    None
}

fn text_field(map: &serde_json::Map<String, Value>, key: &str, max: usize) -> Option<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "null")
        .map(|s| clip_chars(s, max))
}

fn list_field(map: &serde_json::Map<String, Value>, key: &str) -> Vec<String> {
    let Some(Value::Array(items)) = map.get(key) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s.trim().to_string()),
            Value::Null => None,
            other => Some(other.to_string()),
        })
        .filter(|s| !s.is_empty())
        .take(LIST_MAX)
        .map(|s| clip_chars(&s, ITEM_MAX))
        .collect()
}

/// PR URL only if it's http(s) (it ends up as a link in the UI).
pub fn clean_url(s: Option<String>) -> Option<String> {
    s.filter(|u| {
        (u.starts_with("https://") || u.starts_with("http://"))
            && !u.chars().any(char::is_whitespace)
    })
}

/// A reasonable branch name (no spaces or anything that looks like an option).
pub fn clean_branch(s: Option<String>) -> Option<String> {
    s.filter(|b| {
        b.len() <= 200
            && !b.starts_with('-')
            && !b.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

pub fn parse_agent_report(message: &str) -> Option<AgentReport> {
    let map = last_json_with_key(message, "status")?;
    let status = match map
        .get("status")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
    {
        Some(s) if s == "done" => ReportStatus::Done,
        Some(s) if s == "blocked" => ReportStatus::Blocked,
        _ => return None,
    };
    Some(AgentReport {
        status,
        summary: text_field(&map, "summary", SUMMARY_MAX),
        pr: clean_url(text_field(&map, "pr", 500)),
        branch: clean_branch(text_field(&map, "branch", 200)),
    })
}

pub fn parse_verdict(message: &str) -> Option<Verdict> {
    let map = last_json_with_key(message, "verdict")?;
    let pass = match map
        .get("verdict")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
    {
        Some(s) if s == "pass" => true,
        Some(s) if s == "fail" => false,
        _ => return None,
    };
    Some(Verdict {
        pass,
        unmet: list_field(&map, "unmet"),
        nits: list_field(&map, "nits"),
        summary: text_field(&map, "summary", SUMMARY_MAX),
    })
}

#[cfg(test)]
mod tests;
