use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nodal_domain::error::HostError;
use nodal_domain::model::activity::{AgentSession, AppRuns, ExternalSessions};
use nodal_domain::model::chat::{ChatLive, ChatSpec};
use nodal_domain::model::claude::{
    DetailSource, ExtraFlags, LaunchBlocker, LaunchError, RunDetail, RunRef, RunSummary,
    SessionReadout, Transcript,
};
use nodal_domain::model::{Executor, LaunchOptions, RunKind};
use nodal_domain::ports::{BoxFut, ChatRuntime, ClaudeCli, Clock, SessionFiles};
use nodal_domain::sessions::transcript::{TRANSCRIPT_DEFAULT_LIMIT, TRANSCRIPT_MAX_LIMIT};
use nodal_domain::testutil::{project_of, repo_of, run_of};
use nodal_host::testutil::TempDir;
use nodal_store::Db;

use crate::core::Core;
use crate::testutil::{block_on, env_for, rt, NoopNotifier};

use super::{SessionReader, Sessions};

const VALID_SESSION_ID: &str = "ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8";

#[derive(Default)]
struct FakeClaude {
    sessions: Vec<RunSummary>,
    agent_sessions: Vec<AgentSession>,
    err: Option<String>,
}

impl ClaudeCli for FakeClaude {
    fn list_sessions(&self) -> BoxFut<'_, Result<Vec<RunSummary>, HostError>> {
        Box::pin(async move {
            match &self.err {
                Some(e) => Err(HostError::from(e.clone())),
                None => Ok(self.sessions.clone()),
            }
        })
    }

    fn list_agent_sessions(&self) -> BoxFut<'_, Result<Vec<AgentSession>, HostError>> {
        Box::pin(async move {
            match &self.err {
                Some(e) => Err(HostError::from(e.clone())),
                None => Ok(self.agent_sessions.clone()),
            }
        })
    }

    fn launch_bg<'a>(
        &'a self,
        _cwd: String,
        _prompt: String,
        _opts: &'a LaunchOptions,
        _extra: &'a ExtraFlags,
    ) -> BoxFut<'a, Result<RunRef, LaunchError>> {
        Box::pin(async move { unimplemented!("not exercised by sessions tests") })
    }

    fn stop<'a>(&'a self, _short_id: &'a str) -> BoxFut<'a, Result<(), HostError>> {
        Box::pin(async move { unimplemented!("not exercised by sessions tests") })
    }
}

#[derive(Default)]
struct FakeFiles {
    run_detail: Option<RunDetail>,
    launch_blocker: Option<LaunchBlocker>,
    transcript: Option<Transcript>,
    jsonl: Option<PathBuf>,
    external: Option<ExternalSessions>,
    panic: bool,
    last_limit: AtomicUsize,
    /// Canned `count_new_tool_calls` results, popped in order; `(0, from)` once exhausted.
    progress_responses: Mutex<VecDeque<(u32, u64)>>,
    last_progress_from: AtomicU64,
}

impl SessionFiles for FakeFiles {
    fn read_session(&self, _session_id: &str, _cwd: &str) -> SessionReadout {
        unimplemented!("not exercised by sessions tests")
    }

    fn session_tokens(&self, _session_id: &str, _cwd: &str) -> Option<i64> {
        None
    }

    fn run_detail(&self, _session_id: &str, _cwd: &str) -> Result<Option<RunDetail>, HostError> {
        if self.panic {
            panic!("boom");
        }
        Ok(self.run_detail.clone())
    }

    fn launch_blocker(
        &self,
        _session_id: &str,
        _cwd: &str,
    ) -> Result<Option<LaunchBlocker>, HostError> {
        Ok(self.launch_blocker.clone())
    }

    fn agent_transcript(
        &self,
        _session_id: &str,
        _cwd: &str,
        _run_id: &str,
        _agent_id: &str,
        limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        self.last_limit.store(limit, Ordering::SeqCst);
        Ok(self.transcript.clone())
    }

    fn find_session_jsonl(
        &self,
        _projects: &Path,
        _cwd: &str,
        _session_id: &str,
    ) -> Option<PathBuf> {
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
        None
    }

