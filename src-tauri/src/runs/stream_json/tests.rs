use std::path::Path;

use super::*;
use crate::runs::types::ToolPatch;
use crate::work::diff::{DiffFileStatus, DiffLineKind};

fn fixture(name: &str) -> Vec<StreamEvent> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/runs/fixtures/stream_json")
        .join(name);
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(parse_line)
        .collect()
}

#[test]
fn permission_turn() {
    let ev = fixture("permission_turn.jsonl");
    assert!(
        matches!(&ev[0], StreamEvent::Init { session_id, permission_mode: Some(m), .. }
        if session_id == "00000000-0000-4000-8000-000000000001" && m == "default")
    );
    assert_eq!(
        ev[1],
        StreamEvent::TextDelta {
            text: "Creating it.".into()
        }
    );
    assert!(
        matches!(&ev[2], StreamEvent::Item { item: TranscriptItem::ToolUse { id: Some(id), name, summary: Some(s), .. } }
        if id == "toolu_01" && name == "Write" && s == "/Users/me/Code/acme/hello.txt")
    );
    let StreamEvent::PermissionRequest(req) = &ev[3] else {
        panic!("{:?}", ev[3])
    };
    assert_eq!(req.request_id, "9ef8de1d-0000-4000-8000-000000000002");
    assert_eq!(
        (req.tool_name.as_str(), req.tool_use_id.as_deref()),
        ("Write", Some("toolu_01"))
    );
    assert_eq!(req.description.as_deref(), Some("hello.txt"));
    assert_eq!(req.input["content"], "hi");
    assert!(
        matches!(&ev[4], StreamEvent::ToolResult { tool_use_id, result } if tool_use_id == "toolu_01" && !result.is_error)
    );
    assert!(
        matches!(&ev[5], StreamEvent::Item { item: TranscriptItem::Text { text, .. } } if text == "Created hello.txt.")
    );
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
    assert!(
        matches!(&denied[2], StreamEvent::ToolResult { result, .. } if result.is_error && result.text == "The user denied this in Nodal.")
    );

    let interrupted = fixture("interrupted_turn.jsonl");
    assert!(
        matches!(interrupted.last(), Some(StreamEvent::TurnEnd { ok: false, subtype, .. }) if subtype == "error_during_execution")
    );
    assert_eq!(interrupted.len(), 3, "the interrupt ack is not a UI event");

    let auto = fixture("no_permission_tool.jsonl");
    assert!(
        matches!(&auto[1], StreamEvent::PermissionDenied { tool_name, tool_use_id: Some(id), .. } if tool_name == "Write" && id == "toolu_03")
    );
}

#[test]
fn usage_context_window_and_replayed_user() {
    let assistant = r#"{"type":"assistant","message":{"id":"m","content":[{"type":"thinking","thinking":""},{"type":"text","text":"PONG"}],"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"output_tokens":5}},"parent_tool_use_id":null}"#;
    let ev = parse_line(assistant);
    assert!(
        matches!(&ev[0], StreamEvent::Item { item: TranscriptItem::Text { text, .. } } if text == "PONG")
    );
    assert_eq!(
        ev[1],
        StreamEvent::Usage {
            context_tokens: 1110
        }
    );
    assert_eq!(ev.len(), 2, "empty thinking is skipped");

    let result = r#"{"type":"result","subtype":"success","is_error":false,"modelUsage":{"claude-haiku-4-5":{"contextWindow":200000},"claude-x":{"contextWindow":1000000}}}"#;
    assert!(matches!(
        &parse_line(result)[0],
        StreamEvent::TurnEnd {
            context_window: Some(1_000_000),
            ..
        }
    ));

    let replay =
        r#"{"type":"user","message":{"role":"user","content":"Hello"},"parent_tool_use_id":null}"#;
    assert!(
        matches!(&parse_line(replay)[0], StreamEvent::Item { item: TranscriptItem::User { text, .. } } if text == "Hello")
    );
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
    assert_eq!(
        (
            p.file.path.as_str(),
            p.file.status,
            p.file.additions,
            p.file.deletions,
            p.truncated
        ),
        ("/r/a.txt", DiffFileStatus::Modified, 1, 1, false)
    );
    let h = &p.file.hunks[0];
    assert_eq!(h.header, "@@ -1,3 +1,3 @@");
    let rows: Vec<(DiffLineKind, &str, Option<u32>, Option<u32>)> = h
        .lines
        .iter()
        .map(|l| (l.kind, l.text.as_str(), l.old_no, l.new_no))
        .collect();
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
    assert_eq!(
        (p.file.status, p.file.additions, p.file.hunks[0].lines.len()),
        (DiffFileStatus::Added, 2, 2)
    );

    let overwrite = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t3","content":"ok"}]},"tool_use_result":{"type":"update","filePath":"/r/a.txt","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":1,"lines":["-a","+X","\\ No newline at end of file"]}]}}"#;
    let p = patch_of(overwrite).unwrap();
    assert_eq!(
        (
            p.file.additions,
            p.file.deletions,
            p.file.hunks[0].lines.len()
        ),
        (1, 1, 2)
    );
}

