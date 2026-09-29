use super::*;

#[test]
fn agent_names() {
    assert!(is_valid_agent_name("code-reviewer"));
    assert!(is_valid_agent_name("pr-review-toolkit:code-reviewer"));
    assert!(!is_valid_agent_name("-x"));
    assert!(!is_valid_agent_name("a b"));
    assert!(!is_valid_agent_name(""));
}

#[test]
fn workflow_names() {
    assert!(is_valid_workflow_name("linear-issue"));
    assert!(is_valid_workflow_name("plugin:flow_2"));
    assert!(!is_valid_workflow_name("-x"));
    assert!(!is_valid_workflow_name("a b"));
    assert!(!is_valid_workflow_name(""));
}