    fn count_new_tool_calls(&self, _path: &Path, from: u64) -> std::io::Result<(u32, u64)> {
        self.last_progress_from.store(from, Ordering::SeqCst);
        Ok(self
            .progress_responses
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front()
            .unwrap_or((0, from)))
    }

    fn external_sessions(
        &self,
        _repos: &[(String, PathBuf)],
        _agents: &[AgentSession],
        _app: &AppRuns,
        _now: i64,
    ) -> Result<ExternalSessions, HostError> {
        Ok(self.external.clone().unwrap_or(ExternalSessions {
            repos: vec![],
            generated_at: 0,
        }))
    }
}

struct FakeClock(i64);

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

/// Sessions doesn't touch chats; every method is unreachable from these tests.
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
        unimplemented!("not exercised by sessions tests")
    }
    fn respond(
        &self,
        _chat_id: &str,
        _request_id: &str,
        _allow: bool,
        _message: Option<&str>,
    ) -> Result<(), HostError> {
        unimplemented!("not exercised by sessions tests")
    }
    fn interrupt(&self, _chat_id: &str) -> Result<(), HostError> {
        unimplemented!("not exercised by sessions tests")
    }
    fn stop(&self, _chat_id: &str) {}
    fn stop_project(&self, _project_id: &str) {}
    fn live(&self, _chat_id: &str) -> ChatLive {
        unimplemented!("not exercised by sessions tests")
    }
    fn start_reaper(&self) {}
}

fn test_core(
    root: &Path,
    claude: Arc<dyn ClaudeCli>,
    sessions: Arc<dyn SessionFiles>,
    claude_dir: Option<PathBuf>,
) -> Arc<Core> {
    Arc::new(Core {
        db: Db::open_in_memory().unwrap(),
        env: env_for(root, claude_dir),
        rt: rt(),
        clock: Arc::new(FakeClock(1_000)),
        claude,
        sessions,
        notifier: Arc::new(NoopNotifier),
        chats: Arc::new(NoopChatRuntime),
    })
}

fn detail_of(workflow_id: &str) -> RunDetail {
    RunDetail {
        workflow_id: workflow_id.into(),
        workflow_name: None,
        source: DetailSource::Live,
        status: None,
        phases: vec![],
        current_phase: None,
        current_phase_index: None,
        agents: vec![],
        agent_count: 0,
        total_tokens: None,
        total_tool_calls: None,
        duration_ms: None,
        result_status: None,
        result: None,
        workflow_count: 1,
    }
}

fn transcript_of(agent_id: &str) -> Transcript {
    Transcript {
        agent_id: agent_id.into(),
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
    }
}

// ---------- SessionReader ----------

#[test]
fn list_runs_returns_the_claude_cli_sessions() {
    let summary = RunSummary {
        id: "s1".into(),
        session_id: "sess1".into(),
        cwd: None,
        name: None,
        started_at: None,
        pid: None,
        status: None,
        state: None,
        waiting_for: None,
    };
    let reader = SessionReader::new(
        Arc::new(FakeClaude {
            sessions: vec![summary.clone()],
            ..Default::default()
        }),
        Arc::new(FakeFiles::default()),
    );
    assert_eq!(block_on(reader.list_runs()).unwrap(), vec![summary]);
}

#[test]
fn list_runs_propagates_the_claude_cli_error() {
    let reader = SessionReader::new(
        Arc::new(FakeClaude {
            err: Some("`claude agents` failed: boom".into()),
            ..Default::default()
        }),
        Arc::new(FakeFiles::default()),
    );
    let err = block_on(reader.list_runs()).unwrap_err();
    assert_eq!(err.to_string(), "`claude agents` failed: boom");
}

#[test]
fn get_run_detail_rejects_invalid_session_ids() {
    let reader = SessionReader::new(
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
    );
    let err = block_on(reader.get_run_detail("../etc".into(), "/x".into())).unwrap_err();
    assert_eq!(err.to_string(), "Invalid session id: ../etc");
}

#[test]
fn get_run_detail_returns_what_session_files_reports() {
    let detail = detail_of("wf_1");
    let reader = SessionReader::new(
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles {
            run_detail: Some(detail.clone()),
            ..Default::default()
        }),
    );
    let got = block_on(reader.get_run_detail(VALID_SESSION_ID.into(), "/x".into())).unwrap();
    assert_eq!(got, Some(detail));
}

