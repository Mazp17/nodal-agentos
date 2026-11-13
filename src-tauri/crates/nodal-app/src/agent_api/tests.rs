use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nodal_domain::board::dto::{NewProject, NewRepo};
use nodal_domain::error::HostError;
use nodal_domain::model::chat::{ChatLive, ChatSpec};
use nodal_domain::model::claude::Transcript;
use nodal_domain::model::events::ChangeKind;
use nodal_domain::ports::{ChangeNotifier, ChatRuntime, Clock, SessionFiles};
use nodal_host::adapters::{HostClaudeConfig, HostGit, HostLocalFs, HostPlanFiles};
use nodal_host::testutil::TempDir;
use nodal_store::Db;
use serde_json::json;

use crate::board::ops;
use crate::core::{Core, Env};

use super::AgentApi;

/// Not exercised: `AgentApi::call` never reaches `Core::sessions`.
struct NoopSessionFiles;

impl SessionFiles for NoopSessionFiles {
    fn find_session_jsonl(
        &self,
        _projects: &Path,
        _cwd: &str,
        _session_id: &str,
    ) -> Option<PathBuf> {
        unimplemented!("not exercised by the agent_api tests")
    }
    fn read_session_transcript(
        &self,
        _path: &Path,
        _id: &str,
        _label: Option<String>,
        _model: Option<String>,
        _limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        unimplemented!("not exercised by the agent_api tests")
    }
    fn session_title(&self, _path: &Path) -> Option<String> {
        unimplemented!("not exercised by the agent_api tests")
    }
}

/// Not exercised: `AgentApi::call` never reaches `Core::chats`.
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
        unimplemented!("not exercised by the agent_api tests")
    }
    fn respond(
        &self,
        _chat_id: &str,
        _request_id: &str,
        _allow: bool,
        _message: Option<&str>,
        _answers: Option<&std::collections::BTreeMap<String, String>>,
    ) -> Result<(), HostError> {
        unimplemented!("not exercised by the agent_api tests")
    }
    fn interrupt(&self, _chat_id: &str) -> Result<(), HostError> {
        unimplemented!("not exercised by the agent_api tests")
    }
    fn stop(&self, _chat_id: &str) {}
    fn stop_project(&self, _project_id: &str) {}
    fn live(&self, _chat_id: &str) -> ChatLive {
        unimplemented!("not exercised by the agent_api tests")
    }
    fn start_reaper(&self) {}
}

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

/// Records every `notify` call, in order.
#[derive(Default)]
struct SpyNotifier {
    calls: Mutex<Vec<(ChangeKind, Option<String>)>>,
}

impl ChangeNotifier for SpyNotifier {
    fn notify(&self, kind: ChangeKind, project_id: Option<&str>) {
        self.calls
            .lock()
            .unwrap()
            .push((kind, project_id.map(String::from)));
    }
}

/// `Env` for tests: real (blocking, synchronous) host adapters rooted at `root`. Local copy
/// (agent_api owns its own tests; it doesn't reach into `nodal_app::testutil`).
fn test_env(root: &Path) -> Env {
    Env {
        data_dir: root.join("data"),
        worktrees_root: root.join("wt"),
        claude_dir: None,
        plans: Arc::new(HostPlanFiles),
        fs: Arc::new(HostLocalFs),
        git: Arc::new(HostGit),
        claude_config: Arc::new(HostClaudeConfig),
    }
}

fn agent_api(env: Env, notifier: Arc<SpyNotifier>) -> (AgentApi, Db) {
    let db = Db::open_in_memory().unwrap();
    let core = Arc::new(Core {
        db: db.clone(),
        env,
        rt: crate::testutil::rt(),
        clock: Arc::new(FixedClock(10)),
        sessions: Arc::new(NoopSessionFiles),
        notifier,
        chats: Arc::new(NoopChatRuntime),
    });
    (AgentApi::new(core), db)
}

/// Project PAY with one repo, seeded directly through `board::ops` (bypassing MCP dispatch).
fn seed(db: &Db, env: &Env) -> (String, String) {
    let c = db.lock().unwrap();
    let p = ops::create_project(
        &c,
        env,
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
    let repo = ops::add_repo(
        &c,
        &p.id,
        &NewRepo::default(),
        &PathBuf::from("/tmp/pay-web"),
        2,
    )
    .unwrap();
    (p.id, repo.id)
}

#[test]
fn call_dispatches_list_projects_without_notifying() {
    let t = TempDir::new("agent-api-list");
    let notifier = Arc::new(SpyNotifier::default());
    let (api, _db) = agent_api(test_env(&t.0), notifier.clone());

    assert_eq!(api.call("list_projects", json!({})).unwrap(), json!([]));
    assert!(notifier.calls.lock().unwrap().is_empty());
}

#[test]
fn call_creates_a_task_with_the_injected_clock_and_notifies_its_project() {
    let t = TempDir::new("agent-api-create");
    let env = test_env(&t.0);
    let notifier = Arc::new(SpyNotifier::default());
    let (api, db) = agent_api(env.clone(), notifier.clone());
    assert_eq!(api.data_dir(), env.data_dir.as_path());
    let (project_id, repo_id) = seed(&db, &env);

    let value = api
        .call(
            "create_task",
            json!({"repo": repo_id, "title": "From an agent", "plan": "# Plan"}),
        )
        .unwrap();
    assert_eq!(value["key"], "PAY-1");
    assert_eq!(value["createdAt"], 10);

    assert_eq!(
        notifier.calls.lock().unwrap().as_slice(),
        [(ChangeKind::Tasks, Some(project_id))]
    );
}

#[test]
fn call_reports_a_read_only_tool_without_notifying() {
    let t = TempDir::new("agent-api-propose");
    let env = test_env(&t.0);
    let notifier = Arc::new(SpyNotifier::default());
    let (api, db) = agent_api(env.clone(), notifier.clone());
    let (_project_id, repo_id) = seed(&db, &env);

    let value = api
        .call(
            "propose_task",
            json!({"repo": repo_id, "title": "From an agent", "plan": "# Plan"}),
        )
        .unwrap();
    assert_eq!(value["created"], false);
    assert!(notifier.calls.lock().unwrap().is_empty());
}

#[test]
fn call_reports_unknown_tools_without_notifying() {
    let t = TempDir::new("agent-api-unknown");
    let notifier = Arc::new(SpyNotifier::default());
    let (api, _db) = agent_api(test_env(&t.0), notifier.clone());

    let err = api.call("launch_task", json!({})).unwrap_err();
    assert!(err.contains("Unknown tool"));
    assert!(notifier.calls.lock().unwrap().is_empty());
}

#[test]
fn start_task_chat_rejects_unknown_tasks_and_missing_args() {
    let t = TempDir::new("agent-api-chat");
    let (api, _db) = agent_api(test_env(&t.0), Arc::new(SpyNotifier::default()));
    assert!(api.call("start_task_chat", json!({})).unwrap_err().contains("task"));
    assert!(api.call("start_task_chat", json!({"task": "PAY-9"})).is_err());
}
