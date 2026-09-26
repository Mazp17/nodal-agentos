//! WARNING: Claude Code's `-p` stream-json protocol, partly UNDOCUMENTED (observed in
//! v2.1.283; see `stream_json_live` for the spike and `fixtures/stream_json/`).
//!
//! The only place that knows the shape of what a chat's `claude` process prints and of what
//! Nodal writes to its stdin: launch flags, event parsing and the stdin messages.
//! Content blocks share their shape with session files, so they go through `claude_fs`.
//!
//! Policy: tolerate anything unknown and skip it instead of failing.

use serde::Serialize;
use serde_json::{json, Value};

use super::claude_fs;
use super::types::{ToolResultInfo, TranscriptItem};

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
/// with 2.1.283). The id must already be checked with `claude_fs::is_valid_session_id`.
pub fn resume_args(session_id: &str) -> [String; 2] {
    ["--resume".into(), session_id.into()]
}

/// A permission prompt the host must answer (`control_request` / `can_use_tool`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool_name: String,
    /// The `tool_use` block it belongs to (`ToolUse.id` in the transcript).
    pub tool_use_id: Option<String>,
    /// What the tool is about to do, as Claude Code words it (a path, a command…).
    pub description: Option<String>,
    /// One line: the main argument, like transcript tool calls.
    pub summary: Option<String>,
    /// The tool's input as sent by Claude Code; allowing sends it back unchanged.
    pub input: Value,
}

