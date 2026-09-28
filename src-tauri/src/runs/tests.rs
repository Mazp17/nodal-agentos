use super::*;

#[test]
fn append_system_prompt_is_one_arg_after_the_variadic_lists() {
    let extra = ExtraFlags {
        allowed_tools: vec!["Bash(npm test:*)".into()],
        append_system_prompt: Some("Don't ask: stop.".into()),
        ..Default::default()
    };
    assert_eq!(extra.to_args(), ["--allowedTools", "Bash(npm test:*)", "--append-system-prompt", "Don't ask: stop."]);
    assert!(ExtraFlags::default().to_args().is_empty());
}

/// Against the real `claude` and this machine's data: `cargo test -- --ignored`.
/// Read-only (`claude agents` + files); doesn't launch runs.
#[test]
#[ignore]
fn real_list_and_detail() {
    let runs = tauri::async_runtime::block_on(list_runs()).expect("list_runs");
    eprintln!("{} background runs", runs.len());
    for r in runs.iter().take(5) {
        let cwd = r.cwd.clone().unwrap_or_default();
        let d = tauri::async_runtime::block_on(get_run_detail(r.session_id.clone(), cwd)).expect("detail");
        eprintln!(
            "{} {:?} {:?} -> {:?}",
            r.id,
            r.state,
            r.name,
            d.map(|d| (d.workflow_id, d.source, d.current_phase, d.agents.len()))
        );
    }
    let err = tauri::async_runtime::block_on(launch_with("/no/such/dir".into(), "x".into(), &LaunchOptions::default(), &ExtraFlags::default()))
        .unwrap_err();
    assert_eq!(err, "The folder doesn't exist or isn't a directory: /no/such/dir");
}

/// Against the real `claude`: `cargo test -- --ignored`. Launches two runs (plain and with
/// `--agent code-reviewer`, which must exist in `~/.claude/agents`) and stops them.
#[test]
#[ignore]
fn real_launch_with_append_system_prompt() {
    let cwd = env!("CARGO_MANIFEST_DIR").to_string();
    let prompt = "Reply with the single word OK.".to_string();
    let sp = Some(crate::work::launch::UNATTENDED_SYSTEM_PROMPT.to_string());
    for agent in [None, Some("code-reviewer".to_string())] {
        let extra = ExtraFlags { agent: agent.clone(), append_system_prompt: sp.clone(), ..Default::default() };
        let run = tauri::async_runtime::block_on(launch_with(cwd.clone(), prompt.clone(), &LaunchOptions::default(), &extra))
            .unwrap_or_else(|e| panic!("launch with agent {agent:?}: {e}"));
        eprintln!("agent {agent:?} -> backgrounded · {}", run.id);
        tauri::async_runtime::block_on(terminal::stop(&run.id)).expect("stop");
    }
}

#[test]
fn transcript_rejects_traversal_ids() {
    let call = |run: &str, agent: &str| {
        tauri::async_runtime::block_on(get_agent_transcript(
            "ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8".into(),
            "/x".into(),
            run.into(),
            agent.into(),
            None,
        ))
    };
    assert!(call("wf_../../etc", "a1").unwrap_err().contains("Invalid workflow run id"));
    assert!(call("../wf_x", "a1").unwrap_err().contains("Invalid workflow run id"));
    assert!(call("wf_abc", "../../x").unwrap_err().contains("Invalid agent id"));
    assert!(call("wf_abc", "a/b").unwrap_err().contains("Invalid agent id"));
}
