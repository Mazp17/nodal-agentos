use std::path::{Path, PathBuf};

use nodal_domain::testutil::{project_of, repo_of};
use nodal_store::rows::{insert_project, insert_repo};
use nodal_store::Db;

use super::*;

fn seed(c: &Connection) {
    insert_project(c, &project_of("p1", "PAY")).unwrap();
    insert_project(c, &project_of("p2", "WEB")).unwrap();
    insert_repo(c, &repo_of("r1", "p1", "/Users/me/Code/acme-api")).unwrap();
    let mut r2 = repo_of("r2", "p1", "/Users/me/Code/acme-web");
    r2.position = 1;
    insert_repo(c, &r2).unwrap();
    insert_repo(c, &repo_of("r3", "p2", "/Users/me/Code/acme-docs")).unwrap();
}

fn patch(json: &str) -> ChatPatch {
    serde_json::from_str(json).unwrap()
}

#[test]
fn create_validates_project_repo_and_options() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    let chat = ops::create(&c, "p1", &NewChat::default(), 7).unwrap();
    assert!(chat.id.starts_with('c'));
    assert_eq!(
        (
            chat.repo_id.as_deref(),
            chat.title.as_deref(),
            chat.session_id.as_deref()
        ),
        (None, None, None)
    );
    assert_eq!((chat.created_at, chat.updated_at), (7, 7));

    let input: NewChat =
        serde_json::from_str(r#"{"repoId":"r2","title":"  Plan\n the  login ","model":" sonnet ","permissionMode":"plan"}"#)
            .unwrap();
    let chat = ops::create(&c, "p1", &input, 8).unwrap();
    assert_eq!(
        (chat.repo_id.as_deref(), chat.title.as_deref()),
        (Some("r2"), Some("Plan the login"))
    );
    assert_eq!(
        (
            chat.launch.model.as_deref(),
            chat.launch.permission_mode.as_deref()
        ),
        (Some("sonnet"), Some("plan"))
    );

    let other = NewChat {
        repo_id: Some("r3".into()),
        ..NewChat::default()
    };
    assert_eq!(
        ops::create(&c, "p1", &other, 9).unwrap_err(),
        "That repo belongs to another project."
    );
    assert!(ops::create(&c, "missing", &NewChat::default(), 9).is_err());
    let bad = NewChat {
        launch: LaunchOptions {
            effort: Some("ultra".into()),
            ..Default::default()
        },
        ..NewChat::default()
    };
    assert!(ops::create(&c, "p1", &bad, 9)
        .unwrap_err()
        .contains("Invalid effort"));
}

