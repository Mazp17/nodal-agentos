//! Parsing of the final JSON block the prompt asks executors for:
//! - agent/Claude: `{"status":"done|blocked","summary":…,"pr":…,"branch":…}`;
//! - reviewer: `{"verdict":"pass|fail","unmet":[…],"nits":[…],"summary":…}`.
//!
//! It looks for the last JSON object in the message that has the expected key (inside a
//! ```json block or loose). Anything unknown is ignored.

use serde_json::Value;

use crate::domain::Verdict;
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
    let Some(Value::Array(items)) = map.get(key) else { return Vec::new() };
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
    s.filter(|u| (u.starts_with("https://") || u.starts_with("http://")) && !u.chars().any(char::is_whitespace))
}

/// A reasonable branch name (no spaces or anything that looks like an option).
pub fn clean_branch(s: Option<String>) -> Option<String> {
    s.filter(|b| {
        b.len() <= 200 && !b.starts_with('-') && !b.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

pub fn parse_agent_report(message: &str) -> Option<AgentReport> {
    let map = last_json_with_key(message, "status")?;
    let status = match map.get("status").and_then(Value::as_str).map(|s| s.trim().to_ascii_lowercase()) {
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
    let pass = match map.get("verdict").and_then(Value::as_str).map(|s| s.trim().to_ascii_lowercase()) {
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
mod tests {
    use super::*;

    #[test]
    fn agent_report_from_fenced_block_at_the_end() {
        let msg = "Done. I changed the header.\n\nIntermediate example: {\"status\": \"blocked\"}\n\n```json\n{\"status\": \"done\", \"summary\": \"New logo in the header\", \"pr\": \"https://example.com/acme/web/pull/7\", \"branch\": \"nodal/pay-1-logo\"}\n```\n";
        let r = parse_agent_report(msg).unwrap();
        assert_eq!(r.status, ReportStatus::Done);
        assert_eq!(r.summary.as_deref(), Some("New logo in the header"));
        assert_eq!(r.pr.as_deref(), Some("https://example.com/acme/web/pull/7"));
        assert_eq!(r.branch.as_deref(), Some("nodal/pay-1-logo"));
    }

    #[test]
    fn agent_report_blocked_loose_json_and_bad_fields() {
        let msg = "I couldn't run the tests. {\"status\":\"BLOCKED\",\"summary\":\"the DB is missing\",\"pr\":\"javascript:alert(1)\",\"branch\":\"--force\"}";
        let r = parse_agent_report(msg).unwrap();
        assert_eq!(r.status, ReportStatus::Blocked);
        assert_eq!(r.pr, None);
        assert_eq!(r.branch, None);
        assert_eq!(parse_agent_report("no report"), None);
        assert_eq!(parse_agent_report("{\"status\": \"maybe\"}"), None);
        assert_eq!(parse_agent_report("{\"status\": \"done\", \"pr\": null}").unwrap().pr, None);
        // Broken JSON at the end: the last valid one is used.
        let r = parse_agent_report("{\"status\":\"done\",\"summary\":\"ok\"} and then {\"status\": ").unwrap();
        assert_eq!(r.summary.as_deref(), Some("ok"));
    }

    #[test]
    fn verdict_pass_and_fail() {
        let v = parse_verdict("I reviewed everything.\n```json\n{\"verdict\":\"pass\",\"unmet\":[],\"nits\":[\"rename x\"],\"summary\":\"Meets the criteria\"}\n```").unwrap();
        assert!(v.pass);
        assert!(v.unmet.is_empty());
        assert_eq!(v.nits, ["rename x"]);
        assert_eq!(v.summary.as_deref(), Some("Meets the criteria"));
        let v = parse_verdict("{\"verdict\":\"fail\",\"unmet\":[\"The logo doesn't show on mobile\", {\"id\": 2}],\"summary\":null}").unwrap();
        assert!(!v.pass);
        assert_eq!(v.unmet, ["The logo doesn't show on mobile", "{\"id\":2}"]);
        assert!(v.nits.is_empty());
        assert_eq!(v.summary, None);
        assert_eq!(parse_verdict("{\"verdict\":\"ok\"}"), None);
        assert_eq!(parse_verdict("nothing"), None);
    }

    #[test]
    fn reads_last_message_from_session_lines() {
        // Real format observed in the spike (2.1.281): `assistant` lines with `text` blocks.
        let lines = [
            r#"{"type":"agent-setting","agentSetting":"frontend-developer"}"#,
            r#"{"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"Starting."}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"subagent"}]}}"#,
            r#"{"type":"assistant","isSidechain":false,"message":{"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#,
            r#"{"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"Done.\n```json\n{\"status\":\"done\",\"summary\":\"s\"}\n```"}]}}"#,
            r#"{"type":"system","subtype":"turn_duration"}"#,
        ]
        .join("\n");
        let last = crate::runs::claude_fs::last_assistant_text_in(&lines).unwrap();
        assert!(last.starts_with("Done."));
        assert_eq!(parse_agent_report(&last).unwrap().status, ReportStatus::Done);
        assert_eq!(crate::runs::claude_fs::last_assistant_text_in("{\"type\":\"user\"}"), None);
    }
}
