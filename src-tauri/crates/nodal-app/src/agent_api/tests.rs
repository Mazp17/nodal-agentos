use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nodal_domain::board::dto::{NewProject, NewRepo};
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
use serde_json::{json, Value};

use crate::board::ops;
use crate::core::{Core, Env};

use super::AgentApi;

/// Only `list_sessions` is reached (`get_queue`); concurrency 0 keeps the queue from launching.
struct NoopClaude;

impl ClaudeCli for NoopClaude {
    fn list_sessions(&self) -> BoxFut<'_, Result<Vec<RunSummary>, HostError>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
    fn list_agent_sessions(&self) -> BoxFut<'_, Result<Vec<AgentSession>, HostError>> {
        Box::pin(async move { unimplemented!("not exercised by the agent_api tests") })
    }
    fn launch_bg<'a>(
        &'a self,
        _cwd: String,
        _prompt: String,
        _opts: &'a LaunchOptions,
        _extra: &'a ExtraFlags,
    ) -> BoxFut<'a, Result<RunRef, LaunchError>> {
        Box::pin(async move { unimplemented!("not exercised by the agent_api tests") })
    }
    fn stop<'a>(&'a self, _short_id: &'a str) -> BoxFut<'a, Result<(), HostError>> {
        Box::pin(async move { unimplemented!("not exercised by the agent_api tests") })
    }
}

/// Not exercised: `AgentApi::call` never reaches `Core::sessions`.
struct NoopSessionFiles;

