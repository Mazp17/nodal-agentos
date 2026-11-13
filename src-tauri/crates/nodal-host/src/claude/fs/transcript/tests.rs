use super::*;
use crate::testutil::TempDir;

#[test]
fn session_title_prefers_the_last_rename_then_the_last_ai_title() {
    let t = TempDir::new("session-title");
    let f = t.0.join("s.jsonl");
    let write = |lines: &[&str]| fs::write(&f, lines.join("\n")).unwrap();
    write(&[r#"{"type":"user","message":{"content":"hi"}}"#]);
    assert_eq!(session_title(&f), None);
    write(&[
        r#"{"type":"ai-title","aiTitle":"Casual chat","sessionId":"s"}"#,
        r#"{"type":"user","message":{"content":"say \"ai-title\""}}"#,
        r#"{"type":"ai-title","aiTitle":"  Project   status\n"}"#,
        r#"{"type":"ai-title","aiTitle":""}"#,
    ]);
    assert_eq!(session_title(&f).as_deref(), Some("Project status"));
    write(&[
        r#"{"type":"custom-title","customTitle":"okapi spike"}"#,
        r#"{"type":"ai-title","aiTitle":"Later AI title"}"#,
    ]);
    assert_eq!(session_title(&f).as_deref(), Some("okapi spike"));
    assert_eq!(session_title(&t.0.join("missing.jsonl")), None);
}

#[test]
fn session_file_edit_results_carry_their_diff() {
    // Session files record the structured result as `toolUseResult` on the line.
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"edit it"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/r/README.md"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"The file was updated."}]},"toolUseResult":{"filePath":"/r/README.md","structuredPatch":[{"oldStart":17,"oldLines":1,"newStart":17,"newLines":3,"lines":[" last line","+","+Modificado desde Nodal"]}]}}"#,
    ];
    let p = parse_transcript(&lines.join("\n"), 100);
    let TranscriptItem::ToolUse {
        result: Some(r), ..
    } = &p.items[0]
    else {
        panic!("{:?}", p.items)
    };
    let patch = r.patch.as_ref().expect("patch");
    assert_eq!(
        (
            patch.file.path.as_str(),
            patch.file.additions,
            patch.file.deletions
        ),
        ("/r/README.md", 2, 0)
    );
    let last = patch.file.hunks[0].lines.last().unwrap();
    assert_eq!(
        (last.text.as_str(), last.new_no),
        ("Modificado desde Nodal", Some(19))
    );
}

#[test]
fn transcript_tolerates_errors_arrays_and_long_text() {
    let long = "x".repeat(TEXT_MAX + 50);
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"Task without wrapper"}}"#.to_string(),
        r#"{"type":"attachment","attachment":{"type":"skill_listing","content":"user assistant"}}"#.to_string(),
        format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"thinking","thinking":"hmm"}},{{"type":"text","text":"{long}"}}]}}}}"#),
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a/b.rs"}}]}}"#.to_string(),
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"File not found"},{"type":"image"}]}]}}"#.to_string(),
        r#"{"type":"user","message":{"content":"still there?"}}"#.to_string(),
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}]}}"#.to_string(),
        r#"{"type":"assistant","message":{"content":[{"type":"text","te"#.to_string(),
    ];
    let p = parse_transcript(&lines.join("\n"), 100);
    assert_eq!(p.prompt.as_deref(), Some("Task without wrapper"));
    assert_eq!(p.total, 5);
    assert_eq!(p.tool_calls, 2);
    assert!(matches!(&p.items[0], TranscriptItem::Thinking { text, .. } if text == "hmm"));
    assert!(
        matches!(&p.items[1], TranscriptItem::Text { truncated: true, text } if text.chars().count() == TEXT_MAX + 1)
    );
    match &p.items[2] {
        TranscriptItem::ToolUse {
            summary,
            result: Some(r),
            ..
        } => {
            assert_eq!(summary.as_deref(), Some("/a/b.rs"));
            assert!(r.is_error);
            assert_eq!(r.text, "File not found\n[image]");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&p.items[3], TranscriptItem::User { text, .. } if text == "still there?"));
    // Tool with no result yet (agent running) and empty input.
    assert!(matches!(
        &p.items[4],
        TranscriptItem::ToolUse {
            result: None,
            input: None,
            summary: None,
            ..
        }
    ));
    // No StructuredOutput: the final output is the last text.
    assert!(p.final_output.unwrap().starts_with("xxx"));
}