#[test]
fn no_diff_for_errors_other_tools_or_several_results() {
    let tur = r#""tool_use_result":{"filePath":"/r/a.txt","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":1,"lines":["-a","+b"]}]}"#;
    let error = format!(
        r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"boom","is_error":true}}]}},{tur}}}"#
    );
    assert_eq!(patch_of(&error), None);
    let read = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"x"}]},"tool_use_result":{"type":"text","file":{"filePath":"/r/a.txt"}}}"#;
    assert_eq!(patch_of(read), None);
    let two = format!(
        r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"a"}},{{"type":"tool_result","tool_use_id":"t2","content":"b"}}]}},{tur}}}"#
    );
    assert!(parse_line(&two)
        .iter()
        .all(|e| matches!(e, StreamEvent::ToolResult { result, .. } if result.patch.is_none())));
}

#[test]
fn long_diffs_are_capped_but_counted() {
    let lines: Vec<String> = (0..500).map(|i| format!("\"+l{i}\"")).collect();
    let line = format!(
        r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"ok"}}]}},"tool_use_result":{{"filePath":"/r/a.txt","structuredPatch":[{{"oldStart":0,"oldLines":0,"newStart":1,"newLines":500,"lines":[{}]}}]}}}}"#,
        lines.join(",")
    );
    let p = patch_of(&line).unwrap();
    assert_eq!(
        (p.file.additions, p.file.hunks[0].lines.len(), p.truncated),
        (500, 400, true)
    );
}

#[test]
fn multi_hunk_numbers_crlf_and_big_creates() {
    let two = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]},"tool_use_result":{"filePath":"/r/a.txt","structuredPatch":[{"oldStart":2,"oldLines":1,"newStart":2,"newLines":2,"lines":[" a\r","+b\r"]},{"oldStart":40,"oldLines":2,"newStart":41,"newLines":1,"lines":["-x"," y"]}]}}"#;
    let p = patch_of(two).unwrap();
    let rows: Vec<(&str, Option<u32>, Option<u32>)> = p
        .file
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| (l.text.as_str(), l.old_no, l.new_no))
        .collect();
    assert_eq!(
        rows,
        [
            ("a", Some(2), Some(2)),
            ("b", None, Some(3)),
            ("x", Some(40), None),
            ("y", Some(41), Some(41))
        ]
    );

    let content: String = (0..450).map(|i| format!("l{i}\\n")).collect();
    let create = format!(
        r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t2","content":"ok"}}]}},"tool_use_result":{{"type":"create","filePath":"/r/b.txt","content":"{content}","structuredPatch":[]}}}}"#
    );
    let p = patch_of(&create).unwrap();
    assert_eq!(
        (p.file.additions, p.file.hunks[0].lines.len(), p.truncated),
        (450, 400, true)
    );
}

#[test]
fn stdin_messages() {
    let v: Value = serde_json::from_str(&user_message("-rf \"quoted\"\nline")).unwrap();
    assert_eq!(
        v,
        json!({"type": "user", "message": {"role": "user", "content": "-rf \"quoted\"\nline"}})
    );
    let input = json!({"file_path": "/Users/me/a.txt"});
    let v: Value = serde_json::from_str(&permission_response("r1", Some(&input), "")).unwrap();
    assert_eq!(v["response"]["request_id"], "r1");
    assert_eq!(
        v["response"]["response"],
        json!({"behavior": "allow", "updatedInput": input})
    );
    let v: Value = serde_json::from_str(&permission_response("r2", None, "No.")).unwrap();
    assert_eq!(
        v["response"]["response"],
        json!({"behavior": "deny", "message": "No."})
    );
    let v: Value = serde_json::from_str(&interrupt_request("i1")).unwrap();
    assert_eq!(
        v,
        json!({"type": "control_request", "request_id": "i1", "request": {"subtype": "interrupt"}})
    );
    assert_eq!(resume_args("abc"), ["--resume", "abc"]);
}
