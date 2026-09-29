//! WARNING: Claude Code's `-p` stream-json protocol, partly UNDOCUMENTED (observed in
//! v2.1.283; see `stream_json_live` for the spike and `fixtures/stream_json/`).
//!
//! The only place that knows the shape of what a chat's `claude` process prints and of what
//! Nodal writes to its stdin: launch flags, event parsing and the stdin messages.
//! Content blocks share their shape with session files, so they go through `fs::transcript`.
//!
//! Policy: tolerate anything unknown and skip it instead of failing.
//!
//! `parse_line` deserializes into typed structs instead of poking a generic `Value` tree:
//! simple fields borrow (`Cow<str>`, zero-copy unless the JSON has escapes), and the nested
//! shapes `fs::transcript`'s helpers still need as `&Value` (`message.content`, `message.usage`,
//! `tool_use_result`, a `can_use_tool` request's `input`) stay as `&RawValue` — an unparsed
//! slice of the line — until the code that actually uses them converts it. That keeps the (by
//! far most frequent) `stream_event`/`*_delta` lines from ever building a `Value` tree.
//!
//! Dispatch is two steps, not one `#[serde(tag = "type")]` enum: `serde_json`'s internally
//! tagged enums deserialize a variant's fields from a buffered `Content` value, which can't
//! carry `&RawValue`'s "keep this slice raw" marker through it (`Content` always parses/owns).
//! So `parse_line` first peeks `type` (a one-field struct, cheap), then deserializes the whole
//! line straight into the shape for that type — `&RawValue` borrows correctly there because
//! nothing buffers the line in between.
use std::borrow::Cow;
use std::collections::HashMap;

use serde::Deserialize;
use serde_json::value::RawValue;
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

// ---------- Wire shapes ----------
//
// Every field is optional/defaulted so an unexpected or partial shape falls through to the
// same "skip it" branches the old `Value` code had, instead of turning a whole line into a
// hard error.

/// First pass: just the discriminant.
#[derive(Deserialize)]
struct Tagged<'a> {
    #[serde(borrow, rename = "type")]
    kind: Cow<'a, str>,
}

#[derive(Deserialize)]
struct RawSystem<'a> {
    #[serde(default, borrow)]
    subtype: Cow<'a, str>,
    #[serde(default, borrow)]
    session_id: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    model: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    cwd: Option<Cow<'a, str>>,
    #[serde(default, borrow, rename = "permissionMode")]
    permission_mode: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    tool_name: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    tool_use_id: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    message: Option<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct RawStreamEventLine<'a> {
    #[serde(default, borrow, rename = "parent_tool_use_id")]
    parent_tool_use_id: Option<Cow<'a, str>>,
    #[serde(default)]
    event: Option<RawEventField<'a>>,
}

#[derive(Deserialize)]
struct RawEventField<'a> {
    #[serde(default, borrow)]
    delta: Option<RawDelta<'a>>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RawDelta<'a> {
    TextDelta {
        #[serde(borrow)]
        text: Cow<'a, str>,
    },
    ThinkingDelta {
        #[serde(borrow)]
        thinking: Cow<'a, str>,
    },
    #[serde(other)]
    Other,
}

/// `assistant`. `message.content`/`message.usage` are deferred (`&RawValue`): only turned into
/// a `Value` (for `fs::transcript`'s helpers) once we know the line isn't subagent traffic.
#[derive(Deserialize)]
struct RawAssistant<'a> {
    #[serde(default, borrow, rename = "parent_tool_use_id")]
    parent_tool_use_id: Option<Cow<'a, str>>,
    #[serde(default)]
    message: Option<RawContentMessage<'a>>,
}

#[derive(Deserialize)]
struct RawContentMessage<'a> {
    #[serde(default, borrow)]
    content: Option<&'a RawValue>,
    #[serde(default, borrow)]
    usage: Option<&'a RawValue>,
}

/// `user`. `message.content` is a plain string (a replayed prompt) or an array of blocks —
/// either way turned into a `Value` once we're past the subagent check, since that's what
/// tells the two shapes apart.
#[derive(Deserialize)]
struct RawUser<'a> {
    #[serde(default, borrow, rename = "parent_tool_use_id")]
    parent_tool_use_id: Option<Cow<'a, str>>,
    #[serde(default)]
    message: Option<RawUserMessage<'a>>,
    #[serde(default, borrow, rename = "tool_use_result")]
    tool_use_result: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct RawUserMessage<'a> {
    #[serde(default, borrow)]
    content: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct RawControlRequestLine<'a> {
    #[serde(default, borrow)]
    request_id: Option<Cow<'a, str>>,
    #[serde(default)]
    request: Option<RawControlRequest<'a>>,
}

#[derive(Deserialize)]
struct RawControlRequest<'a> {
    #[serde(default, borrow)]
    subtype: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    tool_name: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    tool_use_id: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    description: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    input: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct RawResult<'a> {
    #[serde(default)]
    subtype: Cow<'a, str>,
    #[serde(default, borrow)]
    session_id: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    result: Option<Cow<'a, str>>,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default, rename = "modelUsage")]
    model_usage: Option<HashMap<Cow<'a, str>, RawModelUsage>>,
}

#[derive(Deserialize)]
struct RawModelUsage {
    #[serde(default, rename = "contextWindow")]
    context_window: Option<i64>,
}

fn to_value(raw: &RawValue) -> Option<Value> {
    serde_json::from_str(raw.get()).ok()
}

fn context_tokens(usage: &Value) -> Option<i64> {
    let n: i64 = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ]
    .iter()
    .filter_map(|k| usage.get(*k).and_then(Value::as_i64))
    .sum();
    (n > 0).then_some(n)
}

