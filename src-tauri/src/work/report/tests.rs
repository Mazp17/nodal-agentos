use super::*;

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
    assert_eq!(
        parse_agent_report(&last).unwrap().status,
        ReportStatus::Done
    );
    assert_eq!(
        crate::runs::claude_fs::last_assistant_text_in("{\"type\":\"user\"}"),
        None
    );
}
