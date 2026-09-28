use super::*;

#[test]
fn workflow_review_denial_from_real_session() {
    // Real (trimmed) lines from a claude 2.1.281 --bg session with an unapproved plan-task.
    let text = concat!(
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_01D3","name":"Workflow","input":{"name":"plan-task","args":{"finish":"branch"}}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Review dynamic workflow before running","is_error":true,"tool_use_id":"toolu_01D3"}]},"toolUseResult":"Error: Review dynamic workflow before running"}"#,
        "\n",
    );
    assert_eq!(
        find_workflow_review_denial(text),
        Some(Some("plan-task".into()))
    );
    // Without the call (or with another format): still detected, without a name.
    let only = text.lines().nth(1).unwrap();
    assert_eq!(find_workflow_review_denial(only), Some(None));
    // An assistant text quoting the message doesn't count.
    let quoted = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Review dynamic workflow before running"}]}}"#;
    assert_eq!(find_workflow_review_denial(quoted), None);
    assert_eq!(find_workflow_review_denial("{broken\n"), None);
}
