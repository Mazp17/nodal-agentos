use super::*;

const SANDBOX: &str = "/Users/me/Code/nodal-sandbox";
const DONE_SESSION: &str = "ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8";
const CUT_SESSION: &str = "91a57808-74f6-40fd-9fbf-184b7358bbe4";

fn fixtures_projects() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/runs/fixtures/projects")
}

#[test]
fn session_title_prefers_the_last_rename_then_the_last_ai_title() {
    let t = crate::util::paths::tests::TempDir::new("session-title");
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
    let TranscriptItem::ToolUse { result: Some(r), .. } = &p.items[0] else { panic!("{:?}", p.items) };
    let patch = r.patch.as_ref().expect("patch");
    assert_eq!((patch.file.path.as_str(), patch.file.additions, patch.file.deletions), ("/r/README.md", 2, 0));
    let last = patch.file.hunks[0].lines.last().unwrap();
    assert_eq!((last.text.as_str(), last.new_no), ("Modificado desde Nodal", Some(19)));
}

#[test]
fn workflow_review_denial_from_real_session() {
    // Real (trimmed) lines from a claude 2.1.281 --bg session with an unapproved plan-task.
    let text = concat!(
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_01D3","name":"Workflow","input":{"name":"plan-task","args":{"finish":"branch"}}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Review dynamic workflow before running","is_error":true,"tool_use_id":"toolu_01D3"}]},"toolUseResult":"Error: Review dynamic workflow before running"}"#,
        "\n",
    );
    assert_eq!(find_workflow_review_denial(text), Some(Some("plan-task".into())));
    // Without the call (or with another format): still detected, without a name.
    let only = text.lines().nth(1).unwrap();
    assert_eq!(find_workflow_review_denial(only), Some(None));
    // An assistant text quoting the message doesn't count.
    let quoted = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Review dynamic workflow before running"}]}}"#;
    assert_eq!(find_workflow_review_denial(quoted), None);
    assert_eq!(find_workflow_review_denial("{broken\n"), None);
}

#[test]
fn bg_line_real_format() {
    assert_eq!(parse_bg_line("backgrounded · ddb91222").as_deref(), Some("ddb91222"));
    assert_eq!(parse_bg_line("\u{1b}[2mbackgrounded\u{1b}[0m · \u{1b}[1mddb91222\u{1b}[0m").as_deref(), Some("ddb91222"));
    assert_eq!(parse_bg_line("  backgrounded · ab12cd34  \r").as_deref(), Some("ab12cd34"));
    assert_eq!(parse_bg_line("backgrounded · ab12cd34 · run `claude agents` to view").as_deref(), Some("ab12cd34"));
    assert_eq!(parse_bg_line("backgrounded"), None);
    assert_eq!(parse_bg_line("Error: not logged in"), None);
    assert_eq!(parse_bare_id("ddb91222\n").as_deref(), Some("ddb91222"));
    assert_eq!(parse_bare_id("hello world"), None);
}

#[test]
fn agents_json_filters_background_and_tolerates_missing_fields() {
    let text = r#"[
          {"id":"0eef7f11","cwd":"/a","kind":"background","startedAt":1787690543980,
           "sessionId":"0eef7f11-d932-4001-97f8-df01555801d7","name":"old","state":"done"},
          {"pid":59575,"cwd":"/b","kind":"interactive","startedAt":1790087661575,
           "sessionId":"a5ff54f0-6f8e-49b4-9213-0c859d3432de","name":"interactive","status":"idle"},
          {"pid":123,"id":"ddb91222","cwd":"/c","kind":"background","startedAt":1790192548000,
           "sessionId":"ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8","name":"new","status":"busy",
           "state":"working","newField":{"x":1}},
          {"id":"nosession","kind":"background"},
          {"id":"odd","sessionId":"s","kind":"background","pid":"not-a-number","startedAt":"yesterday"}
        ]"#;
    let runs = parse_agents_json(text).unwrap();
    let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["ddb91222", "0eef7f11", "odd"]);
    assert_eq!(runs[0].pid, Some(123));
    assert_eq!(runs[0].status.as_deref(), Some("busy"));
    assert_eq!(runs[0].state.as_deref(), Some("working"));
    assert_eq!(runs[1].pid, None);
    assert_eq!(runs[1].status, None);
    assert_eq!(runs[2].pid, None);
    assert_eq!(runs[2].started_at, None);
    assert!(parse_agents_json("no json").is_err());
}

