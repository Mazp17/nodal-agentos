use super::*;

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
    assert_eq!(
        usage_tokens_in([r#"{"type":"assistant","message":{"content":[]}}"#]),
        None
    );
    let dir = std::env::temp_dir().join(format!("nodal-usage-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.jsonl");
    std::fs::write(&path, lines.join("\n")).unwrap();
    assert_eq!(read_usage_tokens(&path), Some(1118));
    assert_eq!(read_usage_tokens(&dir.join("nope.jsonl")), None);
    std::fs::remove_dir_all(&dir).ok();
}
