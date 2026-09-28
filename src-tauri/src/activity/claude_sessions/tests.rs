use super::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/activity/fixtures")
}

#[test]
fn agents_real_format_all_kinds() {
    let text = fs::read_to_string(fixtures().join("agents.json")).unwrap();
    let a = parse_agents(&text).unwrap();
    assert_eq!(a.len(), 5, "the entry without sessionId is dropped");
    let coord = a
        .iter()
        .find(|x| x.kind.as_deref() == Some("interactive"))
        .unwrap();
    assert_eq!(coord.id, None);
    assert_eq!(coord.status.as_deref(), Some("busy"));
    assert!(coord.alive());
    let blocked = a
        .iter()
        .find(|x| x.state.as_deref() == Some("blocked"))
        .unwrap();
    assert_eq!(blocked.waiting_for.as_deref(), Some("permission prompt"));
    assert!(blocked.alive());
    let done = a
        .iter()
        .find(|x| x.id.as_deref() == Some("43495c2e"))
        .unwrap();
    assert!(!done.alive());
    assert!(parse_agents("nope").is_err());
    assert_eq!(parse_agents("[]").unwrap(), vec![]);
}

#[test]
fn tail_detects_finished_running_and_tool() {
    let dir =
        fixtures().join("projects/-Users-me-Code/11111111-2222-3333-4444-555555555555/subagents");
    let done = parse_tail(&fs::read_to_string(dir.join("agent-adone00000000001.jsonl")).unwrap());
    assert!(done.finished && done.has_turns);
    assert_eq!(done.cwd.as_deref(), Some("/Users/me/Code"));
    let live = parse_tail(&fs::read_to_string(dir.join("agent-alive0000000001.jsonl")).unwrap());
    assert!(!live.finished && live.has_turns);
    assert_eq!(live.last_tool.as_deref(), Some("Bash"));
    assert_eq!(live.last_tool_summary.as_deref(), Some("cargo test"));
    assert_eq!(
        live.cwd.as_deref(),
        Some("/Users/me/Code/repo/.claude/worktrees/agent-alive0000000001")
    );
    assert_eq!(done.touched_paths, vec!["/Users/me/Code/x.rs".to_string()]);
    // Tail cut mid-line plus garbage: doesn't break.
    let t = parse_tail("{\"type\":\"assi\n{bad json\n");
    assert_eq!(t, TailInfo::default());
}

#[test]
fn meta_is_lenient() {
    let m = parse_meta(
        r#"{"agentType":"general-purpose","worktreePath":"/r/.claude/worktrees/a","spawnDepth":1,"description":42}"#,
    );
    assert_eq!(m.agent_type.as_deref(), Some("general-purpose"));
    assert_eq!(m.worktree_path.as_deref(), Some("/r/.claude/worktrees/a"));
    assert_eq!(m.description, None);
    assert_eq!(parse_meta("garbage"), SubagentMeta::default());
}

#[test]
fn finds_subagents_including_workflow_agents() {
    let dir = fixtures().join("projects/-Users-me-Code-repo/99999999-0000-0000-0000-000000000001");
    let mut files = subagent_files(&dir);
    files.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    let ids: Vec<_> = files
        .iter()
        .map(|f| {
            (
                f.agent_id.as_str(),
                f.workflow_id.as_deref(),
                f.meta.is_some(),
            )
        })
        .collect();
    assert_eq!(ids, vec![("aworkflow000000001", Some("wf_1234"), true)]);
}

#[test]
fn tail_reads_only_the_end() {
    let tmp =
        std::env::temp_dir().join(format!("nodal-activity-tail-{}.jsonl", std::process::id()));
    let mut s = String::new();
    for i in 0..2000 {
        s.push_str(&format!("{{\"type\":\"user\",\"n\":{i}}}\n"));
    }
    fs::write(&tmp, &s).unwrap();
    let tail = read_tail(&tmp, 256).unwrap();
    assert!(tail.len() <= 256);
    assert!(
        tail.lines().all(|l| l.starts_with('{') && l.ends_with('}')),
        "{tail}"
    );
    assert!(tail.contains("\"n\":1999"));
    fs::remove_file(&tmp).unwrap();
}
