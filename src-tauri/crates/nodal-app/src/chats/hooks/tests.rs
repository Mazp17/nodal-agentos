//! Drives a real `ChatProcesses` (with `ChatHooksImpl` wired to an in-memory database) against
//! a fake `claude` that speaks the fixtures' stream-json (was `chats::process::tests`, through
//! the old bridge `Chats` wrapper).
#![allow(clippy::disallowed_methods)] // spins up a fake `claude` script on disk

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::path::Path;

use nodal_domain::error::HostError;
use nodal_domain::model::activity::{AgentSession, AppRuns, ExternalSessions};
use nodal_domain::model::chat::{ChatEnvelope, ChatEvent, ChatLive, ChatSpec, HostEvent, RunState, StreamEvent};
use nodal_domain::model::claude::{LaunchBlocker, RunDetail, SessionReadout, Transcript, TranscriptItem};
use nodal_domain::model::{Chat, LaunchOptions};
use nodal_domain::ports::{ChatHooks, ChatRuntime, ChatSink, SessionFiles};
use nodal_domain::testutil::project_of;
use nodal_host::adapters::HostSessionFiles;
use nodal_host::claude::chats::ChatProcesses;
use nodal_store::{chats, rows, Db};

use crate::chats::hooks::ChatHooksImpl;
use crate::testutil::{block_on, rt, NoopNotifier};

const SESSION: &str = "00000000-0000-4000-8000-000000000009";

/// Records its arguments in `args.txt` (in its cwd) and answers each stdin line: `write`
/// asks for permission first, `slow` waits for an interrupt, `crash` fails.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
echo "$@" >> args.txt
echo "$CLAUDE_CODE_ENTRYPOINT" > entrypoint.txt
echo '{"type":"system","subtype":"init","session_id":"00000000-0000-4000-8000-000000000009","cwd":"/Users/me/Code/acme"}'
while IFS= read -r line; do
  case "$line" in
    *'"subtype":"interrupt"'*)
      echo '{"type":"result","subtype":"error_during_execution","is_error":true}' ;;
    *'"type":"user"'*)
      echo "$line"
      case "$line" in
        *write*)
          echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01","name":"Write","input":{"file_path":"/Users/me/Code/acme/a.txt","content":"x"}}]},"parent_tool_use_id":null}'
          echo '{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/Users/me/Code/acme/a.txt","content":"x"},"tool_use_id":"toolu_01"}}'
          IFS= read -r answer
          case "$answer" in
            *'"behavior":"allow"'*'"updatedInput"'*) echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"written"}]},"parent_tool_use_id":null}' ;;
            *) echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"denied","is_error":true}]},"parent_tool_use_id":null}' ;;
          esac
          echo '{"type":"result","subtype":"success","is_error":false}' ;;
        *slow*) ;;
        *crash*) echo "boom: the model is unavailable" >&2; exit 3 ;;
        *)
          echo '{"type":"stream_event","event":{"delta":{"type":"text_delta","text":"Hi"}},"parent_tool_use_id":null}'
          echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Hi there"}]},"parent_tool_use_id":null}'
          echo '{"type":"result","subtype":"success","is_error":false,"session_id":"00000000-0000-4000-8000-000000000009"}' ;;
      esac ;;
  esac
done
"#;

struct RecordingSink(Arc<Mutex<Vec<ChatEnvelope>>>);

impl ChatSink for RecordingSink {
    fn emit(&self, ev: ChatEnvelope) {
        self.0.lock().unwrap().push(ev);
    }
}

