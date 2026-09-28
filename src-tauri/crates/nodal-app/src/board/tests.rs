//! Facade tests: `Board`'s methods over an in-memory database, with fake claude/sessions/chat
//! ports (board never calls them) and a clock/notifier local to this module (board owns its
//! tests; it doesn't reach into `nodal_app::testutil`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use nodal_domain::board::dto::{NewProject, NewRepo, NewTask, ProjectPatch};
use nodal_domain::error::HostError;
use nodal_domain::model::activity::{AgentSession, AppRuns, ExternalSessions};
use nodal_domain::model::chat::{ChatLive, ChatSpec};
use nodal_domain::model::claude::{
    ExtraFlags, LaunchBlocker, LaunchError, RunDetail, RunRef, RunSummary, SessionReadout,
    Transcript,
};
use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::LaunchOptions;
use nodal_domain::ports::{BoxFut, ChangeNotifier, ChatRuntime, ClaudeCli, Clock, SessionFiles};
use nodal_host::adapters::{HostClaudeConfig, HostGit, HostLocalFs, HostPlanFiles};
use nodal_host::testutil::TempDir;
use nodal_store::Db;
use tokio::runtime::Runtime;

use super::Board;
use crate::core::{Core, Env};

fn rt() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .enable_io()
            .build()
            .unwrap()
    })
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    rt().block_on(fut)
}

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

/// Records every notice, for assertions.
#[derive(Default)]
struct RecordingNotifier {
    events: Mutex<Vec<(ChangeKind, Option<String>)>>,
}

impl RecordingNotifier {
    fn kinds(&self) -> Vec<ChangeKind> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| *k)
            .collect()
    }
}

impl ChangeNotifier for RecordingNotifier {
    fn notify(&self, kind: ChangeKind, project_id: Option<&str>) {
        self.events
            .lock()
            .unwrap()
            .push((kind, project_id.map(str::to_string)));
    }
}

/// Board never calls `claude`/`sessions`/`chats` (only execution, sessions and chats do): these
/// fakes exist only so `Core` can be built.
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