#[test]
fn get_run_detail_reports_a_panic_as_an_internal_error() {
    let reader = SessionReader::new(
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles {
            panic: true,
            ..Default::default()
        }),
    );
    let err = block_on(reader.get_run_detail(VALID_SESSION_ID.into(), "/x".into())).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("Internal error reading the session: "),
        "{err}"
    );
}

#[test]
fn get_launch_blocker_rejects_invalid_session_ids() {
    let reader = SessionReader::new(
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
    );
    let err = block_on(reader.get_launch_blocker("bad id".into(), "/x".into())).unwrap_err();
    assert_eq!(err.to_string(), "Invalid session id: bad id");
}

#[test]
fn get_launch_blocker_returns_what_session_files_reports() {
    let blocker = LaunchBlocker::WorkflowReview {
        workflow: Some("wf".into()),
    };
    let reader = SessionReader::new(
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles {
            launch_blocker: Some(blocker.clone()),
            ..Default::default()
        }),
    );
    let got = block_on(reader.get_launch_blocker(VALID_SESSION_ID.into(), "/x".into())).unwrap();
    assert_eq!(got, Some(blocker));
}

/// Moved from the (now thin) `get_agent_transcript` command: the validation this test checks
/// lives in `SessionReader` now.
#[test]
fn transcript_rejects_traversal_ids() {
    let reader = SessionReader::new(
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
    );
    let call = |run: &str, agent: &str| {
        block_on(reader.get_agent_transcript(
            VALID_SESSION_ID.into(),
            "/x".into(),
            run.into(),
            agent.into(),
            None,
        ))
    };
    assert!(call("wf_../../etc", "a1")
        .unwrap_err()
        .to_string()
        .contains("Invalid workflow run id"));
    assert!(call("../wf_x", "a1")
        .unwrap_err()
        .to_string()
        .contains("Invalid workflow run id"));
    assert!(call("wf_abc", "../../x")
        .unwrap_err()
        .to_string()
        .contains("Invalid agent id"));
    assert!(call("wf_abc", "a/b")
        .unwrap_err()
        .to_string()
        .contains("Invalid agent id"));
}

#[test]
fn get_agent_transcript_returns_what_session_files_reports() {
    let transcript = transcript_of("a1");
    let files = Arc::new(FakeFiles {
        transcript: Some(transcript.clone()),
        ..Default::default()
    });
    let reader = SessionReader::new(Arc::new(FakeClaude::default()), files);
    let got = block_on(reader.get_agent_transcript(
        VALID_SESSION_ID.into(),
        "/x".into(),
        "wf_1".into(),
        "a1".into(),
        None,
    ))
    .unwrap();
    assert_eq!(got, Some(transcript));
}

#[test]
fn get_agent_transcript_defaults_the_limit() {
    let files = Arc::new(FakeFiles::default());
    let reader = SessionReader::new(Arc::new(FakeClaude::default()), files.clone());
    block_on(reader.get_agent_transcript(
        VALID_SESSION_ID.into(),
        "/x".into(),
        "wf_1".into(),
        "a1".into(),
        None,
    ))
    .unwrap();
    assert_eq!(
        files.last_limit.load(Ordering::SeqCst),
        TRANSCRIPT_DEFAULT_LIMIT as usize
    );
}

#[test]
fn get_agent_transcript_clamps_the_limit_to_the_max() {
    let files = Arc::new(FakeFiles::default());
    let reader = SessionReader::new(Arc::new(FakeClaude::default()), files.clone());
    block_on(reader.get_agent_transcript(
        VALID_SESSION_ID.into(),
        "/x".into(),
        "wf_1".into(),
        "a1".into(),
        Some(999_999),
    ))
    .unwrap();
    assert_eq!(
        files.last_limit.load(Ordering::SeqCst),
        TRANSCRIPT_MAX_LIMIT as usize
    );
}

// ---------- Sessions ----------

fn seed_run(core: &Core, run: &nodal_domain::model::Run) {
    let guard = core.db.lock().unwrap();
    nodal_store::execution::runs::insert(&guard, run).unwrap();
}

#[test]
fn get_run_transcript_rejects_invalid_run_ids() {
    let tmp = TempDir::new("sessions-transcript-invalid-id");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let sessions = Sessions::new(core);
    let err = block_on(sessions.get_run_transcript("../nope".into(), None)).unwrap_err();
    assert_eq!(err.to_string(), "Invalid run id: \"../nope\".");
}