#[test]
fn slug_matches_real_project_dirs() {
    assert_eq!(project_slug(SANDBOX), "-Users-me-Code-nodal-sandbox");
    assert_eq!(
        project_slug("/Users/x/Code/repo/.claude/worktrees/acme-38"),
        "-Users-x-Code-repo--claude-worktrees-acme-38"
    );
    assert!(is_valid_session_id(DONE_SESSION));
    assert!(!is_valid_session_id("../etc"));
    assert!(!is_valid_session_id(""));
}

#[test]
fn finds_session_dir_by_slug_and_by_scan() {
    let projects = fixtures_projects();
    let dir = find_session_dir(&projects, SANDBOX, DONE_SESSION).unwrap();
    assert!(dir.ends_with(DONE_SESSION));
    // cwd that doesn't match the slug: falls back to the scan.
    assert_eq!(find_session_dir(&projects, "/somewhere/else", DONE_SESSION), Some(dir));
    assert_eq!(find_session_dir(&projects, SANDBOX, "no-such-session"), None);
}

#[test]
fn completed_run_uses_final_summary() {
    let dir = fixtures_projects().join(project_slug(SANDBOX)).join(DONE_SESSION);
    let d = read_run_detail(&dir).unwrap();
    assert_eq!(d.source, DetailSource::Final);
    assert_eq!(d.workflow_id, "wf_2450b7a8-254");
    assert_eq!(d.workflow_name.as_deref(), Some("demo-board"));
    assert_eq!(d.status.as_deref(), Some("completed"));
    assert_eq!(d.result_status.as_deref(), Some("green"));
    let titles: Vec<&str> = d.phases.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, ["Idear", "Escribir", "Revisar", "Cerrar"]);
    assert_eq!(d.phases[0].detail.as_deref(), Some("un agente propone 4 subtemas"));
    assert_eq!(d.current_phase.as_deref(), Some("Cerrar"));
    assert_eq!(d.current_phase_index, Some(4));
    assert_eq!(d.agent_count, 10);
    assert_eq!(d.agents.len(), 10);
    assert!(d.agents.iter().all(|a| a.state == AgentState::Done));
    assert_eq!(d.total_tokens, Some(377324));
    assert_eq!(d.total_tool_calls, Some(45));
    assert_eq!(d.duration_ms, Some(117897));
    assert_eq!(d.workflow_count, 1);
    let first = &d.agents[0];
    assert_eq!(first.label, "idear:volcanes");
    assert_eq!(first.agent_id.as_deref(), Some("a6daf84e776bfaad5"));
    assert_eq!(first.model.as_deref(), Some("claude-sonnet-5"));
    assert_eq!(first.phase.as_deref(), Some("Idear"));
    assert_eq!(first.tokens, Some(34612));
    assert_eq!(first.tool_calls, Some(1));
    let last = d.agents.last().unwrap();
    assert_eq!(last.last_tool_name.as_deref(), Some("Bash"));
    assert_eq!(last.last_tool_summary.as_deref(), Some("ls -la ./demo-out && wc -w ./demo-out/*.md"));
}

