#![allow(clippy::disallowed_methods)] // real host adapters and fixture folders, like other adapter tests

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use nodal_app::sources::keys::Secrets;
use nodal_app::sources::registry::ProviderRegistry;
use nodal_app::sources::SourcesHub;
use nodal_app::{App, Deps, Env, HubDeps};
use nodal_domain::error::HostError;
use nodal_domain::model::activity::{AgentSession, AppRuns, ExternalSessions};
use nodal_domain::model::chat::{ChatLive, ChatSpec};
use nodal_domain::model::claude::{
    ExtraFlags, LaunchBlocker, LaunchError, RunDetail, RunRef, RunSummary, SessionReadout,
    Transcript,
};
use nodal_domain::model::LaunchOptions;
use nodal_domain::ports::{
    BoxFut, ChangeNotifier, ChatRuntime, ClaudeCli, Clock, SecretStore, SessionFiles,
};
use nodal_host::adapters::{HostClaudeConfig, HostGit, HostLocalFs, HostPlanFiles, SystemClock};
use nodal_host::testutil::TempDir;
use nodal_store::board::projects;
use nodal_store::Db;

use crate::mcp::stdio::{forward, OPEN_NODAL};
use nodal_app::board::ops;
use nodal_domain::board::dto::{NewProject, NewRepo};

use super::*;

/// Not exercised: `agent_api::tools` never reaches `Core::claude`.
struct NoopClaude;

impl ClaudeCli for NoopClaude {
    fn list_sessions(&self) -> BoxFut<'_, Result<Vec<RunSummary>, HostError>> {
        Box::pin(async move { unimplemented!("not exercised by the socket tests") })
    }
    fn list_agent_sessions(&self) -> BoxFut<'_, Result<Vec<AgentSession>, HostError>> {
        Box::pin(async move { unimplemented!("not exercised by the socket tests") })
    }
    fn launch_bg<'a>(
        &'a self,
        _cwd: String,
        _prompt: String,
        _opts: &'a LaunchOptions,
        _extra: &'a ExtraFlags,
    ) -> BoxFut<'a, Result<RunRef, LaunchError>> {
        Box::pin(async move { unimplemented!("not exercised by the socket tests") })
    }
    fn stop<'a>(&'a self, _short_id: &'a str) -> BoxFut<'a, Result<(), HostError>> {
        Box::pin(async move { unimplemented!("not exercised by the socket tests") })
    }
}

/// Not exercised: `agent_api::tools` never reaches `Core::sessions`.
struct NoopSessionFiles;

impl SessionFiles for NoopSessionFiles {
    fn read_session(&self, _session_id: &str, _cwd: &str) -> SessionReadout {
        unimplemented!("not exercised by the socket tests")
    }
    fn session_tokens(&self, _session_id: &str, _cwd: &str) -> Option<i64> {
        unimplemented!("not exercised by the socket tests")
    }
    fn run_detail(&self, _session_id: &str, _cwd: &str) -> Result<Option<RunDetail>, HostError> {
        unimplemented!("not exercised by the socket tests")
    }
    fn launch_blocker(
        &self,
        _session_id: &str,
        _cwd: &str,
    ) -> Result<Option<LaunchBlocker>, HostError> {
        unimplemented!("not exercised by the socket tests")
    }
    fn agent_transcript(
        &self,
        _session_id: &str,
        _cwd: &str,
        _run_id: &str,
        _agent_id: &str,
        _limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        unimplemented!("not exercised by the socket tests")
    }
    fn find_session_jsonl(
        &self,
        _projects: &Path,
        _cwd: &str,
        _session_id: &str,
    ) -> Option<PathBuf> {
        unimplemented!("not exercised by the socket tests")
    }
    fn read_session_transcript(
        &self,
        _path: &Path,
        _id: &str,
        _label: Option<String>,
        _model: Option<String>,
        _limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        unimplemented!("not exercised by the socket tests")
    }
    fn session_title(&self, _path: &Path) -> Option<String> {
        unimplemented!("not exercised by the socket tests")
    }
    fn external_sessions(
        &self,
        _repos: &[(String, PathBuf)],
        _agents: &[AgentSession],
        _app: &AppRuns,
        _now: i64,
    ) -> Result<ExternalSessions, HostError> {
        unimplemented!("not exercised by the socket tests")
    }
}

/// Not exercised: `agent_api::tools` never reaches `Core::chats`.
struct NoopChatRuntime;

impl ChatRuntime for NoopChatRuntime {
    fn send(
        &self,
        _chat_id: &str,
        _project_id: &str,
        _session_id: Option<&str>,
        _spec: ChatSpec,
        _text: &str,
    ) -> Result<(), HostError> {
        unimplemented!("not exercised by the socket tests")
    }
    fn respond(
        &self,
        _chat_id: &str,
        _request_id: &str,
        _allow: bool,
        _message: Option<&str>,
    ) -> Result<(), HostError> {
        unimplemented!("not exercised by the socket tests")
    }
    fn interrupt(&self, _chat_id: &str) -> Result<(), HostError> {
        unimplemented!("not exercised by the socket tests")
    }
    fn stop(&self, _chat_id: &str) {}
    fn stop_project(&self, _project_id: &str) {}
    fn live(&self, _chat_id: &str) -> ChatLive {
        unimplemented!("not exercised by the socket tests")
    }
    fn start_reaper(&self) {}
}

/// Not exercised: the sources hub built below is never reached by these tests either.
struct NoopSecretStore;