struct Fixture {
    chats: Arc<ChatProcesses>,
    db: Db,
    events: Arc<Mutex<Vec<ChatEnvelope>>>,
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("nodal-chats-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("claude");
        std::fs::write(&program, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let db = Db::open_in_memory().unwrap();
        {
            let c = db.lock().unwrap();
            rows::insert_project(&c, &project_of("p1", "PAY")).unwrap();
            let chat = Chat {
                id: "c1".into(),
                project_id: "p1".into(),
                repo_id: None,
                title: None,
                session_title: None,
                session_id: None,
                launch: LaunchOptions::default(),
                created_at: 1,
                updated_at: 1,
            };
            rows::insert_chat(&c, &chat).unwrap();
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<dyn ChatSink> = Arc::new(RecordingSink(events.clone()));
        let hooks: Arc<dyn ChatHooks> =
            Arc::new(ChatHooksImpl::new(db.clone(), Arc::new(NoopNotifier), Arc::new(HostSessionFiles), None, rt()));
        let chats = ChatProcesses::with_program(sink, hooks, rt(), Some(program));
        Fixture { chats, db, events, dir }
    }

    fn spec(&self, args: &[&str]) -> ChatSpec {
        ChatSpec { cwd: self.dir.clone(), args: args.iter().map(|s| s.to_string()).collect() }
    }

    fn events(&self) -> Vec<ChatEvent> {
        self.events.lock().unwrap().iter().map(|e| e.event.clone()).collect()
    }

    /// Waits until `n` events match.
    async fn wait_for(&self, n: usize, what: &str, pred: impl Fn(&ChatEvent) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.events().iter().filter(|e| pred(e)).count() < n {
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {:#?}", self.events());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_turn_end(&self, n: usize) {
        self.wait_for(n, "turn end", |e| matches!(e, ChatEvent::Stream(StreamEvent::TurnEnd { .. }))).await;
    }

    async fn wait_stopped(&self, n: usize) {
        self.wait_for(n, "stopped", |e| *e == ChatEvent::Host(HostEvent::State { state: RunState::Stopped })).await;
    }

    fn args(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("args.txt")).unwrap_or_default().lines().map(String::from).collect()
    }

    fn session(&self) -> Option<String> {
        chats::get(&self.db.lock().unwrap(), "c1").unwrap().session_id
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.chats.clear();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn is_item(e: &ChatEvent, pred: impl Fn(&TranscriptItem) -> bool) -> bool {
    matches!(e, ChatEvent::Stream(StreamEvent::Item { item }) if pred(item))
}

#[test]
fn streams_a_turn_saves_the_session_and_resumes_after_idle() {
    let f = Fixture::new("resume");
    block_on(async {
        f.chats.send("c1", "p1", None, f.spec(&["--model", "haiku"]), "Hello").unwrap();
        assert_eq!(f.chats.live("c1").state, RunState::Busy);
        f.wait_turn_end(1).await;
        f.wait_for(1, "idle", |e| *e == ChatEvent::Host(HostEvent::State { state: RunState::Idle })).await;

        let ev = f.events();
        assert_eq!(ev[0], ChatEvent::Host(HostEvent::State { state: RunState::Busy }));
        assert!(ev.iter().any(|e| matches!(e, ChatEvent::Stream(StreamEvent::Init { session_id, .. }) if session_id == SESSION)));
        assert!(ev.iter().any(|e| is_item(e, |i| matches!(i, TranscriptItem::User { text, .. } if text == "Hello"))));
        assert!(ev.iter().any(|e| *e == ChatEvent::Stream(StreamEvent::TextDelta { text: "Hi".into() })));
        assert!(ev.iter().any(|e| is_item(e, |i| matches!(i, TranscriptItem::Text { text, .. } if text == "Hi there"))));
        assert_eq!(f.chats.live("c1"), ChatLive { state: RunState::Idle, pending: vec![] });

        let deadline = Instant::now() + Duration::from_secs(5);
        while f.session().is_none() {
            assert!(Instant::now() < deadline, "session id never saved");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(f.session().as_deref(), Some(SESSION));

        // Still recent: it stays. Idle for long enough: it's stopped.
        assert_eq!(f.chats.reap_idle(Duration::from_secs(60), Instant::now()), 0);
        assert_eq!(f.chats.reap_idle(Duration::from_secs(60), Instant::now() + Duration::from_secs(61)), 1);
        f.wait_stopped(1).await;
        assert_eq!(f.chats.live("c1").state, RunState::Stopped);
        assert!(!f.events().iter().any(|e| matches!(e, ChatEvent::Host(HostEvent::Error { .. }))), "a stop is not an error");

        // The next message starts it again on the same session.
        f.chats.send("c1", "p1", Some(SESSION), f.spec(&["--model", "haiku"]), "Again").unwrap();
        f.wait_turn_end(2).await;
        let args = f.args();
        assert_eq!(args.len(), 2, "{args:?}");
        assert!(!args[0].contains("--resume"));
        assert!(args[0].starts_with("-p --input-format stream-json --output-format stream-json --verbose"), "{}", args[0]);
        assert!(args[1].ends_with(&format!("--model haiku --resume {SESSION}")), "{}", args[1]);
        assert_eq!(std::fs::read_to_string(f.dir.join("entrypoint.txt")).unwrap().trim(), "nodal");
    });
}

#[test]
fn permission_requests_are_answered_from_the_host() {
    let f = Fixture::new("permission");
    block_on(async {
        f.chats.send("c1", "p1", None, f.spec(&[]), "please write a.txt").unwrap();
        f.wait_for(1, "permission request", |e| matches!(e, ChatEvent::Stream(StreamEvent::PermissionRequest(_)))).await;
        let live = f.chats.live("c1");
        assert_eq!(live.state, RunState::Busy);
        assert_eq!(live.pending.len(), 1);
        assert_eq!(live.pending[0].request_id, "req-1");
        assert_eq!(live.pending[0].summary.as_deref(), Some("/Users/me/Code/acme/a.txt"));

        assert!(f.chats.respond("c1", "nope", true, None).unwrap_err().to_string().contains("no longer pending"));
        f.chats.respond("c1", "req-1", true, None).unwrap();
        f.wait_turn_end(1).await;
        let ev = f.events();
        assert!(ev.contains(&ChatEvent::Host(HostEvent::PermissionResolved { request_id: "req-1".into(), allowed: true })));
        assert!(ev.iter().any(|e| matches!(e, ChatEvent::Stream(StreamEvent::ToolResult { result, .. }) if result.text == "written")));
        assert!(ev.iter().any(|e| is_item(e, |i| matches!(i, TranscriptItem::ToolUse { name, .. } if name == "Write"))));
        assert!(f.chats.live("c1").pending.is_empty());

        f.chats.send("c1", "p1", None, f.spec(&[]), "write it again").unwrap();
        f.wait_for(2, "permission request", |e| matches!(e, ChatEvent::Stream(StreamEvent::PermissionRequest(_)))).await;
        f.chats.respond("c1", "req-1", false, Some("Not that file.")).unwrap();
        f.wait_turn_end(2).await;
        assert!(f.events().iter().any(|e| matches!(e, ChatEvent::Stream(StreamEvent::ToolResult { result, .. }) if result.is_error)));
        assert_eq!(f.args().len(), 1, "one process for the whole chat");
    });
    assert!(f.chats.respond("missing", "req-1", true, None).unwrap_err().to_string().contains("isn't running"));
}

#[test]
fn interrupt_ends_the_turn_and_keeps_the_process() {
    let f = Fixture::new("interrupt");
    block_on(async {
        f.chats.interrupt("c1").unwrap();
        f.chats.send("c1", "p1", None, f.spec(&[]), "slow essay").unwrap();
        f.wait_for(1, "replayed message", |e| matches!(e, ChatEvent::Stream(StreamEvent::Item { .. }))).await;
        f.chats.interrupt("c1").unwrap();
        f.wait_turn_end(1).await;
        assert!(f.events().iter().any(|e| matches!(e, ChatEvent::Stream(StreamEvent::TurnEnd { ok: false, .. }))));
        assert_eq!(f.chats.live("c1").state, RunState::Idle);

        f.chats.send("c1", "p1", None, f.spec(&[]), "Hello").unwrap();
        f.wait_turn_end(2).await;
        assert_eq!(f.args().len(), 1);
    });
}

#[test]
fn new_settings_restart_an_idle_process() {
    let f = Fixture::new("restart");
    block_on(async {
        f.chats.send("c1", "p1", None, f.spec(&["--model", "haiku"]), "Hello").unwrap();
        f.wait_turn_end(1).await;
        f.chats.send("c1", "p1", Some(SESSION), f.spec(&["--model", "opus"]), "Hello").unwrap();
        f.wait_turn_end(2).await;
        let args = f.args();
        assert_eq!(args.len(), 2, "{args:?}");
        assert!(args[1].ends_with(&format!("--model opus --resume {SESSION}")), "{}", args[1]);
        // The replaced process doesn't report the chat as stopped.
        assert!(!f.events().contains(&ChatEvent::Host(HostEvent::State { state: RunState::Stopped })));
        assert_eq!(f.chats.live("c1").state, RunState::Idle);

        f.chats.stop_project("p2");
        assert_eq!(f.chats.live("c1").state, RunState::Idle);
        f.chats.stop_project("p1");
        f.wait_stopped(1).await;
    });
}

#[test]
fn a_crash_mid_turn_is_reported() {
    let f = Fixture::new("crash");
    block_on(async {
        f.chats.send("c1", "p1", None, f.spec(&[]), "crash now").unwrap();
        f.wait_stopped(1).await;
        let err = f.events().into_iter().find_map(|e| match e {
            ChatEvent::Host(HostEvent::Error { message }) => Some(message),
            _ => None,
        });
        assert_eq!(err.as_deref(), Some("boom: the model is unavailable"));
        assert_eq!(f.chats.live("c1").state, RunState::Stopped);
    });
}

#[test]
fn refuses_a_missing_folder_and_a_bad_session() {
    let f = Fixture::new("refuse");
    let missing = ChatSpec { cwd: std::path::Path::new("/Users/me/does-not-exist").into(), args: vec![] };
    assert!(f.chats.send("c1", "p1", None, missing, "Hello").unwrap_err().to_string().contains("doesn't exist"));
    let err = block_on(async { f.chats.send("c1", "p1", Some("../x"), f.spec(&[]), "Hello") });
    assert!(err.unwrap_err().to_string().contains("Invalid session id"));
    assert_eq!(f.chats.live("c1").state, RunState::Stopped);
    assert!(f.events().is_empty());
}

#[test]
fn event_payload_shape() {
    let env = ChatEnvelope { chat_id: "c1".into(), event: ChatEvent::Host(HostEvent::State { state: RunState::Busy }) };
    assert_eq!(serde_json::to_value(env).unwrap(), serde_json::json!({"chatId": "c1", "event": {"type": "state", "state": "busy"}}));
    let env = ChatEnvelope { chat_id: "c1".into(), event: ChatEvent::Stream(StreamEvent::TextDelta { text: "Hi".into() }) };
    assert_eq!(serde_json::to_value(env).unwrap(), serde_json::json!({"chatId": "c1", "event": {"type": "textDelta", "text": "Hi"}}));
    let ev = ChatEvent::Host(HostEvent::PermissionResolved { request_id: "r".into(), allowed: false });
    assert_eq!(serde_json::to_value(ev).unwrap(), serde_json::json!({"type": "permissionResolved", "requestId": "r", "allowed": false}));
}

/// The real `claude` (haiku): a permission prompt answered here, then a resume after a stop,
/// on a session the `claude --resume` picker lists.
/// `cargo test --lib real_chat -- --ignored`.
#[test]
#[ignore]
fn real_chat_asks_then_resumes() {
    let mut f = Fixture::new("real");
    let sink: Arc<dyn ChatSink> = Arc::new(RecordingSink(f.events.clone()));
    let hooks: Arc<dyn ChatHooks> =
        Arc::new(ChatHooksImpl::new(f.db.clone(), Arc::new(NoopNotifier), Arc::new(HostSessionFiles), None, rt()));
    f.chats = ChatProcesses::new(sink, hooks, rt());
    let dir = f.dir.canonicalize().unwrap();
    let spec = ChatSpec { cwd: dir.clone(), args: vec!["--model".into(), "haiku".into()] };
    block_on(async {
        f.chats.send("c1", "p1", None, spec.clone(), "Use the Write tool to create note.txt containing 'x'. Then reply DONE.").unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(r) = f.chats.live("c1").pending.first().cloned() {
                f.chats.respond("c1", &r.request_id, true, None).unwrap();
            }
            if f.events().iter().any(|e| matches!(e, ChatEvent::Stream(StreamEvent::TurnEnd { .. }))) {
                break;
            }
            assert!(Instant::now() < deadline, "no turn end: {:#?}", f.events());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(dir.join("note.txt").exists());
        assert!(f.events().iter().any(|e| matches!(e, ChatEvent::Host(HostEvent::PermissionResolved { allowed: true, .. }))));
        let session = f.events().iter().find_map(|e| match e {
            ChatEvent::Stream(StreamEvent::Init { session_id, .. }) => Some(session_id.clone()),
            _ => None,
        });

        f.chats.stop("c1");
        f.wait_stopped(1).await;
        f.chats.send("c1", "p1", session.as_deref(), spec, "What file did you create? Reply with its name only.").unwrap();
        f.wait_for(2, "second turn", |e| matches!(e, ChatEvent::Stream(StreamEvent::TurnEnd { .. })))
            .await;
        let last = f.events().into_iter().rev().find_map(|e| match e {
            ChatEvent::Stream(StreamEvent::TurnEnd { result, session_id, .. }) => Some((result, session_id)),
            _ => None,
        });
        let (result, sid) = last.unwrap();
        assert_eq!(sid, session);
        assert!(result.unwrap_or_default().contains("note.txt"));

        let projects = nodal_host::claude::fs::paths::claude_config_dir().unwrap().join("projects");
        let session = session.unwrap();
        let jsonl = nodal_host::claude::fs::paths::find_session_jsonl(&projects, &dir.to_string_lossy(), &session).unwrap();
        let text = std::fs::read_to_string(jsonl).unwrap();
        assert!(text.contains(r#""entrypoint":"nodal""#), "the `claude --resume` picker hides SDK entrypoints");
        assert!(!text.contains(r#""entrypoint":"sdk-cli""#));
    });
}

// ---------- ChatHooksImpl, directly ----------
//
// The tests above exercise `ChatHooksImpl` only through a real `ChatProcesses`, with no
// `claude_dir` (so `turn_ended` always takes its early-return branch). These call the hooks
// directly, with a fake `SessionFiles`, to cover the title-refresh path too.

#[derive(Default)]
struct FakeSessionFiles {
    jsonl: Option<std::path::PathBuf>,
    title: Option<String>,
}

impl SessionFiles for FakeSessionFiles {
    fn read_session(&self, _session_id: &str, _cwd: &str) -> SessionReadout {
        unimplemented!()
    }
    fn session_tokens(&self, _session_id: &str, _cwd: &str) -> Option<i64> {
        unimplemented!()
    }
    fn run_detail(&self, _session_id: &str, _cwd: &str) -> Result<Option<RunDetail>, HostError> {
        unimplemented!()
    }
    fn launch_blocker(&self, _session_id: &str, _cwd: &str) -> Result<Option<LaunchBlocker>, HostError> {
        unimplemented!()
    }
    fn agent_transcript(
        &self,
        _session_id: &str,
        _cwd: &str,
        _run_id: &str,
        _agent_id: &str,
        _limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        unimplemented!()
    }
    fn find_session_jsonl(&self, _projects: &Path, _cwd: &str, _session_id: &str) -> Option<std::path::PathBuf> {
        self.jsonl.clone()
    }
    fn read_session_transcript(
        &self,
        _path: &Path,
        _id: &str,
        _label: Option<String>,
        _model: Option<String>,
        _limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        unimplemented!()
    }
    fn session_title(&self, _path: &Path) -> Option<String> {
        self.title.clone()
    }
    fn external_sessions(
        &self,
        _repos: &[(String, std::path::PathBuf)],
        _agents: &[AgentSession],
        _app: &AppRuns,
        _now: i64,
    ) -> Result<ExternalSessions, HostError> {
        unimplemented!()
    }
}

fn seeded_db(session_id: Option<&str>) -> Db {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    rows::insert_project(&c, &project_of("p1", "PAY")).unwrap();
    rows::insert_chat(
        &c,
        &Chat {
            id: "c1".into(),
            project_id: "p1".into(),
            repo_id: None,
            title: None,
            session_title: None,
            session_id: session_id.map(String::from),
            launch: LaunchOptions::default(),
            created_at: 1,
            updated_at: 1,
        },
    )
    .unwrap();
    drop(c);
    db
}

#[test]
fn turn_ended_saves_the_title_session_files_reports_and_notifies() {
    let db = seeded_db(Some(SESSION));
    let sessions: Arc<dyn SessionFiles> = Arc::new(FakeSessionFiles {
        jsonl: Some(std::path::PathBuf::from("/x/s.jsonl")),
        title: Some("Checkout flow".into()),
    });
    let hooks = ChatHooksImpl::new(db.clone(), Arc::new(NoopNotifier), sessions, Some(std::path::PathBuf::from("/claude")), rt());
    hooks.turn_ended("c1", std::path::PathBuf::from("/repo"), "p1".into());

    let deadline = Instant::now() + Duration::from_secs(5);
    while chats::get(&db.lock().unwrap(), "c1").unwrap().session_title.is_none() {
        assert!(Instant::now() < deadline, "title never saved");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(chats::get(&db.lock().unwrap(), "c1").unwrap().session_title.as_deref(), Some("Checkout flow"));
}

#[test]
fn turn_ended_does_nothing_without_a_claude_dir_or_a_saved_session_id() {
    // No `claude_dir`: the real fixture above already covers this (every non-real test uses
    // `claude_dir: None`); this covers the other early return, a chat with no session id yet.
    let db = seeded_db(None);
    let sessions: Arc<dyn SessionFiles> = Arc::new(FakeSessionFiles {
        jsonl: Some(std::path::PathBuf::from("/x/s.jsonl")),
        title: Some("Should not be saved".into()),
    });
    let hooks = ChatHooksImpl::new(db.clone(), Arc::new(NoopNotifier), sessions, Some(std::path::PathBuf::from("/claude")), rt());
    hooks.turn_ended("c1", std::path::PathBuf::from("/repo"), "p1".into());
    block_on(async { tokio::time::sleep(Duration::from_millis(50)).await });
    assert_eq!(chats::get(&db.lock().unwrap(), "c1").unwrap().session_title, None);
}
