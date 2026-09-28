//! The `nodal-mcp` side: MCP over stdio (newline-delimited JSON-RPC 2.0). It answers the
//! handshake and `tools/list` itself, so it registers even while the app is closed, and
//! forwards each `tools/call` to the app's socket. `nodal-mcp --chat` (how Nodal's chats
//! launch it) also lists `propose_task`.

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

/// Makes `nodal-mcp` serve the chat's tools (`tools::chat_definitions`).
pub const CHAT_FLAG: &str = "--chat";

/// Entry point of the `nodal-mcp` binary.
pub fn main() -> ExitCode {
    let defs = if std::env::args().skip(1).any(|a| a == CHAT_FLAG) { tools::chat_definitions() } else { tools::definitions() };
    let socket = paths::data_dir_standalone().map(|d| paths::mcp_socket(&d));
    let call = |tool: &str, args: Value| match &socket {
        Ok(s) => forward(s, tool, args),
        Err(e) => Err(e.clone()),
    };
    match serve(io::stdin().lock(), io::stdout().lock(), &defs, call) {
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

/// Reads JSON-RPC messages until stdin closes. `defs` are the tools it lists; `call` runs one.
pub(crate) fn serve(
    input: impl BufRead,
    mut output: impl Write,
    defs: &Value,
    mut call: impl FnMut(&str, Value) -> Result<Value, String>,
) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg, defs, &mut call),
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
fn handle(msg: &Value, defs: &Value, call: &mut impl FnMut(&str, Value) -> Result<Value, String>) -> Option<Value> {
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
        "tools/list" => json!({"tools": defs}),
        "tools/call" => match tool_call(&params, defs, call) {
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
fn tool_call(params: &Value, defs: &Value, call: &mut impl FnMut(&str, Value) -> Result<Value, String>) -> Result<Value, String> {
    let name = params.get("name").and_then(Value::as_str).ok_or("Missing tool name.")?;
    let known = defs.as_array().is_some_and(|d| d.iter().any(|t| t["name"] == name));
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
mod tests;