#[test]
fn cut_run_is_rebuilt_from_journal() {
    let dir = fixtures_projects().join(project_slug(SANDBOX)).join(CUT_SESSION);
    let d = read_run_detail(&dir).unwrap();
    assert_eq!(d.source, DetailSource::Live);
    assert_eq!(d.workflow_id, "wf_4ec5fcf1-d6b");
    assert_eq!(d.workflow_name.as_deref(), Some("demo-board"));
    assert_eq!(d.status, None);
    assert_eq!(d.result_status, None);
    // Phases from the script's meta: the total is known even if they weren't reached.
    let titles: Vec<&str> = d.phases.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, ["Idear", "Escribir", "Revisar", "Cerrar"]);
    assert_eq!(d.current_phase.as_deref(), Some("Escribir"));
    assert_eq!(d.current_phase_index, Some(2));
    assert_eq!(d.agent_count, 5);
    let states: Vec<(&str, AgentState)> = d.agents.iter().map(|a| (a.label.as_str(), a.state)).collect();
    assert_eq!(
        states,
        [
            ("idear:volcanes", AgentState::Done),
            ("escribir:formacion-volcanica", AgentState::Running),
            ("escribir:tipos-de-volcanes", AgentState::Running),
            ("escribir:erupciones-volcanicas", AgentState::Running),
            ("escribir:volcanes-y-el-planeta", AgentState::Running),
        ]
    );
    assert!(d.agents.iter().all(|a| a.model.as_deref() == Some("claude-haiku-4-5-20251001")));
    // Last tool taken from the transcript's tail (only agents in progress).
    let planeta = d.agents.iter().find(|a| a.label == "escribir:volcanes-y-el-planeta").unwrap();
    assert_eq!(planeta.last_tool_name.as_deref(), Some("Bash"));
    assert_eq!(planeta.last_tool_summary.as_deref(), Some("sleep 34"));
    let idear = &d.agents[0];
    assert_eq!(idear.last_tool_name, None);
}

#[test]
fn journal_ignores_unknown_types_and_broken_lines() {
    let text = concat!(
        "{\"type\":\"launched\"}\n",
        "{\"type\":\"started\",\"key\":\"k1\",\"agentId\":\"a1\",\"label\":\"one\",\"phase\":\"A\"}\n",
        "{\"type\":\"something-new\",\"agentId\":\"a1\"}\n",
        "{\"type\":\"started\",\"key\":\"k2\",\"label\":\"two\",\"phase\":\"B\"}\n",
        "{\"type\":\"result\",\"key\":\"k2\",\"result\":null}\n",
        "{\"type\":\"started\",\"key\":\"k3\",\"agentId\":\"a3\",\"lab",
    );
    let agents = parse_journal(text);
    assert_eq!(agents.len(), 2);
    assert_eq!(agents[0].state, AgentState::Running);
    assert_eq!(agents[1].label, "two");
    assert_eq!(agents[1].state, AgentState::Done);
}

#[test]
fn final_summary_tolerates_garbage() {
    assert!(parse_final("{\"runId\": \"wf_x\", \"workflowProg", "wf_x").is_none());
    let d = parse_final(r#"{"workflowProgress":[{"type":"workflow_phase","index":1,"title":"Only"},
            {"type":"workflow_agent","label":"x","phaseTitle":"Only","state":"exploded","tokens":"lots"}],
            "result":{"status":"purple"}}"#, "wf_y").unwrap();
    assert_eq!(d.workflow_id, "wf_y");
    assert_eq!(d.phases[0].title, "Only");
    assert_eq!(d.agents[0].state, AgentState::Unknown);
    assert_eq!(d.agents[0].tokens, None);
    assert_eq!(d.result_status, None);
}

#[test]
fn script_phases_best_effort() {
    let src = "export const meta = { name: 'x', phases: [ { title: 'One', detail: \"with } brace\" },\n { title: `Two` } ], }";
    let phases = parse_script_phases(src);
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0].detail.as_deref(), Some("with } brace"));
    assert_eq!(phases[1].title, "Two");
    assert!(parse_script_phases("no phases").is_empty());
}

#[test]
fn agents_json_exposes_blocked_on_permission() {
    // Real entry from a --bg session waiting for approval of a Write (claude 2.1.281).
    let text = r#"[{"pid":21111,"id":"af5deb85","cwd":"/Users/me/Code/nodal-sandbox",
          "kind":"background","startedAt":1790198688952,"sessionId":"af5deb85-3fe1-4ea9-b172-00b68389e167",
          "name":"create perm-test.txt","status":"waiting","waitingFor":"permission prompt","state":"blocked"}]"#;
    let r = &parse_agents_json(text).unwrap()[0];
    assert_eq!(r.state.as_deref(), Some("blocked"));
    assert_eq!(r.status.as_deref(), Some("waiting"));
    assert_eq!(r.waiting_for.as_deref(), Some("permission prompt"));
    assert!(r.is_in_progress());
}