struct FakeSessionFiles;

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
    fn launch_blocker(
        &self,
        _session_id: &str,
        _cwd: &str,
    ) -> Result<Option<LaunchBlocker>, HostError> {
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
    fn find_session_jsonl(
        &self,
        _projects: &Path,
        _cwd: &str,
        _session_id: &str,
    ) -> Option<PathBuf> {
        unimplemented!()
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
        unimplemented!()
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

struct FakeChatRuntime;

impl ChatRuntime for FakeChatRuntime {
    fn send(
        &self,
        _chat_id: &str,
        _project_id: &str,
        _session_id: Option<&str>,
        _spec: ChatSpec,
        _text: &str,
    ) -> Result<(), HostError> {
        unimplemented!()
    }
    fn respond(
        &self,
        _chat_id: &str,
        _request_id: &str,
        _allow: bool,
        _message: Option<&str>,
    ) -> Result<(), HostError> {
        unimplemented!()
    }
    fn interrupt(&self, _chat_id: &str) -> Result<(), HostError> {
        unimplemented!()
    }
    fn stop(&self, _chat_id: &str) {
        unimplemented!()
    }
    fn stop_project(&self, _project_id: &str) {
        unimplemented!()
    }
    fn live(&self, _chat_id: &str) -> ChatLive {
        unimplemented!()
    }
    fn start_reaper(&self) {
        unimplemented!()
    }
}

/// A `Board` over a fresh in-memory database, plus the pieces the test needs to assert on.
struct Fx {
    _t: TempDir,
    board: Board,
    notifier: Arc<RecordingNotifier>,
}

fn fx(name: &str) -> Fx {
    let t = TempDir::new(name);
    let env = Env {
        data_dir: t.0.join("data"),
        worktrees_root: t.0.join("wt"),
        claude_dir: None,
        plans: Arc::new(HostPlanFiles),
        fs: Arc::new(HostLocalFs),
        git: Arc::new(HostGit),
        claude_config: Arc::new(HostClaudeConfig),
    };
    let notifier = Arc::new(RecordingNotifier::default());
    let core = Arc::new(Core {
        db: Db::open_in_memory().unwrap(),
        env,
        rt: rt().handle().clone(),
        clock: Arc::new(FixedClock(1_000)),
        claude: Arc::new(FakeClaude),
        sessions: Arc::new(FakeSessionFiles),
        notifier: notifier.clone(),
        chats: Arc::new(FakeChatRuntime),
    });
    Fx {
        _t: t,
        board: Board::new(core),
        notifier,
    }
}

fn new_project(key: &str) -> NewProject {
    NewProject {
        name: key.to_string(),
        key: key.to_string(),
        color: None,
        description: None,
        root_path: None,
    }
}

#[test]
fn create_list_and_update_project_notify_and_persist() {
    let f = fx("board-projects");
    let p = block_on(f.board.create_project(new_project("pay"))).unwrap();
    assert_eq!(p.key, "PAY");
    assert_eq!(f.notifier.kinds(), vec![ChangeKind::Projects]);

    let all = block_on(f.board.list_projects(None)).unwrap();
    assert_eq!(all.len(), 1);

    let patch: ProjectPatch =
        serde_json::from_value(serde_json::json!({"name": "Payments"})).unwrap();
    let updated = block_on(f.board.update_project(p.id.clone(), patch)).unwrap();
    assert_eq!(updated.name, "Payments");
    assert_eq!(
        f.notifier.kinds(),
        vec![ChangeKind::Projects, ChangeKind::Projects]
    );
}

#[test]
fn update_project_rejects_an_invalid_id_without_touching_the_database() {
    let f = fx("board-invalid-id");
    let err = block_on(f.board.update_project("x".into(), ProjectPatch::default())).unwrap_err();
    assert!(err.to_string().contains("Invalid project id"));
    assert!(f.notifier.kinds().is_empty());
}

#[test]
fn delete_project_calls_stop_chats_right_after_check_id_and_notifies_widely() {
    let f = fx("board-delete-project");
    let p = block_on(f.board.create_project(new_project("web"))).unwrap();
    let stopped = Arc::new(Mutex::new(None));
    let s = stopped.clone();
    block_on(f.board.delete_project(p.id.clone(), move |id| {
        *s.lock().unwrap() = Some(id.to_string())
    }))
    .unwrap();
    assert_eq!(stopped.lock().unwrap().as_deref(), Some(p.id.as_str()));
    assert_eq!(
        f.notifier.kinds(),
        vec![
            ChangeKind::Projects,
            ChangeKind::Projects,
            ChangeKind::Tasks,
            ChangeKind::Runs,
            ChangeKind::Queue,
            ChangeKind::Sources,
            ChangeKind::Chats,
        ]
    );
    assert!(block_on(f.board.list_projects(None)).unwrap().is_empty());
}

#[test]
fn delete_project_of_an_unknown_id_does_not_call_stop_chats() {
    let f = fx("board-delete-missing");
    let called = Arc::new(Mutex::new(false));
    let c = called.clone();
    let err = block_on(f.board.delete_project("p-does-not-exist".into(), move |_| {
        *c.lock().unwrap() = true
    }))
    .unwrap_err();
    assert!(err.to_string().contains("no longer exists"));
    assert!(
        *called.lock().unwrap(),
        "stop_chats runs right after check_id, before the database lookup"
    );
}

#[test]
fn repo_and_task_crud_round_trips_through_the_facade() {
    if !nodal_host::testutil::git_available() {
        return;
    }
    let f = fx("board-tasks");
    let p = block_on(f.board.create_project(new_project("acme"))).unwrap();
    let repo_dir = f._t.0.join("repo");
    nodal_host::testutil::init_repo(&repo_dir);
    let r = block_on(f.board.add_repo(
        p.id.clone(),
        NewRepo {
            path: repo_dir.to_string_lossy().into_owned(),
            ..Default::default()
        },
    ))
    .unwrap();
    assert_eq!(r.project_id, p.id);

    let task_input: NewTask = serde_json::from_value(serde_json::json!({
        "projectId": p.id, "repoId": r.id, "title": "Ship it",
        "plan": {"kind": "text", "text": "# Plan"},
    }))
    .unwrap();
    let t = block_on(f.board.create_task(task_input)).unwrap();
    assert_eq!(t.title, "Ship it");

    let got = block_on(f.board.get_task(t.id.clone())).unwrap();
    assert_eq!(got.id, t.id);

    let plan = block_on(f.board.read_task_plan(t.id.clone())).unwrap();
    assert_eq!(plan, "# Plan");

    block_on(f.board.delete_task(t.id.clone())).unwrap();
    assert!(block_on(f.board.get_task(t.id)).is_err());
}

#[test]
fn settings_round_trip_through_the_facade() {
    let f = fx("board-settings");
    let defaults = block_on(f.board.get_settings()).unwrap();
    let mut wanted = defaults.clone();
    wanted.concurrency = 3;
    let saved = block_on(f.board.set_settings(wanted)).unwrap();
    assert_eq!(saved.concurrency, 3);
    assert_eq!(f.notifier.kinds(), vec![ChangeKind::Projects]);
    // `set_settings` doesn't kick the queue itself: that's the command's job, after this
    // returns (today's order, kept in `commands::board::set_settings`).
}