impl SecretStore for NoopSecretStore {
    fn read(&self, _account: &str) -> Result<Option<String>, HostError> {
        unimplemented!("not exercised by the socket tests")
    }
    fn write(&self, _account: &str, _value: &str) -> Result<(), HostError> {
        unimplemented!("not exercised by the socket tests")
    }
    fn delete(&self, _account: &str) -> Result<(), HostError> {
        unimplemented!("not exercised by the socket tests")
    }
}

/// Short names: a socket path is limited to 104 bytes on macOS. Real (blocking, synchronous)
/// host adapters, an in-memory database and a `SourcesHub` that is built but never reached
/// (`agent_api::tools` doesn't touch it): this only exercises the socket and `AgentApi::call`.
fn test_app(root: &Path) -> (Arc<App>, Db, Env) {
    let data_dir = root.join("d");
    let env = Env {
        data_dir: data_dir.clone(),
        worktrees_root: root.join("w"),
        claude_dir: None,
        plans: Arc::new(HostPlanFiles),
        fs: Arc::new(HostLocalFs),
        git: Arc::new(HostGit),
        claude_config: Arc::new(HostClaudeConfig),
    };
    let db = Db::open_in_memory().unwrap();
    let rt = tauri::async_runtime::handle().inner().clone();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let notifier: Arc<dyn ChangeNotifier> = Arc::new(crate::adapters::events::Events::default());
    let sources = SourcesHub::new(HubDeps {
        db: Some(db.clone()),
        data_dir,
        rt: rt.clone(),
        clock: clock.clone(),
        secrets: Secrets::new(Arc::new(NoopSecretStore)),
        providers: ProviderRegistry::new(vec![]),
        notifier: notifier.clone(),
        plans: Arc::new(HostPlanFiles),
    });
    let app = App::new(Deps {
        db: db.clone(),
        env: env.clone(),
        rt,
        clock,
        claude: Arc::new(NoopClaude),
        sessions: Arc::new(NoopSessionFiles),
        notifier,
        chats: Arc::new(NoopChatRuntime),
        sources,
    });
    (app, db, env)
}

#[test]
fn socket_is_private_and_serves_the_same_ops_as_the_ui() {
    let t = crate::util::paths::tests::TempDir::new("mcps");
    let (app, db, env) = test_app(&t.0);
    let repo_dir = t.0.join("web");
    fs::create_dir_all(&repo_dir).unwrap();
    {
        let c = db.lock().unwrap();
        let p = ops::create_project(
            &c,
            &env,
            &NewProject {
                name: "Pay".into(),
                key: "PAY".into(),
                color: None,
                description: None,
                root_path: None,
            },
            1,
        )
        .unwrap();
        ops::add_repo(&c, &p.id, &NewRepo::default(), &repo_dir, 2).unwrap();
    }
    let socket = mcp_socket(&env.data_dir);
    assert_eq!(
        forward(&socket, "list_projects", json!({})).unwrap_err(),
        OPEN_NODAL
    );

    let listening = start(app.clone()).unwrap();
    assert_eq!(listening.path(), socket);
    let mode = fs::metadata(&socket).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert!(fs::read_dir(&env.data_dir).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".mcp-")));

    let created = forward(
        &socket,
        "create_task",
        json!({"repo": "web", "title": "From an agent", "plan": "# Plan"}),
    )
    .unwrap();
    assert_eq!(created["key"], "PAY-1");
    let listed = forward(&socket, "list_tasks", json!({"status": "todo"})).unwrap();
    assert_eq!(listed[0]["title"], "From an agent");
    let err = forward(
        &socket,
        "update_task",
        json!({"task": "PAY-1", "title": " "}),
    )
    .unwrap_err();
    assert_eq!(err, "The title is empty.");
    assert_eq!(
        projects::list(&db.lock().unwrap(), false).unwrap()[0].next_task_number,
        2
    );

    // Several requests on one connection, and garbage gets an error instead of a hang.
    let mut s = UnixStream::connect(&socket).unwrap();
    s.write_all(b"not json\n{\"tool\":\"list_projects\"}\n")
        .unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let lines: Vec<Reply> = BufReader::new(s)
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    assert!(matches!(&lines[0], Reply::Error(e) if e.starts_with("Invalid request")));
    assert!(matches!(&lines[1], Reply::Result(v) if v[0]["key"] == "PAY"));

    // A second app with the same data folder does not steal the socket.
    assert!(start(app.clone())
        .unwrap_err()
        .contains("already listening"));

    // Stopped: the socket is gone and agents are told to open Nodal; it can start again.
    listening.stop();
    assert!(!socket.exists());
    assert_eq!(
        forward(&socket, "list_projects", json!({})).unwrap_err(),
        OPEN_NODAL
    );
    let again = start(app.clone()).unwrap();
    assert_eq!(
        forward(&socket, "list_projects", json!({})).unwrap()[0]["key"],
        "PAY"
    );

    // Someone else's socket at the same path survives our stop.
    fs::remove_file(&socket).unwrap();
    let other = UnixListener::bind(&socket).unwrap();
    again.stop();
    assert!(socket.exists());
    drop(other);
}

#[test]
fn stale_socket_is_replaced_and_other_files_are_kept() {
    let t = TempDir::new("mcpst");
    let path = t.0.join("s.sock");
    drop(UnixListener::bind(&path).unwrap());
    // A child spawned meanwhile by a parallel test (git) can inherit the listener for a moment
    // (no atomic CLOEXEC on macOS); wait until the socket is really stale.
    for _ in 0..500 {
        if UnixStream::connect(&path).is_err() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(path.exists());
    let l = bind_private(&path).unwrap();
    assert!(UnixStream::connect(&path).is_ok());
    drop(l);

    let file = t.0.join("f.sock");
    fs::write(&file, "x").unwrap();
    assert!(bind_private(&file).unwrap_err().contains("not a socket"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "x");
}
