use std::path::PathBuf;

use super::*;
use crate::claude::fs::paths::{claude_config_dir, project_slug};
use crate::testutil::TempDir;

const SANDBOX: &str = "/Users/me/Code/nodal-sandbox";
const DONE_SESSION: &str = "ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8";

fn fixtures_projects() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/claude/fixtures/projects")
}

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
fn transcript_from_real_fixture() {
    let dir = fixtures_projects()
        .join(project_slug(SANDBOX))
        .join(DONE_SESSION);
    let t = read_agent_transcript(&dir, "wf_2450b7a8-254", "a1007a0f03db17270", 200)
        .unwrap()
        .unwrap();
    assert_eq!(t.label.as_deref(), Some("revisar:volcanes-famosos"));
    assert_eq!(t.model.as_deref(), Some("claude-sonnet-5"));
    assert_eq!(t.phase.as_deref(), Some("Revisar"));
    // Without the harness wrapper and without the indentation.
    let prompt = t.prompt.unwrap();
    assert!(
        prompt.starts_with("Leé /Users/me/Code/nodal-sandbox/demo-out/04-volcanes-famosos.md"),
        "{prompt}"
    );
    assert!(!prompt.contains("Workflow harness"));
    // Empty (signed) thinking is skipped: Bash, text, StructuredOutput.
    assert_eq!(t.total_items, 3);
    assert_eq!(t.tool_calls, 2);
    assert_eq!(t.omitted, 0);
    match &t.items[0] {
        TranscriptItem::ToolUse {
            name,
            summary,
            result,
            ..
        } => {
            assert_eq!(name, "Bash");
            assert!(summary
                .as_deref()
                .unwrap()
                .starts_with("cat /Users/me/Code/nodal-sandbox/demo-out/04"));
            let r = result.as_ref().unwrap();
            assert!(r.text.starts_with("# Volcanes famosos del mundo"));
            assert!(!r.is_error);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(&t.items[1], TranscriptItem::Text { text, .. } if text.starts_with("160 palabras"))
    );
    assert!(t.final_output.unwrap().contains("\"ok\": true"));
    assert!(!t.partial);

    // Limit: only the last N.
    let t = read_agent_transcript(&dir, "wf_2450b7a8-254", "a1007a0f03db17270", 1)
        .unwrap()
        .unwrap();
    assert_eq!((t.items.len(), t.omitted, t.total_items), (1, 2, 3));
    assert!(
        matches!(&t.items[0], TranscriptItem::ToolUse { name, .. } if name == "StructuredOutput")
    );

    // No file → None; ids with paths → error.
    assert_eq!(
        read_agent_transcript(&dir, "wf_2450b7a8-254", "nope", 10).unwrap(),
        None
    );
    assert!(read_agent_transcript(&dir, "wf_2450b7a8-254", "../x", 10).is_err());
    assert!(read_agent_transcript(&dir, "..", "a1", 10).is_err());
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

/// Every workflow transcript on this machine: none fails, and timings are measured.
#[test]
#[ignore]
fn real_transcripts_on_disk() {
    let projects = claude_config_dir().unwrap().join("projects");
    let (mut n, mut slowest) = (0, (std::time::Duration::ZERO, PathBuf::new()));
    for proj in fs::read_dir(&projects).unwrap().flatten() {
        for sess in fs::read_dir(proj.path()).into_iter().flatten().flatten() {
            let wfs = sess.path().join("subagents").join("workflows");
            for wf in fs::read_dir(&wfs).into_iter().flatten().flatten() {
                let wf_id = wf.file_name().to_string_lossy().into_owned();
                for f in fs::read_dir(wf.path()).into_iter().flatten().flatten() {
                    let name = f.file_name().to_string_lossy().into_owned();
                    let Some(agent) = name
                        .strip_prefix("agent-")
                        .and_then(|s| s.strip_suffix(".jsonl"))
                    else {
                        continue;
                    };
                    let t0 = std::time::Instant::now();
                    let t = read_agent_transcript(&sess.path(), &wf_id, agent, 200)
                        .unwrap()
                        .unwrap();
                    let dt = t0.elapsed();
                    assert!(t.items.len() <= 200);
                    if dt > slowest.0 {
                        slowest = (dt, f.path());
                    }
                    n += 1;
                }
            }
        }
    }
    eprintln!(
        "{n} transcripts; slowest {:?} {}",
        slowest.0,
        slowest.1.display()
    );
}
