use super::*;

#[test]
fn agent_report_from_fenced_block_at_the_end() {
    let msg = "Done. I changed the header.\n\nIntermediate example: {\"status\": \"blocked\"}\n\n```json\n{\"status\": \"done\", \"summary\": \"New logo in the header\", \"pr\": \"https://example.com/acme/web/pull/7\", \"branch\": \"nodal/pay-1-logo\"}\n```\n";
    let r = parse_agent_report(msg).unwrap();
    assert_eq!(r.status, ReportStatus::Done);
    assert_eq!(r.summary.as_deref(), Some("New logo in the header"));
    assert_eq!(r.pr.as_deref(), Some("https://example.com/acme/web/pull/7"));
    assert_eq!(r.branch.as_deref(), Some("nodal/pay-1-logo"));
}

#[test]
fn agent_report_blocked_loose_json_and_bad_fields() {
    let msg = "I couldn't run the tests. {\"status\":\"BLOCKED\",\"summary\":\"the DB is missing\",\"pr\":\"javascript:alert(1)\",\"branch\":\"--force\"}";
    let r = parse_agent_report(msg).unwrap();
    assert_eq!(r.status, ReportStatus::Blocked);
    assert_eq!(r.pr, None);
    assert_eq!(r.branch, None);
    assert_eq!(parse_agent_report("no report"), None);
    assert_eq!(parse_agent_report("{\"status\": \"maybe\"}"), None);
    assert_eq!(parse_agent_report("{\"status\": \"done\", \"pr\": null}").unwrap().pr, None);
    // Broken JSON at the end: the last valid one is used.
    let r = parse_agent_report("{\"status\":\"done\",\"summary\":\"ok\"} and then {\"status\": ").unwrap();
    assert_eq!(r.summary.as_deref(), Some("ok"));
}

#[test]
fn verdict_pass_and_fail() {
    let v = parse_verdict("I reviewed everything.\n```json\n{\"verdict\":\"pass\",\"unmet\":[],\"nits\":[\"rename x\"],\"summary\":\"Meets the criteria\"}\n```").unwrap();
    assert!(v.pass);
    assert!(v.unmet.is_empty());
    assert_eq!(v.nits, ["rename x"]);
    assert_eq!(v.summary.as_deref(), Some("Meets the criteria"));
    let v = parse_verdict("{\"verdict\":\"fail\",\"unmet\":[\"The logo doesn't show on mobile\", {\"id\": 2}],\"summary\":null}").unwrap();
    assert!(!v.pass);
    assert_eq!(v.unmet, ["The logo doesn't show on mobile", "{\"id\":2}"]);
    assert!(v.nits.is_empty());
    assert_eq!(v.summary, None);
    assert_eq!(parse_verdict("{\"verdict\":\"ok\"}"), None);
    assert_eq!(parse_verdict("nothing"), None);
}

#[test]
fn reads_last_message_from_session_lines() {
    // Real format observed in the spike (2.1.281): `assistant` lines with `text` blocks.
    let lines = [
        r#"{"type":"agent-setting","agentSetting":"frontend-developer"}"#,
        r#"{"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"Starting."}]}}"#,
        r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"subagent"}]}}"#,
        r#"{"type":"assistant","isSidechain":false,"message":{"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#,
        r#"{"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"Done.\n```json\n{\"status\":\"done\",\"summary\":\"s\"}\n```"}]}}"#,
        r#"{"type":"system","subtype":"turn_duration"}"#,
    ]
    .join("\n");
    let last = crate::runs::claude_fs::last_assistant_text_in(&lines).unwrap();
    assert!(last.starts_with("Done."));
    assert_eq!(parse_agent_report(&last).unwrap().status, ReportStatus::Done);
    assert_eq!(crate::runs::claude_fs::last_assistant_text_in("{\"type\":\"user\"}"), None);
}