/// What a chat process reports, ready for the UI. `TextDelta`/`ThinkingDelta` preview text
/// that a later `Item` carries in full.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StreamEvent {
    #[serde(rename_all = "camelCase")]
    Init { session_id: String, model: Option<String>, cwd: Option<String>, permission_mode: Option<String> },
    #[serde(rename_all = "camelCase")]
    TextDelta { text: String },
    #[serde(rename_all = "camelCase")]
    ThinkingDelta { text: String },
    /// A complete text, thinking, tool call or (replayed) user message.
    #[serde(rename_all = "camelCase")]
    Item { item: TranscriptItem },
    #[serde(rename_all = "camelCase")]
    ToolResult { tool_use_id: String, result: ToolResultInfo },
    PermissionRequest(PermissionRequest),
    /// A tool was denied without asking (the permission mode or a rule decided).
    #[serde(rename_all = "camelCase")]
    PermissionDenied { tool_name: String, tool_use_id: Option<String>, message: Option<String> },
    /// Tokens in the context after the last model response: input + cache reads + cache writes.
    #[serde(rename_all = "camelCase")]
    Usage { context_tokens: i64 },
    /// The turn is over: `ok` is false for errors and interrupts (`subtype`
    /// `error_during_execution`, `error_max_turns`…).
    #[serde(rename_all = "camelCase")]
    TurnEnd {
        ok: bool,
        subtype: String,
        session_id: Option<String>,
        result: Option<String>,
        cost_usd: Option<f64>,
        context_window: Option<i64>,
    },
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
                .filter_map(claude_fs::assistant_item)
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
                    vec![StreamEvent::Item { item: claude_fs::user_item(s) }]
                }
                Some(Value::Array(blocks)) => {
                    let structured = claude_fs::single_result(blocks, v.get("tool_use_result"));
                    blocks
                        .iter()
                        .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                            Some("tool_result") => {
                                let (id, result) = claude_fs::tool_result_info(b, structured);
                                Some(StreamEvent::ToolResult { tool_use_id: id?.to_string(), result })
                            }
                            Some("text") => {
                                let s = b.get("text").and_then(Value::as_str).filter(|s| !s.trim().is_empty())?;
                                Some(StreamEvent::Item { item: claude_fs::user_item(s) })
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
                summary: claude_fs::tool_input_summary(&input),
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
mod tests {
    use std::path::Path;

    use super::*;
    use crate::runs::types::ToolPatch;
    use crate::work::diff::{DiffFileStatus, DiffLineKind};

    fn fixture(name: &str) -> Vec<StreamEvent> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runs/fixtures/stream_json").join(name);
        std::fs::read_to_string(path).unwrap().lines().flat_map(parse_line).collect()
    }

    #[test]
    fn permission_turn() {
        let ev = fixture("permission_turn.jsonl");
        assert!(matches!(&ev[0], StreamEvent::Init { session_id, permission_mode: Some(m), .. }
            if session_id == "00000000-0000-4000-8000-000000000001" && m == "default"));
        assert_eq!(ev[1], StreamEvent::TextDelta { text: "Creating it.".into() });
        assert!(matches!(&ev[2], StreamEvent::Item { item: TranscriptItem::ToolUse { id: Some(id), name, summary: Some(s), .. } }
            if id == "toolu_01" && name == "Write" && s == "/Users/me/Code/acme/hello.txt"));
        let StreamEvent::PermissionRequest(req) = &ev[3] else { panic!("{:?}", ev[3]) };
        assert_eq!(req.request_id, "9ef8de1d-0000-4000-8000-000000000002");
        assert_eq!((req.tool_name.as_str(), req.tool_use_id.as_deref()), ("Write", Some("toolu_01")));
        assert_eq!(req.description.as_deref(), Some("hello.txt"));
        assert_eq!(req.input["content"], "hi");
        assert!(matches!(&ev[4], StreamEvent::ToolResult { tool_use_id, result } if tool_use_id == "toolu_01" && !result.is_error));
        assert!(matches!(&ev[5], StreamEvent::Item { item: TranscriptItem::Text { text, .. } } if text == "Created hello.txt."));
        assert_eq!(
            ev[6],
            StreamEvent::TurnEnd {
                ok: true,
                subtype: "success".into(),
                session_id: Some("00000000-0000-4000-8000-000000000001".into()),
                result: Some("Created hello.txt.".into()),
                cost_usd: Some(0.03),
                context_window: None,
            }
        );
        assert_eq!(ev.len(), 7);
    }

    #[test]
    fn denial_interrupt_and_auto_deny() {
        let denied = fixture("denied_turn.jsonl");
        assert!(matches!(&denied[2], StreamEvent::ToolResult { result, .. } if result.is_error && result.text == "The user denied this in Nodal."));

        let interrupted = fixture("interrupted_turn.jsonl");
        assert!(matches!(interrupted.last(), Some(StreamEvent::TurnEnd { ok: false, subtype, .. }) if subtype == "error_during_execution"));
        assert_eq!(interrupted.len(), 3, "the interrupt ack is not a UI event");

        let auto = fixture("no_permission_tool.jsonl");
        assert!(matches!(&auto[1], StreamEvent::PermissionDenied { tool_name, tool_use_id: Some(id), .. } if tool_name == "Write" && id == "toolu_03"));
    }

    #[test]
    fn usage_context_window_and_replayed_user() {
        let assistant = r#"{"type":"assistant","message":{"id":"m","content":[{"type":"thinking","thinking":""},{"type":"text","text":"PONG"}],"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"output_tokens":5}},"parent_tool_use_id":null}"#;
        let ev = parse_line(assistant);
        assert!(matches!(&ev[0], StreamEvent::Item { item: TranscriptItem::Text { text, .. } } if text == "PONG"));
        assert_eq!(ev[1], StreamEvent::Usage { context_tokens: 1110 });
        assert_eq!(ev.len(), 2, "empty thinking is skipped");

        let result = r#"{"type":"result","subtype":"success","is_error":false,"modelUsage":{"claude-haiku-4-5":{"contextWindow":200000},"claude-x":{"contextWindow":1000000}}}"#;
        assert!(matches!(&parse_line(result)[0], StreamEvent::TurnEnd { context_window: Some(1_000_000), .. }));

        let replay = r#"{"type":"user","message":{"role":"user","content":"Hello"},"parent_tool_use_id":null}"#;
        assert!(matches!(&parse_line(replay)[0], StreamEvent::Item { item: TranscriptItem::User { text, .. } } if text == "Hello"));
    }

    #[test]
    fn skips_subagents_and_junk() {
        for line in [
            "",
            "not json",
            r#"{"type":"rate_limit_event"}"#,
            r#"{"type":"system","subtype":"hook_started"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"inner"}]},"parent_tool_use_id":"toolu_9"}"#,
            r#"{"type":"stream_event","event":{"delta":{"type":"text_delta","text":"x"}},"parent_tool_use_id":"toolu_9"}"#,
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"i"}}"#,
            r#"{"type":"control_request","request":{"subtype":"can_use_tool"}}"#,
        ] {
            assert!(parse_line(line).is_empty(), "{line}");
        }
    }

    fn patch_of(line: &str) -> Option<ToolPatch> {
        match parse_line(line).into_iter().next() {
            Some(StreamEvent::ToolResult { result, .. }) => result.patch,
            other => panic!("expected a tool result, got {other:?}"),
        }
    }

    #[test]
    fn edit_and_write_results_carry_their_diff() {
        // Shapes observed with 2.1.283 (`originalFile` and friends trimmed).
        let edit = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]},"parent_tool_use_id":null,
            "tool_use_result":{"filePath":"/r/a.txt","oldString":"line two","newString":"line 2","structuredPatch":[{"oldStart":1,"oldLines":3,"newStart":1,"newLines":3,"lines":[" line one","-line two","+line 2"," line three"]}]}}"#;
        let p = patch_of(&edit.replace('\n', "")).unwrap();
        assert_eq!((p.file.path.as_str(), p.file.status, p.file.additions, p.file.deletions, p.truncated), ("/r/a.txt", DiffFileStatus::Modified, 1, 1, false));
        let h = &p.file.hunks[0];
        assert_eq!(h.header, "@@ -1,3 +1,3 @@");
        let rows: Vec<(DiffLineKind, &str, Option<u32>, Option<u32>)> = h.lines.iter().map(|l| (l.kind, l.text.as_str(), l.old_no, l.new_no)).collect();
        assert_eq!(
            rows,
            [
                (DiffLineKind::Context, "line one", Some(1), Some(1)),
                (DiffLineKind::Del, "line two", Some(2), None),
                (DiffLineKind::Add, "line 2", None, Some(2)),
                (DiffLineKind::Context, "line three", Some(3), Some(3)),
            ]
        );

        let create = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"ok"}]},"tool_use_result":{"type":"create","filePath":"/r/b.txt","content":"hi\nthere\n","structuredPatch":[]}}"#;
        let p = patch_of(create).unwrap();
        assert_eq!((p.file.status, p.file.additions, p.file.hunks[0].lines.len()), (DiffFileStatus::Added, 2, 2));

        let overwrite = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","content":"ok"}]},"tool_use_result":{"type":"update","filePath":"/r/a.txt","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":1,"lines":["-a","+X","\\ No newline at end of file"]}]}}"#;
        let p = patch_of(overwrite).unwrap();
        assert_eq!((p.file.additions, p.file.deletions, p.file.hunks[0].lines.len()), (1, 1, 2));
    }

    #[test]
    fn no_diff_for_errors_other_tools_or_several_results() {
        let tur = r#""tool_use_result":{"filePath":"/r/a.txt","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":1,"lines":["-a","+b"]}]}"#;
        let error = format!(r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"boom","is_error":true}}]}},{tur}}}"#);
        assert_eq!(patch_of(&error), None);
        let read = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"x"}]},"tool_use_result":{"type":"text","file":{"filePath":"/r/a.txt"}}}"#;
        assert_eq!(patch_of(read), None);
        let two = format!(r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"a"}},{{"type":"tool_result","tool_use_id":"t2","content":"b"}}]}},{tur}}}"#);
        assert!(parse_line(&two).iter().all(|e| matches!(e, StreamEvent::ToolResult { result, .. } if result.patch.is_none())));
    }

    #[test]
    fn long_diffs_are_capped_but_counted() {
        let lines: Vec<String> = (0..500).map(|i| format!("\"+l{i}\"")).collect();
        let line = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"ok"}}]}},"tool_use_result":{{"filePath":"/r/a.txt","structuredPatch":[{{"oldStart":0,"oldLines":0,"newStart":1,"newLines":500,"lines":[{}]}}]}}}}"#,
            lines.join(",")
        );
        let p = patch_of(&line).unwrap();
        assert_eq!((p.file.additions, p.file.hunks[0].lines.len(), p.truncated), (500, 400, true));
    }

    #[test]
    fn stdin_messages() {
        let v: Value = serde_json::from_str(&user_message("-rf \"quoted\"\nline")).unwrap();
        assert_eq!(v, json!({"type": "user", "message": {"role": "user", "content": "-rf \"quoted\"\nline"}}));
        let input = json!({"file_path": "/Users/me/a.txt"});
        let v: Value = serde_json::from_str(&permission_response("r1", Some(&input), "")).unwrap();
        assert_eq!(v["response"]["request_id"], "r1");
        assert_eq!(v["response"]["response"], json!({"behavior": "allow", "updatedInput": input}));
        let v: Value = serde_json::from_str(&permission_response("r2", None, "No.")).unwrap();
        assert_eq!(v["response"]["response"], json!({"behavior": "deny", "message": "No."}));
        let v: Value = serde_json::from_str(&interrupt_request("i1")).unwrap();
        assert_eq!(v, json!({"type": "control_request", "request_id": "i1", "request": {"subtype": "interrupt"}}));
        assert_eq!(resume_args("abc"), ["--resume", "abc"]);
    }
}
