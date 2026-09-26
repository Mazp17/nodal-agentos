//! Spike for per-project chats: how `claude -p` in stream-json mode asks for permissions,
//! stops a turn and resumes a session. Verified against the real CLI (v2.1.283); the live
//! tests are `#[ignore]` (`cargo test --lib stream_json_live -- --ignored`), and
//! `fixtures/stream_json/` keeps the observed shapes (with made-up data) for the parser.
//!
//! Launch: `claude -p --input-format stream-json --output-format stream-json --verbose
//! --include-partial-messages --permission-prompt-tool stdio`, stdin piped (`claude_command`
//! sets it to null). `--output-format stream-json` with `-p` requires `--verbose`.
//!
//! - Permissions: `--permission-prompts host` alone (it is the default) does NOT reach the
//!   host: anything that would prompt is denied with a `system`/`permission_denied` event.
//!   `--permission-prompt-tool stdio` makes the CLI write a `control_request` with subtype
//!   `can_use_tool` (`tool_name`, `input`, `tool_use_id`, `description`,
//!   `permission_suggestions`) and wait. The host answers on stdin with a `control_response`
//!   whose `response.response` is `{behavior:"allow", updatedInput}` or
//!   `{behavior:"deny", message}`; a denial becomes an `is_error` `tool_result` with that message.
//! - Interrupt: a `control_request` `{subtype:"interrupt"}` on stdin is acknowledged with a
//!   `control_response` and ends the turn at once with `result`/`error_during_execution`.
//!   The process stays alive and takes the next user message in the same session.
//! - Resume: a new process with `--resume <session_id>` keeps the same id and the history,
//!   also after an interrupted turn. `claude --resume <id>` opens it interactively, but the
//!   bare `claude --resume` picker does not list `-p` sessions (their `entrypoint` is
//!   `sdk-cli`), with or without `--name`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

use crate::runs::claude_bin;

const TURN_TIMEOUT: Duration = Duration::from_secs(120);

fn fixture(name: &str) -> Vec<Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runs/fixtures/stream_json").join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("fixture line is JSON"))
        .collect()
}

fn find<'a>(events: &'a [Value], ty: &str, subtype: &str) -> &'a Value {
    events
        .iter()
        .find(|e| e["type"] == ty && (e["subtype"] == subtype || e["request"]["subtype"] == subtype || e["response"]["subtype"] == subtype))
        .unwrap_or_else(|| panic!("no {ty}/{subtype} event"))
}

#[test]
fn fixture_permission_request_has_the_fields_the_host_needs() {
    let events = fixture("permission_turn.jsonl");
    let req = find(&events, "control_request", "can_use_tool");
    assert!(req["request_id"].is_string());
    assert_eq!(req["request"]["tool_name"], "Write");
    assert!(req["request"]["input"].is_object());
    let tool_use_id = req["request"]["tool_use_id"].as_str().unwrap();
    let tool_use = events
        .iter()
        .flat_map(|e| e["message"]["content"].as_array().cloned().unwrap_or_default())
        .find(|b| b["type"] == "tool_use")
        .expect("tool_use block");
    assert_eq!(tool_use["id"], tool_use_id);
    assert_eq!(find(&events, "result", "success")["session_id"], events[0]["session_id"]);
}

#[test]
fn fixture_denial_and_interrupt_shapes() {
    let denied = fixture("denied_turn.jsonl");
    let result = denied
        .iter()
        .flat_map(|e| e["message"]["content"].as_array().cloned().unwrap_or_default())
        .find(|b| b["type"] == "tool_result")
        .expect("tool_result");
    assert_eq!(result["is_error"], true);
    assert_eq!(result["content"], "The user denied this in Nodal.");

    let interrupted = fixture("interrupted_turn.jsonl");
    let ack = find(&interrupted, "control_response", "success");
    assert_eq!(ack["response"]["request_id"], "interrupt-1");
    assert_eq!(find(&interrupted, "result", "error_during_execution")["is_error"], true);
    assert!(interrupted.iter().any(|e| e["type"] == "stream_event" && e["event"]["delta"]["type"] == "text_delta"));

    let no_host = fixture("no_permission_tool.jsonl");
    assert_eq!(find(&no_host, "system", "permission_denied")["tool_name"], "Write");
    assert!(!no_host.iter().any(|e| e["type"] == "control_request"));
}

