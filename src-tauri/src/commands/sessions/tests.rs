use super::*;

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
    assert!(call("wf_../../etc", "a1")
        .unwrap_err()
        .contains("Invalid workflow run id"));
    assert!(call("../wf_x", "a1")
        .unwrap_err()
        .contains("Invalid workflow run id"));
    assert!(call("wf_abc", "../../x")
        .unwrap_err()
        .contains("Invalid agent id"));
    assert!(call("wf_abc", "a/b")
        .unwrap_err()
        .contains("Invalid agent id"));
}