#[test]
fn linear_issue_result_is_typed() {
    let v: Value = serde_json::from_str(
        r#"{"issue":"ACME-4","status":"green","pr":"https://github.com/o/r/pull/3","branch":"feat/acme-4",
            "workdir":"/w/run-acme-4","states":{"inReview":"In Review"},"children":["ACME-35"],"levels":[["ACME-35"]],
            "plan":null,"unmetAcceptance":["Works offline", 3],"nits":["Extract hook"," "]}"#,
    )
    .unwrap();
    let r = parse_result(Some(&v)).unwrap();
    assert_eq!(r.issue.as_deref(), Some("ACME-4"));
    assert_eq!(r.pr.as_deref(), Some("https://github.com/o/r/pull/3"));
    assert_eq!(r.branch.as_deref(), Some("feat/acme-4"));
    assert_eq!(r.workdir.as_deref(), Some("/w/run-acme-4"));
    assert_eq!(r.unmet_acceptance, Some(vec!["Works offline".to_string()]));
    assert_eq!(r.nits, Some(vec!["Extract hook".to_string()]));
    assert!(r.raw.unwrap().contains("\"children\""));

    // Another workflow: no known fields, but with the raw JSON.
    let demo = read_run_detail(&fixtures_projects().join(project_slug(SANDBOX)).join(DONE_SESSION)).unwrap();
    let res = demo.result.unwrap();
    assert_eq!((res.pr, res.unmet_acceptance, res.nits), (None, None, None));
    assert!(res.raw.unwrap().contains("\"tema\": \"volcanes\""));

    // PR that isn't a web URL: dropped. `null` or missing: no result.
    let v: Value = serde_json::from_str(r#"{"pr":"javascript:alert(1)","nits":"not-a-list"}"#).unwrap();
    let r = parse_result(Some(&v)).unwrap();
    assert_eq!((r.pr, r.nits), (None, None));
    assert_eq!(parse_result(Some(&Value::Null)), None);
    assert_eq!(parse_result(None), None);
    assert_eq!(parse_result(Some(&Value::String("done".into()))).unwrap().raw.as_deref(), Some("done"));
}

#[test]
fn transcript_from_real_fixture() {
    let dir = fixtures_projects().join(project_slug(SANDBOX)).join(DONE_SESSION);
    let t = read_agent_transcript(&dir, "wf_2450b7a8-254", "a1007a0f03db17270", 200).unwrap().unwrap();
    assert_eq!(t.label.as_deref(), Some("revisar:volcanes-famosos"));
    assert_eq!(t.model.as_deref(), Some("claude-sonnet-5"));
    assert_eq!(t.phase.as_deref(), Some("Revisar"));
    // Without the harness wrapper and without the indentation.
    let prompt = t.prompt.unwrap();
    assert!(prompt.starts_with("Leé /Users/me/Code/nodal-sandbox/demo-out/04-volcanes-famosos.md"), "{prompt}");
    assert!(!prompt.contains("Workflow harness"));
    // Empty (signed) thinking is skipped: Bash, text, StructuredOutput.
    assert_eq!(t.total_items, 3);
    assert_eq!(t.tool_calls, 2);
    assert_eq!(t.omitted, 0);
    match &t.items[0] {
        TranscriptItem::ToolUse { name, summary, result, .. } => {
            assert_eq!(name, "Bash");
            assert!(summary.as_deref().unwrap().starts_with("cat /Users/me/Code/nodal-sandbox/demo-out/04"));
            let r = result.as_ref().unwrap();
            assert!(r.text.starts_with("# Volcanes famosos del mundo"));
            assert!(!r.is_error);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&t.items[1], TranscriptItem::Text { text, .. } if text.starts_with("160 palabras")));
    assert!(t.final_output.unwrap().contains("\"ok\": true"));
    assert!(!t.partial);

    // Limit: only the last N.
    let t = read_agent_transcript(&dir, "wf_2450b7a8-254", "a1007a0f03db17270", 1).unwrap().unwrap();
    assert_eq!((t.items.len(), t.omitted, t.total_items), (1, 2, 3));
    assert!(matches!(&t.items[0], TranscriptItem::ToolUse { name, .. } if name == "StructuredOutput"));

    // No file → None; ids with paths → error.
    assert_eq!(read_agent_transcript(&dir, "wf_2450b7a8-254", "nope", 10).unwrap(), None);
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
    assert!(matches!(&p.items[1], TranscriptItem::Text { truncated: true, text } if text.chars().count() == TEXT_MAX + 1));
    match &p.items[2] {
        TranscriptItem::ToolUse { summary, result: Some(r), .. } => {
            assert_eq!(summary.as_deref(), Some("/a/b.rs"));
            assert!(r.is_error);
            assert_eq!(r.text, "File not found\n[image]");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&p.items[3], TranscriptItem::User { text, .. } if text == "still there?"));
    // Tool with no result yet (agent running) and empty input.
    assert!(matches!(&p.items[4], TranscriptItem::ToolUse { result: None, input: None, summary: None, .. }));
    // No StructuredOutput: the final output is the last text.
    assert!(p.final_output.unwrap().starts_with("xxx"));
}