struct Chat {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Chat {
    fn start(cwd: &Path, extra: &[&str]) -> Chat {
        let mut cmd = claude_bin::claude_command().expect("claude");
        cmd.args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"])
            .args(["--include-partial-messages", "--permission-prompt-tool", "stdio", "--model", "haiku"])
            .args(extra)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().expect("spawn claude");
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        Chat { child, stdin, lines }
    }

    async fn send(&mut self, v: Value) {
        self.stdin.write_all(format!("{v}\n").as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn say(&mut self, text: &str) {
        self.send(json!({"type": "user", "message": {"role": "user", "content": text}})).await;
    }

    /// Reads events until the turn's `result`, letting `on` answer on stdin.
    async fn turn(&mut self, mut on: impl FnMut(&Value) -> Option<Value>) -> (Value, Vec<Value>) {
        let mut seen = Vec::new();
        loop {
            let line = tokio::time::timeout(TURN_TIMEOUT, self.lines.next_line())
                .await
                .expect("turn timed out")
                .unwrap()
                .expect("claude exited mid-turn");
            let ev: Value = serde_json::from_str(&line).unwrap();
            if let Some(reply) = on(&ev) {
                self.send(reply).await;
            }
            if ev["type"] == "result" {
                return (ev, seen);
            }
            seen.push(ev);
        }
    }

    async fn close(mut self) {
        drop(self.stdin);
        let _ = tokio::time::timeout(Duration::from_secs(30), self.child.wait()).await;
    }
}

fn answer(req: &Value, allow: bool) -> Value {
    let response = if allow {
        json!({"behavior": "allow", "updatedInput": req["request"]["input"]})
    } else {
        json!({"behavior": "deny", "message": "The user denied this in Nodal."})
    };
    json!({"type": "control_response", "response": {"subtype": "success", "request_id": req["request_id"], "response": response}})
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nodal-stream-json-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn is_permission_request(ev: &Value) -> bool {
    ev["type"] == "control_request" && ev["request"]["subtype"] == "can_use_tool"
}

#[test]
#[ignore]
fn real_permission_prompts_reach_the_host() {
    let dir = scratch();
    tauri::async_runtime::block_on(async {
        for (file, allow) in [("allowed.txt", true), ("denied.txt", false)] {
            let mut chat = Chat::start(&dir, &[]);
            chat.say(&format!("Use the Write tool to create {file} containing 'x'. Then reply DONE.")).await;
            let mut asked = 0;
            let (result, _) = chat
                .turn(|ev| {
                    is_permission_request(ev).then(|| {
                        asked += 1;
                        answer(ev, allow)
                    })
                })
                .await;
            chat.close().await;
            assert!(asked >= 1, "{file}: no can_use_tool request");
            assert_eq!(result["subtype"], "success");
            assert_eq!(dir.join(file).exists(), allow, "{file}");
        }
    });
}

#[test]
#[ignore]
fn real_interrupt_then_resume() {
    let dir = scratch();
    tauri::async_runtime::block_on(async {
        let mut chat = Chat::start(&dir, &[]);
        chat.say("Write a 600-word essay about lighthouses. Use no tools.").await;
        let mut sent = false;
        let (result, seen) = chat
            .turn(|ev| {
                let delta = ev["type"] == "stream_event" && ev["event"]["delta"]["type"] == "text_delta";
                (delta && !std::mem::replace(&mut sent, true))
                    .then(|| json!({"type": "control_request", "request_id": "interrupt-1", "request": {"subtype": "interrupt"}}))
            })
            .await;
        assert!(sent, "no text delta streamed");
        assert_eq!(result["subtype"], "error_during_execution");
        assert!(seen.iter().any(|e| e["type"] == "control_response" && e["response"]["request_id"] == "interrupt-1"));
        let session = result["session_id"].as_str().unwrap().to_string();

        chat.say("Reply with exactly: ALIVE").await;
        let (again, _) = chat.turn(|_| None).await;
        assert_eq!(again["subtype"], "success");
        assert_eq!(again["session_id"], session.as_str());
        chat.close().await;

        let mut resumed = Chat::start(&dir, &["--resume", &session]);
        resumed.say("What single word did I last ask you to reply with? Reply with that word only.").await;
        let (last, _) = resumed.turn(|_| None).await;
        resumed.close().await;
        assert_eq!(last["session_id"], session.as_str());
        assert!(last["result"].as_str().unwrap_or_default().contains("ALIVE"), "{last}");
    });
}