#[test]
fn get_run_transcript_errors_for_workflow_runs() {
    let tmp = TempDir::new("sessions-transcript-workflow");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let mut run = run_of(
        Executor::Workflow { name: "wf".into() },
        RunKind::Work,
        false,
    );
    run.task_id = None;
    run.repo_id = None;
    seed_run(&core, &run);
    let sessions = Sessions::new(core);
    let err = block_on(sessions.get_run_transcript(run.id.clone(), None)).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Workflow runs have one transcript per agent: open it from the run detail."
    );
}

#[test]
fn get_run_transcript_returns_none_without_a_valid_session_id() {
    let tmp = TempDir::new("sessions-transcript-no-session");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let mut run = run_of(Executor::Claude, RunKind::Work, false);
    run.task_id = None;
    run.repo_id = None;
    run.session_id = None;
    seed_run(&core, &run);
    let sessions = Sessions::new(core);
    assert_eq!(
        block_on(sessions.get_run_transcript(run.id.clone(), None)).unwrap(),
        None
    );
}

#[test]
fn get_run_transcript_errors_without_a_claude_dir() {
    let tmp = TempDir::new("sessions-transcript-no-claude-dir");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let mut run = run_of(Executor::Claude, RunKind::Work, false);
    run.task_id = None;
    run.repo_id = None;
    run.session_id = Some("sess-1".into());
    seed_run(&core, &run);
    let sessions = Sessions::new(core);
    let err = block_on(sessions.get_run_transcript(run.id.clone(), None)).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Couldn't locate the Claude Code folder ($HOME is not set)."
    );
}

#[test]
fn get_run_transcript_reads_the_session_transcript() {
    let tmp = TempDir::new("sessions-transcript-happy");
    let transcript = transcript_of("a1");
    let files = Arc::new(FakeFiles {
        transcript: Some(transcript.clone()),
        jsonl: Some(tmp.0.join("s.jsonl")),
        ..Default::default()
    });
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        files,
        Some(tmp.0.clone()),
    );
    let mut run = run_of(Executor::Claude, RunKind::Work, false);
    run.task_id = None;
    run.repo_id = None;
    run.session_id = Some("sess-1".into());
    seed_run(&core, &run);
    let sessions = Sessions::new(core);
    assert_eq!(
        block_on(sessions.get_run_transcript(run.id.clone(), None)).unwrap(),
        Some(transcript)
    );
}

// ---------- run_progress (P03) ----------

#[test]
fn run_progress_rejects_invalid_run_ids() {
    let tmp = TempDir::new("sessions-progress-invalid-id");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let sessions = Sessions::new(core);
    let err = block_on(sessions.run_progress("../nope".into())).unwrap_err();
    assert_eq!(err.to_string(), "Invalid run id: \"../nope\".");
}

#[test]
fn run_progress_errors_for_workflow_runs() {
    let tmp = TempDir::new("sessions-progress-workflow");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let mut run = run_of(
        Executor::Workflow { name: "wf".into() },
        RunKind::Work,
        false,
    );
    run.task_id = None;
    run.repo_id = None;
    seed_run(&core, &run);
    let sessions = Sessions::new(core);
    let err = block_on(sessions.run_progress(run.id.clone())).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Workflow runs report progress per phase: open the run detail instead."
    );
}

#[test]
fn run_progress_returns_none_without_a_valid_session_id() {
    let tmp = TempDir::new("sessions-progress-no-session");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let mut run = run_of(Executor::Claude, RunKind::Work, false);
    run.task_id = None;
    run.repo_id = None;
    run.session_id = None;
    seed_run(&core, &run);
    let sessions = Sessions::new(core);
    assert_eq!(
        block_on(sessions.run_progress(run.id.clone())).unwrap(),
        None
    );
}

#[test]
fn run_progress_errors_without_a_claude_dir() {
    let tmp = TempDir::new("sessions-progress-no-claude-dir");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let mut run = run_of(Executor::Claude, RunKind::Work, false);
    run.task_id = None;
    run.repo_id = None;
    run.session_id = Some("sess-1".into());
    seed_run(&core, &run);
    let sessions = Sessions::new(core);
    let err = block_on(sessions.run_progress(run.id.clone())).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Couldn't locate the Claude Code folder ($HOME is not set)."
    );
}