#[test]
fn session_transcript_skips_sidechains() {
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"Fix the login bug"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Looking at the code."}]}}"#,
        r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"subagent"}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"a.rs"}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}"#,
    ];
    let dir = std::env::temp_dir().join(format!("nodal-session-tx-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.jsonl");
    fs::write(&path, lines.join("\n")).unwrap();
    let t = read_session_transcript(&path, "run-1", Some("Claude".into()), None, 2)
        .unwrap()
        .unwrap();
    assert_eq!(t.agent_id, "run-1");
    assert_eq!(t.prompt.as_deref(), Some("Fix the login bug"));
    assert_eq!((t.total_items, t.omitted, t.tool_calls), (3, 1, 1));
    assert_eq!(t.final_output.as_deref(), Some("Done."));
    assert!(!t
        .items
        .iter()
        .any(|i| matches!(i, TranscriptItem::Text { text, .. } if text == "subagent")));
    assert!(
        read_session_transcript(&dir.join("nope.jsonl"), "x", None, None, 10)
            .unwrap()
            .is_none()
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn agent_results_carry_their_stats() {
    let lines = [
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"prompt":"go"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"done"}]},"toolUseResult":{"status":"completed","agentId":"a73d70ac1e77c19f3","agentType":"web-fetch","totalDurationMs":11270,"totalTokens":16367,"totalToolUseCount":2}}"#,
    ];
    let p = parse_transcript(&lines.join("\n"), 100);
    let TranscriptItem::ToolUse { result: Some(r), .. } = &p.items[0] else { panic!("{:?}", p.items) };
    let a = r.agent.as_ref().expect("agent");
    assert_eq!((a.agent_id.as_str(), a.agent_type.as_deref(), a.status.as_deref()), ("a73d70ac1e77c19f3", Some("web-fetch"), Some("completed")));
    assert_eq!((a.duration_ms, a.total_tokens, a.tool_uses), (Some(11270), Some(16367), Some(2)));
}

#[test]
fn subagents_are_listed_and_read_from_the_session_folder() {
    let t = TempDir::new("subagents");
    let session = t.0.join("sid.jsonl");
    let dir = t.0.join("sid").join("subagents");
    fs::create_dir_all(dir.join("workflows")).unwrap();
    fs::write(dir.join("agent-a1.meta.json"), r#"{"agentType":"web-fetch","description":"Read orca.dev","toolUseId":"toolu_1","model":"sonnet","stoppedByUser":true}"#).unwrap();
    fs::write(dir.join("agent-a1.jsonl"), [
        r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"Fetch it"}}"#,
        r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"Fetched."}]}}"#,
    ].join("\n")).unwrap();
    fs::write(dir.join("agent-b2.jsonl"), "").unwrap();
    let subs = list_subagents(&session);
    assert_eq!(subs.len(), 2, "{subs:?}");
    let a1 = subs.iter().find(|s| s.agent_id == "a1").unwrap();
    assert_eq!((a1.tool_use_id.as_deref(), a1.agent_type.as_deref(), a1.stopped_by_user), (Some("toolu_1"), Some("web-fetch"), true));
    assert!(a1.started_at.is_some());
    let tr = read_subagent_transcript(&session, "a1", 10).unwrap().unwrap();
    assert_eq!((tr.label.as_deref(), tr.model.as_deref(), tr.prompt.as_deref()), (Some("Read orca.dev"), Some("sonnet"), Some("Fetch it")));
    assert_eq!(tr.final_output.as_deref(), Some("Fetched."));
    for bad in ["../x", "a/b", "", "a.b"] {
        assert!(read_subagent_transcript(&session, bad, 10).unwrap().is_none(), "{bad}");
    }
    assert!(read_subagent_transcript(&session, "zz", 10).unwrap().is_none());
    assert!(list_subagents(&t.0.join("none.jsonl")).is_empty());
}
