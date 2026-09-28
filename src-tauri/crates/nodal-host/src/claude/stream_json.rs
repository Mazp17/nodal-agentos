//! WARNING: Claude Code's `-p` stream-json protocol, partly UNDOCUMENTED (observed in
//! v2.1.283; see `stream_json_live` for the spike and `fixtures/stream_json/`).
//!
//! The only place that knows the shape of what a chat's `claude` process prints and of what
//! Nodal writes to its stdin: launch flags, event parsing and the stdin messages.
//! Content blocks share their shape with session files, so they go through `fs::transcript`.
//!
//! Policy: tolerate anything unknown and skip it instead of failing.

use serde_json::{json, Value};

use nodal_domain::model::chat::{PermissionRequest, StreamEvent};
use nodal_domain::sessions::transcript::user_item;

use super::fs::transcript::{assistant_item, single_result, tool_input_summary, tool_result_info};

/// Flags every chat process starts with, each one its own argument. `--output-format
/// stream-json` with `-p` requires `--verbose`; `--permission-prompt-tool stdio` makes
/// permission prompts reach the host as `can_use_tool` requests (without it, and with the
/// default `--permission-prompts host`, they are denied); `--replay-user-messages` echoes
/// each stdin message back (with `isReplay`), so the UI gets it in order. Verified with 2.1.283.
pub const CHAT_ARGS: [&str; 10] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--include-partial-messages",
    "--permission-prompt-tool",
    "stdio",
    "--replay-user-messages",
];

/// Env var set on every chat process so its session is listed by the bare `claude --resume`
/// picker, which hides `sdk-cli`/`sdk-ts`/`sdk-py` sessions. Without it `-p` records
/// `sdk-cli`, and `cli` is rewritten to `sdk-cli` under `-p`, so the value must be neither.
/// Verified with 2.1.283.
pub const CHAT_ENTRYPOINT: (&str, &str) = ("CLAUDE_CODE_ENTRYPOINT", "nodal");

/// `--resume <session-id>`: keeps the id and the history, also from another cwd (verified
/// with 2.1.283). The id must already be checked with `is_valid_session_id`.
pub fn resume_args(session_id: &str) -> [String; 2] {
    ["--resume".into(), session_id.into()]
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn context_tokens(usage: &Value) -> Option<i64> {
    let n: i64 = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
        .iter()
        .filter_map(|k| usage.get(*k).and_then(Value::as_i64))
        .sum();
    (n > 0).then_some(n)
}

/// Parses one stdout line. Most lines give zero or one event; an assistant or user message
/// gives one per content block.
pub fn parse_line(line: &str) -> Vec<StreamEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return vec![] };
    match v.get("type").and_then(Value::as_str) {
        Some("system") => match v.get("subtype").and_then(Value::as_str) {
            Some("init") => str_of(&v, "session_id")
                .map(|session_id| StreamEvent::Init {
                    session_id,
                    model: str_of(&v, "model"),
                    cwd: str_of(&v, "cwd"),
                    permission_mode: str_of(&v, "permissionMode"),
                })
                .into_iter()
                .collect(),
            Some("permission_denied") => vec![StreamEvent::PermissionDenied {
                tool_name: str_of(&v, "tool_name").unwrap_or_default(),
                tool_use_id: str_of(&v, "tool_use_id"),
                message: str_of(&v, "message"),
            }],
            _ => vec![],
        },
        Some("stream_event") => {
            let Some(delta) = v.pointer("/event/delta") else { return vec![] };
            if v.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
                return vec![];
            }
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => str_of(delta, "text").map(|text| StreamEvent::TextDelta { text }).into_iter().collect(),
                Some("thinking_delta") => {
                    str_of(delta, "thinking").map(|text| StreamEvent::ThinkingDelta { text }).into_iter().collect()
                }
                _ => vec![],
            }
        }
        Some("assistant") => {
            // Subagent (`Task`) traffic is not part of the chat's own conversation.
            if v.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
                return vec![];
            }
            let blocks = v.pointer("/message/content").and_then(Value::as_array);
            let mut out: Vec<StreamEvent> = blocks
                .into_iter()
                .flatten()
                .filter_map(assistant_item)
                .map(|item| StreamEvent::Item { item })
                .collect();
            if let Some(n) = v.pointer("/message/usage").and_then(context_tokens) {
                out.push(StreamEvent::Usage { context_tokens: n });
            }
            out
        }
        Some("user") => {
            if v.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
                return vec![];
            }
            match v.pointer("/message/content") {
                Some(Value::String(s)) if !s.trim().is_empty() => {
                    vec![StreamEvent::Item { item: user_item(s) }]
                }
                Some(Value::Array(blocks)) => {
                    let structured = single_result(blocks, v.get("tool_use_result"));
                    blocks
                        .iter()
                        .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                            Some("tool_result") => {
                                let (id, result) = tool_result_info(b, structured);
                                Some(StreamEvent::ToolResult { tool_use_id: id?.to_string(), result })
                            }
                            Some("text") => {
                                let s = b.get("text").and_then(Value::as_str).filter(|s| !s.trim().is_empty())?;
                                Some(StreamEvent::Item { item: user_item(s) })
                            }
                            _ => None,
                        })
                        .collect()
                }
                _ => vec![],
            }
        }
        Some("control_request") if v.pointer("/request/subtype").and_then(Value::as_str) == Some("can_use_tool") => {
            let (Some(request_id), Some(req)) = (str_of(&v, "request_id"), v.get("request")) else { return vec![] };
            let input = req.get("input").cloned().unwrap_or_else(|| json!({}));
            vec![StreamEvent::PermissionRequest(PermissionRequest {
                request_id,
                tool_name: str_of(req, "tool_name").unwrap_or_default(),
                tool_use_id: str_of(req, "tool_use_id"),
                description: str_of(req, "description"),
                summary: tool_input_summary(&input),
                input,
            })]
        }
        Some("result") => {
            let context_window = v
                .get("modelUsage")
                .and_then(Value::as_object)
                .and_then(|m| m.values().filter_map(|u| u.get("contextWindow").and_then(Value::as_i64)).max());
            vec![StreamEvent::TurnEnd {
                ok: !v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                subtype: str_of(&v, "subtype").unwrap_or_default(),
                session_id: str_of(&v, "session_id"),
                result: str_of(&v, "result"),
                cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
                context_window,
            }]
        }
        _ => vec![],
    }
}

/// A user message for stdin (one line, without the newline).
pub fn user_message(text: &str) -> String {
    json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
}

/// The answer to a `can_use_tool` request. Allowing sends the original input back
/// (`updatedInput` is required); a denial's message reaches the model as the tool's error.
pub fn permission_response(request_id: &str, allow: Option<&Value>, deny_message: &str) -> String {
    let response = match allow {
        Some(input) => json!({"behavior": "allow", "updatedInput": input}),
        None => json!({"behavior": "deny", "message": deny_message}),
    };
    json!({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": response}})
        .to_string()
}

/// Stops the current turn; the process stays alive for the next message.
pub fn interrupt_request(request_id: &str) -> String {
    json!({"type": "control_request", "request_id": request_id, "request": {"subtype": "interrupt"}}).to_string()
}

#[cfg(test)]
mod tests;