/// The point of the incremental cursor: each poll passes the offset the previous one left
/// off at, and the returned count is cumulative, not the delta.
#[test]
fn run_progress_reads_from_the_cursor_and_accumulates() {
    let tmp = TempDir::new("sessions-progress-happy");
    let files = Arc::new(FakeFiles {
        jsonl: Some(tmp.0.join("s.jsonl")),
        progress_responses: Mutex::new(VecDeque::from([(3, 100), (0, 100), (2, 250)])),
        ..Default::default()
    });
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        files.clone(),
        Some(tmp.0.clone()),
    );
    let mut run = run_of(Executor::Claude, RunKind::Work, false);
    run.task_id = None;
    run.repo_id = None;
    run.session_id = Some("sess-1".into());
    seed_run(&core, &run);
    let sessions = Sessions::new(core);

    let p1 = block_on(sessions.run_progress(run.id.clone())).unwrap().unwrap();
    assert_eq!(p1.tool_calls, 3);
    assert_eq!(files.last_progress_from.load(Ordering::SeqCst), 0);

    // Steady state: no new data, the count doesn't change and the cursor isn't rewound.
    let p2 = block_on(sessions.run_progress(run.id.clone())).unwrap().unwrap();
    assert_eq!(p2.tool_calls, 3);
    assert_eq!(files.last_progress_from.load(Ordering::SeqCst), 100);

    // New data: the delta is added on top, not replacing the running total.
    let p3 = block_on(sessions.run_progress(run.id.clone())).unwrap().unwrap();
    assert_eq!(p3.tool_calls, 5);
    assert_eq!(files.last_progress_from.load(Ordering::SeqCst), 100);
}

#[test]
fn external_sessions_rejects_invalid_project_ids() {
    let tmp = TempDir::new("sessions-external-invalid-id");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let sessions = Sessions::new(core);
    let err = block_on(sessions.external_sessions(Some("bad id".into()))).unwrap_err();
    assert_eq!(err.to_string(), "Invalid project id: \"bad id\".");
}

#[test]
fn external_sessions_returns_empty_when_there_are_no_repos() {
    let tmp = TempDir::new("sessions-external-no-repos");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude::default()),
        Arc::new(FakeFiles::default()),
        None,
    );
    let sessions = Sessions::new(core);
    let got = block_on(sessions.external_sessions(None)).unwrap();
    assert_eq!(
        got,
        ExternalSessions {
            repos: vec![],
            generated_at: 1_000
        }
    );
}

#[test]
fn external_sessions_reads_through_the_ports() {
    let tmp = TempDir::new("sessions-external-happy");
    let want = ExternalSessions {
        repos: vec![],
        generated_at: 42,
    };
    let files = Arc::new(FakeFiles {
        external: Some(want.clone()),
        ..Default::default()
    });
    let core = test_core(&tmp.0, Arc::new(FakeClaude::default()), files, None);
    {
        let guard = core.db.lock().unwrap();
        nodal_store::rows::insert_project(&guard, &project_of("p1", "PAY")).unwrap();
        nodal_store::rows::insert_repo(&guard, &repo_of("r1", "p1", "/r1")).unwrap();
    }
    let sessions = Sessions::new(core);
    assert_eq!(block_on(sessions.external_sessions(None)).unwrap(), want);
}

#[test]
fn external_sessions_propagates_the_claude_cli_error() {
    let tmp = TempDir::new("sessions-external-claude-err");
    let core = test_core(
        &tmp.0,
        Arc::new(FakeClaude {
            err: Some("agents failed".into()),
            ..Default::default()
        }),
        Arc::new(FakeFiles::default()),
        None,
    );
    {
        let guard = core.db.lock().unwrap();
        nodal_store::rows::insert_project(&guard, &project_of("p1", "PAY")).unwrap();
        nodal_store::rows::insert_repo(&guard, &repo_of("r1", "p1", "/r1")).unwrap();
    }
    let sessions = Sessions::new(core);
    let err = block_on(sessions.external_sessions(None)).unwrap_err();
    assert_eq!(err.to_string(), "agents failed");
}
