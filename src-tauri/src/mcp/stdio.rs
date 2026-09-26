//! The `nodal-mcp` side: MCP over stdio (newline-delimited JSON-RPC 2.0). It answers the
//! handshake and `tools/list` itself, so it registers even while the app is closed, and
//! forwards each `tools/call` to the app's socket.

use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{json, Value};

use super::{tools, Reply, Request, MAX_LINE};
use crate::util::paths;

pub const OPEN_NODAL: &str =
    "Open Nodal first: agents can only reach it while the app is running and its MCP server is on (Settings → Diagnostics).";
const TIMEOUT: Duration = Duration::from_secs(30);
/// Newest first; an unknown version requested by the client gets the newest.
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Entry point of the `nodal-mcp` binary.
pub fn main() -> ExitCode {
    let socket = paths::data_dir_standalone().map(|d| paths::mcp_socket(&d));
    let call = |tool: &str, args: Value| match &socket {
        Ok(s) => forward(s, tool, args),
        Err(e) => Err(e.clone()),
    };
    match serve(io::stdin().lock(), io::stdout().lock(), call) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nodal-mcp: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Sends one tool request to the app and waits for its answer.
pub fn forward(socket: &Path, tool: &str, args: Value) -> Result<Value, String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| match e.kind() {
        ErrorKind::NotFound | ErrorKind::ConnectionRefused => OPEN_NODAL.to_string(),
        _ => format!("Couldn't reach Nodal at {}: {e}", socket.display()),
    })?;
    let lost = |e: io::Error| format!("Lost the connection to Nodal: {e}");
    stream.set_read_timeout(Some(TIMEOUT)).map_err(lost)?;
    let mut line = serde_json::to_vec(&Request { tool: tool.to_string(), arguments: args }).map_err(|e| e.to_string())?;
    line.push(b'\n');
    stream.write_all(&line).map_err(lost)?;
    stream.shutdown(std::net::Shutdown::Write).map_err(lost)?;
    let mut reply = String::new();
    BufReader::new(stream).take(MAX_LINE).read_line(&mut reply).map_err(lost)?;
    if reply.trim().is_empty() {
        return Err("Nodal closed the connection without answering.".into());
    }
    match serde_json::from_str::<Reply>(&reply) {
        Ok(Reply::Result(v)) => Ok(v),
        Ok(Reply::Error(e)) => Err(e),
        Err(e) => Err(format!("Unexpected answer from Nodal: {e}")),
    }
}

/// Reads JSON-RPC messages until stdin closes. `call` runs a tool.
pub(crate) fn serve(
    input: impl BufRead,
    mut output: impl Write,
    mut call: impl FnMut(&str, Value) -> Result<Value, String>,
) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg, &mut call),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, &format!("Parse error: {e}"))),
        };
        if let Some(r) = reply {
            serde_json::to_writer(&mut output, &r)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// The answer to one message; `None` for notifications and responses.
fn handle(msg: &Value, call: &mut impl FnMut(&str, Value) -> Result<Value, String>) -> Option<Value> {
    let id = msg.get("id").filter(|id| !id.is_null()).cloned();
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        let is_response = msg.get("result").is_some() || msg.get("error").is_some();
        return (!is_response).then(|| error(id.unwrap_or_default(), INVALID_REQUEST, "Invalid request."));
    };
    let id = id?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => initialize(&params),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools::definitions()}),
        "tools/call" => match tool_call(&params, call) {
            Ok(r) => r,
            Err(e) => return Some(error(id, INVALID_PARAMS, &e)),
        },
        _ => return Some(error(id, METHOD_NOT_FOUND, &format!("Method not found: {method}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked.filter(|v| PROTOCOL_VERSIONS.contains(v)).unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "nodal", "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Tasks on the Nodal board: list projects and tasks, create and update tasks, and read run results. Runs are launched from the Nodal app, not from here. Nodal must be open."
    })
}

/// A failing tool is a result with `isError` (the agent reads it); a malformed call is a
/// JSON-RPC error.
fn tool_call(params: &Value, call: &mut impl FnMut(&str, Value) -> Result<Value, String>) -> Result<Value, String> {
    let name = params.get("name").and_then(Value::as_str).ok_or("Missing tool name.")?;
    let known = tools::definitions().as_array().is_some_and(|d| d.iter().any(|t| t["name"] == name));
    if !known {
        return Err(format!("Unknown tool: {name}."));
    }
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let (text, is_error) = match call(name, args) {
        Ok(v) => (serde_json::to_string_pretty(&v).unwrap_or_default(), false),
        Err(e) => (e, true),
    };
    Ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &str, mut call: impl FnMut(&str, Value) -> Result<Value, String>) -> Vec<Value> {
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, &mut call).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
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
        assert_eq!(calls, [("get_task".to_string(), json!({"task": "PAY-1"})), ("list_projects".to_string(), json!({}))]);
        assert_eq!(out.len(), 5);
        assert_eq!(out[0]["result"]["isError"], false);
        let text: Value = serde_json::from_str(out[0]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text["key"], "PAY-1");
        assert_eq!(out[1]["result"]["isError"], true);
        assert_eq!(out[1]["result"]["content"][0]["text"], OPEN_NODAL);
        assert_eq!(out[2]["error"]["code"], INVALID_PARAMS);
        assert_eq!(out[3]["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!((out[4]["error"]["code"].as_i64(), &out[4]["id"]), (Some(PARSE_ERROR), &Value::Null));
    }

    #[test]
    fn a_closed_app_means_open_nodal_first() {
        let t = crate::util::paths::tests::TempDir::new("mcpcl");
        let missing = t.0.join("mcp.sock");
        assert_eq!(forward(&missing, "list_projects", json!({})).unwrap_err(), OPEN_NODAL);
        // Stale socket file left by an app that quit.
        drop(std::os::unix::net::UnixListener::bind(&missing).unwrap());
        assert_eq!(forward(&missing, "list_projects", json!({})).unwrap_err(), OPEN_NODAL);
    }
}
