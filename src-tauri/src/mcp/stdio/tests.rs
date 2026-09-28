use super::*;

fn run(input: &str, call: impl FnMut(&str, Value) -> Result<Value, String>) -> Vec<Value> {
    run_with(input, &tools::definitions(), call)
}

fn run_with(
    input: &str,
    defs: &Value,
    mut call: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Vec<Value> {
    let mut out = Vec::new();
    serve(input.as_bytes(), &mut out, defs, &mut call).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn handshake_and_tool_list_work_without_the_app() {
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"x","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
    ]
    .join("\n");
    let out = run(&input, |_, _| panic!("no tool call expected"));
    assert_eq!(out.len(), 4, "notifications get no answer: {out:?}");
    assert_eq!(out[0]["id"], 1);
    assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "nodal");
    assert!(out[0]["result"]["capabilities"]["tools"].is_object());
    assert_eq!(out[1]["result"]["tools"], tools::definitions());
    assert_eq!(out[2]["result"], json!({}));
    assert_eq!(out[3]["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
}

#[test]
fn tool_calls_are_forwarded_and_errors_reach_the_agent() {
    let input = [
        r#"{"jsonrpc":"2.0","id":"a","method":"tools/call","params":{"name":"get_task","arguments":{"task":"PAY-1"}}}"#,
        r#"{"jsonrpc":"2.0","id":"b","method":"tools/call","params":{"name":"list_projects"}}"#,
        r#"{"jsonrpc":"2.0","id":"c","method":"tools/call","params":{"name":"launch_task"}}"#,
        r#"{"jsonrpc":"2.0","id":"d","method":"resources/list"}"#,
        "{nope",
        r#"{"jsonrpc":"2.0","id":9,"result":{}}"#,
    ]
    .join("\n");
    let mut calls = Vec::new();
    let out = run(&input, |tool, args| {
        calls.push((tool.to_string(), args.clone()));
        if tool == "get_task" {
            Ok(json!({"key": "PAY-1"}))
        } else {
            Err(OPEN_NODAL.to_string())
        }
    });
    assert_eq!(
        calls,
        [
            ("get_task".to_string(), json!({"task": "PAY-1"})),
            ("list_projects".to_string(), json!({}))
        ]
    );
    assert_eq!(out.len(), 5);
    assert_eq!(out[0]["result"]["isError"], false);
    let text: Value =
        serde_json::from_str(out[0]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["key"], "PAY-1");
    assert_eq!(out[1]["result"]["isError"], true);
    assert_eq!(out[1]["result"]["content"][0]["text"], OPEN_NODAL);
    assert_eq!(out[2]["error"]["code"], INVALID_PARAMS);
    assert_eq!(out[3]["error"]["code"], METHOD_NOT_FOUND);
    assert_eq!(
        (out[4]["error"]["code"].as_i64(), &out[4]["id"]),
        (Some(PARSE_ERROR), &Value::Null)
    );
}

#[test]
fn only_the_chat_server_lists_and_forwards_propose_task() {
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"propose_task","arguments":{"title":"x"}}}"#,
    ]
    .join("\n");
    let out = run(&input, |_, _| panic!("propose_task isn't an agent tool"));
    assert_eq!(out[0]["result"]["tools"], tools::definitions());
    assert_eq!(out[1]["error"]["code"], INVALID_PARAMS);

    let mut calls = Vec::new();
    let out = run_with(&input, &tools::chat_definitions(), |tool, args| {
        calls.push((tool.to_string(), args));
        Ok(json!({"created": false}))
    });
    assert_eq!(out[0]["result"]["tools"], tools::chat_definitions());
    assert_eq!(out[1]["result"]["isError"], false);
    assert_eq!(calls, [("propose_task".to_string(), json!({"title": "x"}))]);
}

#[test]
fn a_closed_app_means_open_nodal_first() {
    let t = crate::util::paths::tests::TempDir::new("mcpcl");
    let missing = t.0.join("mcp.sock");
    assert_eq!(
        forward(&missing, "list_projects", json!({})).unwrap_err(),
        OPEN_NODAL
    );
    // Stale socket file left by an app that quit.
    drop(std::os::unix::net::UnixListener::bind(&missing).unwrap());
    assert_eq!(
        forward(&missing, "list_projects", json!({})).unwrap_err(),
        OPEN_NODAL
    );
}