#[test]
fn path_ids() {
    assert!(is_valid_path_id("wf_2450b7a8-254"));
    assert!(is_valid_path_id("a1007a0f03db17270"));
    for bad in ["", "..", "a/b", "a\\b", "-rf", "wf_x.json", "a b"] {
        assert!(!is_valid_path_id(bad), "{bad}");
    }
}

#[test]
fn session_without_workflows_has_no_detail() {
    let tmp = std::env::temp_dir().join(format!("nodal-runs-test-{}", std::process::id()));
    fs::create_dir_all(&tmp).unwrap();
    assert_eq!(read_run_detail(&tmp), None);
    let _ = fs::remove_dir_all(&tmp);
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
                    let Some(agent) = name.strip_prefix("agent-").and_then(|s| s.strip_suffix(".jsonl")) else { continue };
                    let t0 = std::time::Instant::now();
                    let t = read_agent_transcript(&sess.path(), &wf_id, agent, 200).unwrap().unwrap();
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
    eprintln!("{n} transcripts; slowest {:?} {}", slowest.0, slowest.1.display());
}

/// Against this machine's real data:
/// `NODAL_SANDBOX=<sandbox repo path> cargo test -- --ignored`.
#[test]
#[ignore]
fn real_sessions_on_disk() {
    let sandbox = std::env::var("NODAL_SANDBOX").expect("NODAL_SANDBOX=<sandbox repo path>");
    let projects = claude_config_dir().unwrap().join("projects");
    for session in [DONE_SESSION, CUT_SESSION] {
        let dir = find_session_dir(&projects, &sandbox, session).expect("real session not found");
        let d = read_run_detail(&dir).expect("no workflow");
        eprintln!("{session}: {d:#?}");
    }
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
    let t = read_session_transcript(&path, "run-1", Some("Claude".into()), None, 2).unwrap().unwrap();
    assert_eq!(t.agent_id, "run-1");
    assert_eq!(t.prompt.as_deref(), Some("Fix the login bug"));
    assert_eq!((t.total_items, t.omitted, t.tool_calls), (3, 1, 1));
    assert_eq!(t.final_output.as_deref(), Some("Done."));
    assert!(!t.items.iter().any(|i| matches!(i, TranscriptItem::Text { text, .. } if text == "subagent")));
    assert!(read_session_transcript(&dir.join("nope.jsonl"), "x", None, None, 10).unwrap().is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn usage_tokens_dedupe_by_message_id() {
    let lines = [
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"a"}],"usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000}}}"#,
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t","name":"Bash","input":{}}],"usage":{"input_tokens":10,"output_tokens":5,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000}}}"#,
        r#"{"type":"user","message":{"content":"usage"}}"#,
        r#"{"type":"assistant","isSidechain":true,"message":{"id":"m2","content":[],"usage":{"input_tokens":1,"output_tokens":2}}}"#,
        "not json \"usage\" \"assistant\"",
    ];
    assert_eq!(usage_tokens_in(lines), Some(1115 + 3));
    assert_eq!(usage_tokens_in([r#"{"type":"assistant","message":{"content":[]}}"#]), None);
    let dir = std::env::temp_dir().join(format!("nodal-usage-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.jsonl");
    fs::write(&path, lines.join("\n")).unwrap();
    assert_eq!(read_usage_tokens(&path), Some(1118));
    assert_eq!(read_usage_tokens(&dir.join("nope.jsonl")), None);
    fs::remove_dir_all(&dir).ok();
}