fn parse_system(line: &str) -> Vec<StreamEvent> {
    let Ok(v) = serde_json::from_str::<RawSystem>(line) else {
        return vec![];
    };
    match v.subtype.as_ref() {
        "init" => v
            .session_id
            .map(|session_id| StreamEvent::Init {
                session_id: session_id.into_owned(),
                model: v.model.map(Cow::into_owned),
                cwd: v.cwd.map(Cow::into_owned),
                permission_mode: v.permission_mode.map(Cow::into_owned),
            })
            .into_iter()
            .collect(),
        "permission_denied" => vec![StreamEvent::PermissionDenied {
            tool_name: v.tool_name.map(Cow::into_owned).unwrap_or_default(),
            tool_use_id: v.tool_use_id.map(Cow::into_owned),
            message: v.message.map(Cow::into_owned),
        }],
        _ => vec![],
    }
}

fn parse_stream_event(line: &str) -> Vec<StreamEvent> {
    let Ok(v) = serde_json::from_str::<RawStreamEventLine>(line) else {
        return vec![];
    };
    let Some(delta) = v.event.and_then(|e| e.delta) else {
        return vec![];
    };
    if v.parent_tool_use_id.is_some() {
        return vec![];
    }
    match delta {
        RawDelta::TextDelta { text } => vec![StreamEvent::TextDelta {
            text: text.into_owned(),
        }],
        RawDelta::ThinkingDelta { thinking } => vec![StreamEvent::ThinkingDelta {
            text: thinking.into_owned(),
        }],
        RawDelta::Other => vec![],
    }
}

fn parse_assistant(line: &str) -> Vec<StreamEvent> {
    let Ok(v) = serde_json::from_str::<RawAssistant>(line) else {
        return vec![];
    };
    // Subagent (`Task`) traffic is not part of the chat's own conversation.
    if v.parent_tool_use_id.is_some() {
        return vec![];
    }
    let Some(message) = v.message else {
        return vec![];
    };
    let content = message.content.and_then(to_value);
    let mut out: Vec<StreamEvent> = content
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(assistant_item)
        .map(|item| StreamEvent::Item { item })
        .collect();
    let usage = message.usage.and_then(to_value);
    if let Some(n) = usage.as_ref().and_then(context_tokens) {
        out.push(StreamEvent::Usage { context_tokens: n });
    }
    out
}

fn parse_user(line: &str) -> Vec<StreamEvent> {
    let Ok(v) = serde_json::from_str::<RawUser>(line) else {
        return vec![];
    };
    if v.parent_tool_use_id.is_some() {
        return vec![];
    }
    let Some(content) = v.message.and_then(|m| m.content).and_then(to_value) else {
        return vec![];
    };
    match content {
        Value::String(s) if !s.trim().is_empty() => {
            vec![StreamEvent::Item { item: user_item(&s) }]
        }
        Value::Array(blocks) => {
            let tool_use_result = v.tool_use_result.and_then(to_value);
            let structured = single_result(&blocks, tool_use_result.as_ref());
            blocks
                .iter()
                .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                    Some("tool_result") => {
                        let (id, result) = tool_result_info(b, structured);
                        Some(StreamEvent::ToolResult {
                            tool_use_id: id?.to_string(),
                            result,
                        })
                    }
                    Some("text") => {
                        let s = b
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|s| !s.trim().is_empty())?;
                        Some(StreamEvent::Item { item: user_item(s) })
                    }
                    _ => None,
                })
                .collect()
        }
        _ => vec![],
    }
}

fn parse_control_request(line: &str) -> Vec<StreamEvent> {
    let Ok(v) = serde_json::from_str::<RawControlRequestLine>(line) else {
        return vec![];
    };
    let Some(req) = v
        .request
        .filter(|r| r.subtype.as_deref() == Some("can_use_tool"))
    else {
        return vec![];
    };
    let Some(request_id) = v.request_id else {
        return vec![];
    };
    let input = req.input.and_then(to_value).unwrap_or_else(|| json!({}));
    vec![StreamEvent::PermissionRequest(PermissionRequest {
        request_id: request_id.into_owned(),
        tool_name: req.tool_name.map(Cow::into_owned).unwrap_or_default(),
        tool_use_id: req.tool_use_id.map(Cow::into_owned),
        description: req.description.map(Cow::into_owned),
        summary: tool_input_summary(&input),
        input,
    })]
}

fn parse_result(line: &str) -> Vec<StreamEvent> {
    let Ok(v) = serde_json::from_str::<RawResult>(line) else {
        return vec![];
    };
    let context_window = v
        .model_usage
        .as_ref()
        .and_then(|m| m.values().filter_map(|u| u.context_window).max());
    vec![StreamEvent::TurnEnd {
        ok: !v.is_error,
        subtype: v.subtype.into_owned(),
        session_id: v.session_id.map(Cow::into_owned),
        result: v.result.map(Cow::into_owned),
        cost_usd: v.total_cost_usd,
        context_window,
    }]
}

/// Parses one stdout line. Most lines give zero or one event; an assistant or user message
/// gives one per content block.
pub fn parse_line(line: &str) -> Vec<StreamEvent> {
    let line = line.trim();
    let Ok(tagged) = serde_json::from_str::<Tagged>(line) else {
        return vec![];
    };
    match tagged.kind.as_ref() {
        "system" => parse_system(line),
        "stream_event" => parse_stream_event(line),
        "assistant" => parse_assistant(line),
        "user" => parse_user(line),
        "control_request" => parse_control_request(line),
        "result" => parse_result(line),
        // `control_response`, `rate_limit_event` and anything else observed or future.
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