#[test]
fn update_patches_only_what_it_gets() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    let input = NewChat {
        repo_id: Some("r1".into()),
        launch: LaunchOptions {
            model: Some("opus".into()),
            ..Default::default()
        },
        ..NewChat::default()
    };
    let chat = ops::create(&c, "p1", &input, 1).unwrap();

    let u = ops::update(
        &c,
        &chat.id,
        &patch(r#"{"title":"Billing","effort":"high"}"#),
    )
    .unwrap();
    assert_eq!(
        (u.title.as_deref(), u.repo_id.as_deref()),
        (Some("Billing"), Some("r1"))
    );
    assert_eq!(
        (u.launch.model.as_deref(), u.launch.effort.as_deref()),
        (Some("opus"), Some("high"))
    );

    let u = ops::update(&c, &chat.id, &patch(r#"{"repoId":null,"model":null}"#)).unwrap();
    assert_eq!(
        (u.repo_id.as_deref(), u.launch.model.as_deref()),
        (None, None)
    );
    assert_eq!(u.launch.effort.as_deref(), Some("high"));

    assert!(ops::update(&c, &chat.id, &patch(r#"{"repoId":"r3"}"#)).is_err());
    assert!(ops::update(&c, &chat.id, &patch(r#"{"permissionMode":"yolo"}"#)).is_err());
    assert!(
        serde_json::from_str::<ChatPatch>(r#"{"sessionId":"x"}"#).is_err(),
        "the session isn't patchable"
    );
    assert_eq!(chats::get(&c, &chat.id).unwrap(), u);
}

#[test]
fn prepare_send_names_and_touches_the_chat() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    let chat = ops::create(&c, "p1", &NewChat::default(), 1).unwrap();
    assert_eq!(
        ops::prepare_send(&c, &chat.id, "  \n ", 2).unwrap_err(),
        "The message is empty."
    );
    assert!(
        ops::prepare_send(&c, &chat.id, &"x".repeat(MESSAGE_MAX + 1), 2)
            .unwrap_err()
            .contains("too long")
    );

    let long = format!("Turn the checkout flow\ninto tasks {}", "a".repeat(200));
    let (sent, project, repos) = ops::prepare_send(&c, &chat.id, &long, 5).unwrap();
    assert_eq!(sent.updated_at, 5);
    let t = sent.title.clone().unwrap();
    assert!(
        t.starts_with("Turn the checkout flow into tasks") && t.chars().count() == TITLE_MAX,
        "{t}"
    );
    assert_eq!(project.id, "p1");
    assert_eq!(
        repos.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["r1", "r2"]
    );

    let (again, _, _) = ops::prepare_send(&c, &chat.id, "Second message", 6).unwrap();
    assert_eq!(again.title, sent.title, "the title is kept");
    assert_eq!(chats::get(&c, &chat.id).unwrap().updated_at, 6);
}

#[test]
fn spec_runs_in_the_chat_repo_or_the_first_one() {
    let project = project_of("p1", "PAY");
    let mut second = repo_of("r2", "p1", "/Users/me/Code/acme-web");
    second.launch.permission_mode = Some("bypassPermissions".into());
    let repos = vec![repo_of("r1", "p1", "/Users/me/Code/acme-api"), second];
    let mut chat = Chat {
        id: "c1".into(),
        project_id: "p1".into(),
        repo_id: None,
        title: None,
        session_title: None,
        session_id: None,
        launch: LaunchOptions {
            model: Some("haiku".into()),
            effort: Some("low".into()),
            permission_mode: None,
        },
        created_at: 1,
        updated_at: 1,
    };
    let s = spec(&chat, &project, &repos, None).unwrap();
    assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme-api"));
    assert_eq!(
        s.args[..6],
        [
            "--model",
            "haiku",
            "--effort",
            "low",
            "--add-dir",
            "/Users/me/Code/acme-web"
        ]
    );
    assert_eq!(s.args[6], "--append-system-prompt");

    chat.repo_id = Some("r2".into());
    let s = spec(
        &chat,
        &project,
        &repos,
        Some(Path::new("/Users/me/nodal-mcp")),
    )
    .unwrap();
    assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme-web"));
    assert!(s.args.iter().any(|a| a == "--mcp-config") && !s.args.iter().any(|a| a == "--add-dir"));
    assert!(
        !s.args.iter().any(|a| a == "--permission-mode"),
        "the repo's permission mode isn't inherited"
    );

    chat.repo_id = Some("gone".into());
    assert!(spec(&chat, &project, &repos, None).is_err());
    chat.repo_id = None;
    assert!(spec(&chat, &project, &[], None)
        .unwrap_err()
        .contains("Add a repo"));
}

#[test]
fn a_repo_less_chat_runs_at_the_project_root_when_it_has_one() {
    let mut project = project_of("p1", "PAY");
    project.root_path = Some("/Users/me/Code/acme".into());
    let repos = vec![
        repo_of("r1", "p1", "/Users/me/Code/acme/api"),
        repo_of("r2", "p1", "/Users/me/Code/acme/web"),
        repo_of("r3", "p1", "/Users/me/Code/acme-docs"),
    ];
    let mut chat = Chat {
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
    let s = spec(&chat, &project, &repos, None).unwrap();
    assert_eq!(s.cwd, PathBuf::from("/Users/me/Code/acme"));
    assert_eq!(
        s.args[..2],
        ["--add-dir", "/Users/me/Code/acme-docs"],
        "only the repo outside the root"
    );
    assert!(s
        .args
        .last()
        .unwrap()
        .contains("This chat runs at the project root."));
    assert_eq!(
        s.cwd,
        PathBuf::from(chat_cwd(&chat, &project, &repos).unwrap().path()),
        "the transcript looks it up here"
    );

    let s = spec(&chat, &project, &[], None).unwrap();
    assert_eq!(
        s.cwd,
        PathBuf::from("/Users/me/Code/acme"),
        "a root is enough to chat"
    );

    chat.repo_id = Some("r2".into());
    let s = spec(&chat, &project, &repos, None).unwrap();
    assert_eq!(
        s.cwd,
        PathBuf::from("/Users/me/Code/acme/web"),
        "a repo chat ignores the root"
    );
    assert!(!s.args.iter().any(|a| a == "--add-dir"));

    chat.repo_id = None;
    project.root_path = None;
    assert_eq!(
        chat_cwd(&chat, &project, &repos).unwrap(),
        ChatCwd::Repo(&repos[0])
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // builds its input from a real session file, like the transcript command does
fn first_message_goes_back_into_the_items() {
    use nodal_domain::model::claude::TranscriptItem;

    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"Split the checkout into tasks"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Here is a plan."}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":"Thanks"}}"#,
    ];
    let path = std::env::temp_dir().join(format!(
        "nodal-chat-transcript-{}.jsonl",
        std::process::id()
    ));
    std::fs::write(&path, lines.join("\n")).unwrap();
    let t = nodal_host::claude::fs::transcript::read_session_transcript(&path, "c1", None, None, 10)
        .unwrap()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    let t = transcript::with_first_message(t);
    let texts: Vec<_> = t
        .items
        .iter()
        .map(|i| match i {
            TranscriptItem::User { text, .. } => format!("user: {text}"),
            TranscriptItem::Text { text, .. } => format!("claude: {text}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        texts,
        [
            "user: Split the checkout into tasks",
            "claude: Here is a plan.",
            "user: Thanks"
        ]
    );
    assert_eq!((t.prompt.as_deref(), t.total_items), (None, 3));

    let cut = Transcript {
        omitted: 2,
        prompt: Some("first".into()),
        ..t.clone()
    };
    assert_eq!(
        transcript::with_first_message(cut).prompt.as_deref(),
        Some("first"),
        "not next to a gap"
    );
}

// ---------- Chats facade ----------
//
// Facade tests: `Chats`'s methods over an in-memory database, with a recording fake
// `ChatRuntime` (the real one is `nodal_host::claude::chats::ChatProcesses`, exercised
// end-to-end in `hooks::tests`) and a fake `SessionFiles` for the transcript path.

use std::sync::{Arc, Mutex};

use nodal_domain::error::HostError;
use nodal_domain::model::activity::{AgentSession, AppRuns, ExternalSessions};
use nodal_domain::model::chat::RunState;
use nodal_domain::model::claude::{ExtraFlags, LaunchBlocker, LaunchError, RunDetail, RunRef, RunSummary, SessionReadout};
use nodal_domain::model::events::ChangeKind;
use nodal_domain::ports::{BoxFut, ChangeNotifier, ChatRuntime, ClaudeCli, Clock, SessionFiles};
use nodal_host::testutil::TempDir;

use crate::core::Core;
use crate::testutil::{block_on, env_for};

/// Chats never calls `claude` directly (only `core.sessions`/`core.chats`): this fake exists
/// only so `Core` can be built.
struct FakeClaude;

impl ClaudeCli for FakeClaude {
    fn list_sessions(&self) -> BoxFut<'_, Result<Vec<RunSummary>, HostError>> {
        unimplemented!()
    }
    fn list_agent_sessions(&self) -> BoxFut<'_, Result<Vec<AgentSession>, HostError>> {
        unimplemented!()
    }
    fn launch_bg<'a>(
        &'a self,
        _cwd: String,
        _prompt: String,
        _opts: &'a LaunchOptions,
        _extra: &'a ExtraFlags,
    ) -> BoxFut<'a, Result<RunRef, LaunchError>> {
        unimplemented!()
    }
    fn stop<'a>(&'a self, _short_id: &'a str) -> BoxFut<'a, Result<(), HostError>> {
        unimplemented!()
    }
}

#[derive(Default)]
struct FakeSessionFiles {
    jsonl: Option<PathBuf>,
    transcript: Option<Transcript>,
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
    fn find_session_jsonl(&self, _projects: &Path, _cwd: &str, _session_id: &str) -> Option<PathBuf> {
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
        Ok(self.transcript.clone())
    }
    fn session_title(&self, _path: &Path) -> Option<String> {
        self.title.clone()
    }
    fn external_sessions(
        &self,
        _repos: &[(String, PathBuf)],
        _agents: &[AgentSession],
        _app: &AppRuns,
        _now: i64,
    ) -> Result<ExternalSessions, HostError> {
        unimplemented!()
    }
}

/// One recorded call to `ChatRuntime::send`: `(chat_id, project_id, session_id, spec, text)`.
type SentCall = (String, String, Option<String>, ChatSpec, String);

/// Records every call, so the facade's wiring (spec building, order of operations) can be
/// asserted on without a real `claude` process.
#[derive(Default)]
struct FakeChatRuntime {
    sent: Mutex<Vec<SentCall>>,
    stopped: Mutex<Vec<String>>,
    stopped_projects: Mutex<Vec<String>>,
}

impl ChatRuntime for FakeChatRuntime {
    fn send(&self, chat_id: &str, project_id: &str, session_id: Option<&str>, spec: ChatSpec, text: &str) -> Result<(), HostError> {
        self.sent
            .lock()
            .unwrap()
            .push((chat_id.into(), project_id.into(), session_id.map(String::from), spec, text.into()));
        Ok(())
    }
    fn respond(&self, _chat_id: &str, request_id: &str, _allow: bool, _message: Option<&str>) -> Result<(), HostError> {
        if request_id == "bad" {
            return Err("that permission request is gone".into());
        }
        Ok(())
    }
    fn interrupt(&self, _chat_id: &str) -> Result<(), HostError> {
        Ok(())
    }
    fn stop(&self, chat_id: &str) {
        self.stopped.lock().unwrap().push(chat_id.into());
    }
    fn stop_project(&self, project_id: &str) {
        self.stopped_projects.lock().unwrap().push(project_id.into());
    }
    fn live(&self, chat_id: &str) -> ChatLive {
        let state = if self.stopped.lock().unwrap().iter().any(|c| c == chat_id) { RunState::Stopped } else { RunState::Idle };
        ChatLive { state, pending: vec![] }
    }
    fn start_reaper(&self) {}
}

/// Records every notice, for assertions.
#[derive(Default)]
struct RecordingNotifier {
    events: Mutex<Vec<(ChangeKind, Option<String>)>>,
}

impl RecordingNotifier {
    fn kinds(&self) -> Vec<ChangeKind> {
        self.events.lock().unwrap().iter().map(|(k, _)| *k).collect()
    }
}

impl ChangeNotifier for RecordingNotifier {
    fn notify(&self, kind: ChangeKind, project_id: Option<&str>) {
        self.events.lock().unwrap().push((kind, project_id.map(str::to_string)));
    }
}

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

/// A `Chats` over a fresh in-memory database, with one project ("p1") and one repo ("r1").
struct Fx {
    _t: TempDir,
    db: Db,
    chats: Chats,
    runtime: Arc<FakeChatRuntime>,
    notifier: Arc<RecordingNotifier>,
    repo_path: PathBuf,
}

fn fx(name: &str) -> Fx {
    fx_with(name, Arc::new(FakeSessionFiles::default()))
}

fn fx_with(name: &str, sessions: Arc<dyn SessionFiles>) -> Fx {
    let t = TempDir::new(name);
    // Not created on disk: `FakeChatRuntime` doesn't check it exists (unlike the real
    // `ChatProcesses`, exercised with a real cwd in `hooks::tests`).
    let repo_path = t.0.join("repo");
    let db = Db::open_in_memory().unwrap();
    let runtime = Arc::new(FakeChatRuntime::default());
    let notifier = Arc::new(RecordingNotifier::default());
    let core = Arc::new(Core {
        db: db.clone(),
        env: env_for(&t.0, Some(t.0.join("claude"))),
        rt: crate::testutil::rt(),
        clock: Arc::new(FixedClock(1_000)),
        claude: Arc::new(FakeClaude),
        sessions,
        notifier: notifier.clone(),
        chats: runtime.clone(),
    });
    {
        let guard = db.lock().unwrap();
        insert_project(&guard, &project_of("p1", "PAY")).unwrap();
        insert_repo(&guard, &repo_of("r1", "p1", &repo_path.to_string_lossy())).unwrap();
    }
    let chats = Chats::new(core);
    Fx { _t: t, db, chats, runtime, notifier, repo_path }
}

#[test]
fn create_list_update_and_delete_round_trip() {
    let f = fx("chats-crud");
    let chat = block_on(f.chats.create_chat("p1".into(), NewChat::default())).unwrap();
    assert_eq!(chat.project_id, "p1");
    assert!(chat.title.is_none());
    assert_eq!(f.notifier.kinds(), vec![ChangeKind::Chats]);

    let all = block_on(f.chats.list_chats(Some("p1".into()))).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(block_on(f.chats.list_chats(None)).unwrap().len(), 1);

    let patch: ChatPatch = serde_json::from_value(serde_json::json!({"title": "Renamed"})).unwrap();
    let updated = block_on(f.chats.update_chat(chat.id.clone(), patch)).unwrap();
    assert_eq!(updated.title.as_deref(), Some("Renamed"));

    block_on(f.chats.delete_chat(chat.id.clone())).unwrap();
    assert!(block_on(f.chats.list_chats(None)).unwrap().is_empty());
    assert_eq!(f.runtime.stopped.lock().unwrap().as_slice(), [chat.id]);
    assert_eq!(f.notifier.kinds(), vec![ChangeKind::Chats, ChangeKind::Chats, ChangeKind::Chats]);
}

#[test]
fn create_chat_rejects_an_invalid_project_id() {
    let f = fx("chats-bad-project");
    let err = block_on(f.chats.create_chat("../nope".into(), NewChat::default())).unwrap_err();
    assert!(err.to_string().contains("Invalid project id"));
}

#[test]
fn send_chat_message_rejects_an_empty_or_too_long_message() {
    let f = fx("chats-message-bounds");
    let chat = block_on(f.chats.create_chat("p1".into(), NewChat::default())).unwrap();
    let err = block_on(f.chats.send_chat_message(chat.id.clone(), "   ".into())).unwrap_err();
    assert_eq!(err.to_string(), "The message is empty.");

    let too_long = "x".repeat(super::MESSAGE_MAX + 1);
    let err = block_on(f.chats.send_chat_message(chat.id, too_long)).unwrap_err();
    assert!(err.to_string().contains("too long"));
}

#[test]
fn send_chat_message_builds_the_spec_and_names_the_chat() {
    let f = fx("chats-send");
    let chat = block_on(f.chats.create_chat("p1".into(), NewChat::default())).unwrap();
    let sent = block_on(f.chats.send_chat_message(chat.id.clone(), "Turn the checkout flow into tasks please".into())).unwrap();
    assert!(sent.title.as_deref().unwrap().starts_with("Turn the checkout flow"));
    assert_eq!(f.notifier.kinds(), vec![ChangeKind::Chats, ChangeKind::Chats]);

    let calls = f.runtime.sent.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let (cid, pid, sid, spec, text) = &calls[0];
    assert_eq!(cid, &chat.id);
    assert_eq!(pid, "p1");
    assert_eq!(sid, &None);
    assert_eq!(spec.cwd, f.repo_path);
    assert_eq!(text, "Turn the checkout flow into tasks please");
}

#[test]
fn respond_interrupt_stop_and_live_delegate_to_the_runtime() {
    let f = fx("chats-runtime-passthrough");
    let chat = block_on(f.chats.create_chat("p1".into(), NewChat::default())).unwrap();

    block_on(f.chats.respond_chat_permission(chat.id.clone(), "req-1".into(), true, None)).unwrap();
    let err = block_on(f.chats.respond_chat_permission(chat.id.clone(), "bad".into(), true, None)).unwrap_err();
    assert!(err.to_string().contains("gone"));

    block_on(f.chats.interrupt_chat(chat.id.clone())).unwrap();
    assert_eq!(block_on(f.chats.get_chat_live(chat.id.clone())).unwrap().state, RunState::Idle);
    block_on(f.chats.stop_chat(chat.id.clone())).unwrap();
    assert_eq!(block_on(f.chats.get_chat_live(chat.id)).unwrap().state, RunState::Stopped);

    let err = block_on(f.chats.stop_chat("../nope".into())).unwrap_err();
    assert!(err.to_string().contains("Invalid chat id"));
}

#[test]
fn get_claude_defaults_rejects_an_invalid_repo_id() {
    let f = fx("chats-defaults-bad-repo");
    let err = block_on(f.chats.get_claude_defaults(Some("../nope".into()))).unwrap_err();
    assert!(err.to_string().contains("Invalid repo id"));
}

#[test]
fn get_claude_defaults_without_a_repo_reads_only_the_user_settings() {
    let f = fx("chats-defaults");
    let defaults = block_on(f.chats.get_claude_defaults(None)).unwrap();
    assert_eq!(defaults.model, None);
    assert_eq!(defaults.effort, None);
}

#[test]
fn get_chat_transcript_rejects_an_invalid_chat_id() {
    let f = fx("chats-transcript-bad-id");
    let err = block_on(f.chats.get_chat_transcript("../nope".into(), None)).unwrap_err();
    assert!(err.to_string().contains("Invalid chat id"));
}

#[test]
fn get_chat_transcript_returns_none_without_a_session_id() {
    let f = fx("chats-transcript-no-session");
    let chat = block_on(f.chats.create_chat("p1".into(), NewChat::default())).unwrap();
    assert_eq!(block_on(f.chats.get_chat_transcript(chat.id, None)).unwrap(), None);
}

#[test]
fn get_chat_transcript_reads_through_the_session_files_port_and_refreshes_the_title() {
    let t = TempDir::new("chats-transcript-happy");
    let jsonl = t.0.join("s.jsonl");
    let transcript = Transcript {
        agent_id: "a1".into(),
        label: None,
        model: None,
        phase: None,
        prompt: None,
        items: vec![],
        total_items: 0,
        omitted: 0,
        tool_calls: 0,
        final_output: None,
        partial: false,
        bytes: 0,
    };
    let files = Arc::new(FakeSessionFiles {
        jsonl: Some(jsonl),
        transcript: Some(transcript.clone()),
        title: Some("Checkout flow".into()),
    });
    let f = fx_with("chats-transcript-happy", files);
    let chat = block_on(f.chats.create_chat("p1".into(), NewChat::default())).unwrap();
    // The session id normally arrives through `ChatHooksImpl::session_started`, once the real
    // `claude` process reports it: set directly here, since `FakeChatRuntime` doesn't run one.
    assert!(nodal_store::chats::set_session(&f.db.lock().unwrap(), &chat.id, "sess-1").unwrap());

    let got = block_on(f.chats.get_chat_transcript(chat.id.clone(), None)).unwrap();
    assert_eq!(got, Some(transcript));

    // The title Claude Code gave the session is new: it's saved and notified.
    let all = block_on(f.chats.list_chats(Some("p1".into()))).unwrap();
    assert_eq!(all[0].session_title.as_deref(), Some("Checkout flow"));
    assert!(f.notifier.kinds().contains(&ChangeKind::Chats));
}

#[test]
fn stop_project_delegates_to_the_runtime() {
    let f = fx("chats-stop-project");
    f.chats.stop_project("p1");
    assert_eq!(f.runtime.stopped_projects.lock().unwrap().as_slice(), ["p1".to_string()]);
}