impl SessionFiles for NoopSessionFiles {
    fn read_session(&self, _session_id: &str, _cwd: &str) -> SessionReadout {
        unimplemented!("not exercised by the agent_api tests")
    }
    fn session_tokens(&self, _session_id: &str, _cwd: &str) -> Option<i64> {
        unimplemented!("not exercised by the agent_api tests")
    }
    fn run_detail(&self, _session_id: &str, _cwd: &str) -> Result<Option<RunDetail>, HostError> {
        unimplemented!("not exercised by the agent_api tests")
    }
    fn launch_blocker(
        &self,
        _session_id: &str,
        _cwd: &str,
    ) -> Result<Option<LaunchBlocker>, HostError> {
        unimplemented!("not exercised by the agent_api tests")
    }
    fn agent_transcript(
        &self,
        _session_id: &str,
        _cwd: &str,
        _run_id: &str,
        _agent_id: &str,
        _limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        unimplemented!("not exercised by the agent_api tests")
    }
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
    fn external_sessions(
        &self,
        _repos: &[(String, PathBuf)],
        _agents: &[AgentSession],
        _app: &AppRuns,
        _now: i64,
    ) -> Result<ExternalSessions, HostError> {
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
    {
        // Concurrency 0: the queue pass `launch_run` kicks never launches, so runs stay queued.
        let mut c = db.lock().unwrap();
        let mut s = nodal_store::rows::load_settings(&c).unwrap();
        s.concurrency = 0;
        nodal_store::rows::save_settings(&mut c, &s).unwrap();
    }
    let core = Arc::new(Core {
        db: db.clone(),
        env,
        rt: crate::testutil::rt(),
        clock: Arc::new(FixedClock(10)),
        claude: Arc::new(NoopClaude),
        sessions: Arc::new(NoopSessionFiles),
        notifier,
        chats: Arc::new(NoopChatRuntime),
    });
    let execution = crate::execution::Execution::new(core.clone());
    (AgentApi::new(core, execution), db)
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

/// A task PAY-1 in a repo folder on disk (in place, so no worktree is needed).
#[allow(clippy::disallowed_methods)] // builds the repo folder directly on disk
fn seed_task(api: &AgentApi, db: &Db, env: &Env, root: &Path) -> String {
    let (project_id, _) = seed(db, env);
    let dir = root.join("web");
    std::fs::create_dir_all(&dir).unwrap();
    let repo = ops::add_repo(
        &db.lock().unwrap(),
        &project_id,
        &serde_json::from_value(json!({"path": "web", "defaultIsolation": "in_place"})).unwrap(),
        &dir,
        3,
    )
    .unwrap();
    let task = api
        .call(
            "create_task",
            json!({"repo": repo.id, "title": "Logo", "plan": "# Plan"}),
        )
        .unwrap();
    task["key"].as_str().unwrap().to_string()
}

#[test]
fn launch_run_queues_with_the_given_options_and_the_run_tools_follow_it() {
    let t = TempDir::new("agent-api-launch");
    let env = test_env(&t.0);
    let notifier = Arc::new(SpyNotifier::default());
    let (api, db) = agent_api(env.clone(), notifier.clone());
    let key = seed_task(&api, &db, &env, &t.0);
    notifier.calls.lock().unwrap().clear();

    let run = api
        .call(
            "launch_run",
            json!({"task": key, "model": "sonnet", "effort": "high", "finish": "commit",
                   "review": false, "extraInstructions": "Be brief."}),
        )
        .unwrap();
    assert_eq!(run["status"], "queued");
    assert_eq!(run["taskKey"], "PAY-1");
    assert_eq!(run["options"]["model"], "sonnet");
    assert_eq!(run["options"]["effort"], "high");
    assert_eq!(run["finish"], "commit");
    assert_eq!(run["isolation"], "in_place");
    assert!(notifier
        .calls
        .lock()
        .unwrap()
        .contains(&(ChangeKind::Runs, None)));

    let err = api.call("launch_run", json!({"task": "PAY-1"})).unwrap_err();
    assert!(err.contains("already has a queued run"), "{err}");

    let queue = api.call("get_queue", json!({"project": "pay"})).unwrap();
    assert_eq!(queue["queued"], 1);
    assert_eq!(queue["runs"][0]["id"], run["id"]);

    let listed = api
        .call("list_runs", json!({"task": "PAY-1", "status": "queued"}))
        .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["taskKey"], "PAY-1");
    let got = api.call("get_run", json!({"task": "PAY-1"})).unwrap();
    assert_eq!(got["run"]["id"], run["id"]);

    let canceled = api
        .call("cancel_run", json!({"runId": run["id"]}))
        .unwrap();
    assert_eq!(canceled["status"], "canceled");
    let listed = api
        .call("list_runs", json!({"status": ["queued", "launched"]}))
        .unwrap();
    assert_eq!(listed, json!([]));
}

#[test]
fn launch_run_rejects_invalid_options_and_unknown_tasks() {
    let t = TempDir::new("agent-api-launch-invalid");
    let env = test_env(&t.0);
    let (api, db) = agent_api(env.clone(), Arc::new(SpyNotifier::default()));
    let key = seed_task(&api, &db, &env, &t.0);

    let err = api
        .call("launch_run", json!({"task": key, "effort": "huge"}))
        .unwrap_err();
    assert!(err.contains("Invalid effort"), "{err}");
    let err = api
        .call(
            "launch_run",
            json!({"task": key, "executor": {"kind": "workflow", "name": "nope"}}),
        )
        .unwrap_err();
    assert!(err.contains("Workflow \"nope\" not found"), "{err}");
    let err = api.call("launch_run", json!({"task": "PAY-9"})).unwrap_err();
    assert!(err.contains("No task"), "{err}");
    let err = api
        .call("launch_run", json!({"task": key, "bogus": 1}))
        .unwrap_err();
    assert!(err.contains("Invalid arguments"), "{err}");
}

#[test]
fn list_executors_reports_the_launch_options_and_the_repo_defaults() {
    let t = TempDir::new("agent-api-executors");
    let env = test_env(&t.0);
    let (api, db) = agent_api(env.clone(), Arc::new(SpyNotifier::default()));
    let (_, repo_id) = seed(&db, &env);

    let out = api.call("list_executors", json!({"repo": repo_id})).unwrap();
    assert!(out["executors"].is_array());
    assert!(out["efforts"].as_array().unwrap().contains(&json!("max")));
    assert!(out["models"].as_array().unwrap().contains(&json!("opus")));
    assert_eq!(out["repoDefaults"]["repoId"], repo_id);
    let out = api.call("list_executors", json!({})).unwrap();
    assert_eq!(out["repoDefaults"], Value::Null);
}
