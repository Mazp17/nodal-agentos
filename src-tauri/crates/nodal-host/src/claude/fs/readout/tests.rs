use super::*;
use crate::testutil::TempDir;

#[test]
fn read_session_close_rejects_invalid_session_ids() {
    let (readout, tokens) = read_session_close("../etc", "/x");
    assert!(readout.detail.is_none());
    assert!(readout.last_message.is_none());
    assert!(readout.blocker.is_none());
    assert_eq!(tokens, None);
}

/// The single scan `read_session_close` runs over the transcript: same data `session_tokens`
/// + `read_session` used to compute from 3 separate opens (P12).
#[test]
fn scan_jsonl_reads_the_file_once_for_all_three_fields() {
    let t = TempDir::new("readout-scan");
    let path = t.0.join("s.jsonl");
    let lines = [
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"toolu_1","name":"Workflow","input":{"name":"plan-task"}}],"usage":{"input_tokens":10,"output_tokens":5}}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"Review dynamic workflow before running","is_error":true,"tool_use_id":"toolu_1"}]}}"#,
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":1,"output_tokens":1}}}"#,
    ];
    std::fs::write(&path, lines.join("\n")).unwrap();

    let (last_message, blocker, tokens) = scan_jsonl(&path);
    assert_eq!(last_message.as_deref(), Some("Done."));
    assert_eq!(blocker, Some(Some("plan-task".to_string())));
    assert_eq!(tokens, Some(17));
}

#[test]
fn scan_jsonl_missing_file_returns_none_for_everything() {
    let t = TempDir::new("readout-scan-missing");
    assert_eq!(scan_jsonl(&t.0.join("nope.jsonl")), (None, None, None));
}
