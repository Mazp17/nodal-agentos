//! Facade tests: `Execution`'s Tauri-command methods, exercised end to end against an
//! in-memory `Db` with fake `Git`/`ClaudeCli` ports (so nothing here needs a real repo or the
//! `claude` binary). `PlanFiles`/`LocalFs`/`ClaudeConfig` stay the real, blocking host adapters
//! (as the rest of nodal-app's ported tests already do): they're simple disk probes rooted in
//! a `TempDir`, not worth faking.
#![allow(clippy::disallowed_methods)] // TempDir-rooted fixtures touch the real filesystem

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;

use nodal_domain::board::dto::{MergeInput, NewProject, NewRepo, NewTask};
use nodal_domain::diff::CommitInfo;
use nodal_domain::error::HostError;
use nodal_domain::execution::worktree::{MergeOutcome, WorktreeStatus};
use nodal_domain::model::activity::{AgentSession, AppRuns, ExternalSessions};
use nodal_domain::model::chat::{ChatLive, ChatSpec};
use nodal_domain::model::claude::{
    ExtraFlags, LaunchBlocker, LaunchError, RunDetail, RunRef, RunSummary, SessionReadout, Transcript,
};
use nodal_domain::model::*;
use nodal_domain::ports::{BoxFut, ChatRuntime, ClaudeCli, Git, SessionFiles};
use nodal_domain::testutil::run_of;
use nodal_host::adapters::{HostClaudeConfig, HostLocalFs, HostPlanFiles};
use nodal_host::testutil::TempDir;
use nodal_store::board::{projects, repos, tasks};
use nodal_store::execution::runs as qruns;
use nodal_store::Db;

use crate::board::ops;
use crate::core::{Core, Env};
use crate::testutil::{block_on, NoopNotifier};

use super::Execution;

#[derive(Default)]
struct FakeGit {
    status: Mutex<WorktreeStatus>,
    diff: Mutex<(String, bool)>,
    commits: Mutex<Vec<CommitInfo>>,
    branch: Mutex<Option<String>>,
    base: Mutex<String>,
    merge_outcome: Mutex<Option<MergeOutcome>>,
}

impl Git for FakeGit {
    fn require_git_root(&self, path: &str) -> Result<PathBuf, HostError> {
        Ok(PathBuf::from(path))
    }
    fn ensure_worktree(&self, _repo: &Path, dir: &Path, branch: &str, existing: Option<&WorktreeRef>) -> Result<WorktreeRef, HostError> {
        Ok(existing.cloned().unwrap_or_else(|| WorktreeRef {
            path: dir.to_string_lossy().into_owned(),
            branch: branch.to_string(),
            base: self.base.lock().unwrap().clone(),
        }))
    }
    fn cleanup_worktree(&self, _repo: &Path, _wt: &WorktreeRef) -> Result<(), HostError> {
        Ok(())
    }
    fn worktree_status(&self, _repo: &Path, _wt: Option<&WorktreeRef>) -> Result<WorktreeStatus, HostError> {
        Ok(self.status.lock().unwrap().clone())
    }
    fn current_base(&self, _repo: &Path) -> Result<String, HostError> {
        Ok(self.base.lock().unwrap().clone())
    }
    fn diff(&self, _cwd: &Path, _base: Option<&str>) -> Result<(String, bool), HostError> {
        Ok(self.diff.lock().unwrap().clone())
    }
    fn diff_branch(&self, _repo: &Path, _base: &str, _branch: &str) -> Result<String, HostError> {
        Ok(self.diff.lock().unwrap().0.clone())
    }
    fn commits(&self, _dir: &Path, _base: &str, _head: &str) -> Result<Vec<CommitInfo>, HostError> {
        Ok(self.commits.lock().unwrap().clone())
    }
    fn current_branch(&self, _dir: &Path) -> Option<String> {
        self.branch.lock().unwrap().clone()
    }
    fn merge(&self, _repo: &Path, _wt: &WorktreeRef, _message: &str, _squash: bool) -> Result<MergeOutcome, HostError> {
        Ok(self.merge_outcome.lock().unwrap().clone().unwrap_or(MergeOutcome::Merged {
            commit: "c1".into(),
            commits: 1,
            squashed: false,
            moved: false,
        }))
    }
    fn push_base(&self, _repo: &Path, _base: &str) -> Result<String, HostError> {
        Ok("origin/main".into())
    }
}

#[derive(Default)]
struct FakeClaude {
    sessions: Mutex<Vec<RunSummary>>,
    stopped: Mutex<Vec<String>>,
}

impl ClaudeCli for FakeClaude {
    fn list_sessions(&self) -> BoxFut<'_, Result<Vec<RunSummary>, HostError>> {
        let v = self.sessions.lock().unwrap().clone();
        Box::pin(async move { Ok(v) })
    }
    fn list_agent_sessions(&self) -> BoxFut<'_, Result<Vec<AgentSession>, HostError>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn launch_bg<'a>(
        &'a self,
        cwd: String,
        _prompt: String,
        _opts: &'a LaunchOptions,
        _extra: &'a ExtraFlags,
    ) -> BoxFut<'a, Result<RunRef, LaunchError>> {
        Box::pin(async move { Ok(RunRef { id: "bg1".into(), cwd }) })
    }
    fn stop<'a>(&'a self, short_id: &'a str) -> BoxFut<'a, Result<(), HostError>> {
        self.stopped.lock().unwrap().push(short_id.to_string());
        Box::pin(async { Ok(()) })
    }
}

struct FakeSessions;

impl SessionFiles for FakeSessions {
    fn read_session(&self, _session_id: &str, _cwd: &str) -> SessionReadout {
        SessionReadout::default()
    }
    fn session_tokens(&self, _session_id: &str, _cwd: &str) -> Option<i64> {
        Some(42)
    }
    fn run_detail(&self, _session_id: &str, _cwd: &str) -> Result<Option<RunDetail>, HostError> {
        unimplemented!("not exercised by Execution")
    }
    fn launch_blocker(&self, _session_id: &str, _cwd: &str) -> Result<Option<LaunchBlocker>, HostError> {
        unimplemented!("not exercised by Execution")
    }
    fn agent_transcript(
        &self,
        _session_id: &str,
        _cwd: &str,
        _run_id: &str,
        _agent_id: &str,
        _limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        unimplemented!("not exercised by Execution")
    }
    fn find_session_jsonl(&self, _projects: &Path, _cwd: &str, _session_id: &str) -> Option<PathBuf> {
        unimplemented!("not exercised by Execution")
    }
    fn read_session_transcript(
        &self,
        _path: &Path,
        _id: &str,
        _label: Option<String>,
        _model: Option<String>,
        _limit: usize,
    ) -> Result<Option<Transcript>, HostError> {
        unimplemented!("not exercised by Execution")
    }
    fn session_title(&self, _path: &Path) -> Option<String> {
        unimplemented!("not exercised by Execution")
    }
    fn external_sessions(
        &self,
        _repos: &[(String, PathBuf)],
        _agents: &[AgentSession],
        _app: &AppRuns,
        _now: i64,
    ) -> Result<ExternalSessions, HostError> {
        unimplemented!("not exercised by Execution")
    }
}

struct FakeChats;

impl ChatRuntime for FakeChats {
    fn send(&self, _chat_id: &str, _project_id: &str, _session_id: Option<&str>, _spec: ChatSpec, _text: &str) -> Result<(), HostError> {
        unimplemented!("not exercised by Execution")
    }
    fn respond(&self, _chat_id: &str, _request_id: &str, _allow: bool, _message: Option<&str>) -> Result<(), HostError> {
        unimplemented!("not exercised by Execution")
    }
    fn interrupt(&self, _chat_id: &str) -> Result<(), HostError> {
        unimplemented!("not exercised by Execution")
    }
    fn stop(&self, _chat_id: &str) {}
    fn stop_project(&self, _project_id: &str) {}
    fn live(&self, _chat_id: &str) -> ChatLive {
        unimplemented!("not exercised by Execution")
    }
    fn start_reaper(&self) {}
}

struct Fx {
    _t: TempDir,
    exec: Arc<Execution>,
    db: Db,
    env: Env,
    git: Arc<FakeGit>,
    claude: Arc<FakeClaude>,
}

fn fx(name: &str) -> Fx {
    let t = TempDir::new(name);
    let db = Db::open_in_memory().unwrap();
    let git = Arc::new(FakeGit::default());
    let claude = Arc::new(FakeClaude::default());
    let env = Env {
        data_dir: t.0.join("data"),
        worktrees_root: t.0.join("wt"),
        claude_dir: None,
        plans: Arc::new(HostPlanFiles),
        fs: Arc::new(HostLocalFs),
        git: git.clone() as Arc<dyn Git>,
        claude_config: Arc::new(HostClaudeConfig),
    };
    let core = Arc::new(Core {
        db: db.clone(),
        env: env.clone(),
        rt: crate::testutil::rt(),
        clock: Arc::new(nodal_host::adapters::SystemClock),
        claude: claude.clone() as Arc<dyn ClaudeCli>,
        sessions: Arc::new(FakeSessions),
        notifier: Arc::new(NoopNotifier),
        chats: Arc::new(FakeChats),
    });
    let exec = Execution::new(core);
    Fx { _t: t, exec, db, env, git, claude }
}

/// A project, an in-place repo (folder on disk) and a task with a text plan.
fn task_fixture(f: &Fx, repo_dir: &Path) -> Task {
    std::fs::create_dir_all(repo_dir).unwrap();
    let mut c = f.db.lock().unwrap();
    let p = ops::create_project(
        &c,
        &f.env,
        &NewProject { name: "Pay".into(), key: "PAY".into(), color: None, description: None, root_path: None },
        1,
    )
    .unwrap();
    let input: NewRepo = serde_json::from_value(json!({"path": "x", "defaultIsolation": "in_place"})).unwrap();
    let r = ops::add_repo(&c, &p.id, &input, repo_dir, 1).unwrap();
    let nt: NewTask = serde_json::from_value(json!({
        "projectId": p.id, "repoId": r.id, "title": "Logo", "plan": {"kind": "text", "text": "# Plan"}
    }))
    .unwrap();
    ops::create_task(&mut c, &f.env, &nt, 2).unwrap()
}

fn task_status(f: &Fx, id: &str) -> TaskStatus {
    tasks::get(&f.db.lock().unwrap(), id).unwrap().status
}

#[test]
fn worktree_status_reports_the_fake_git_state() {
    block_on(async {
        let f = fx("worktree-status");
        let task = task_fixture(&f, &f._t.0.join("web"));
        *f.git.status.lock().unwrap() = WorktreeStatus { exists: true, ahead: 2, unpushed: 1, dirty: true, ..Default::default() };
        let status = f.exec.worktree_status(task.id).await.unwrap();
        assert!(status.exists);
        assert_eq!(status.ahead, 2);
        assert_eq!(status.unpushed, 1);
        assert!(status.dirty);
    });
}

#[test]
fn launch_task_enqueues_a_queued_run_in_place() {
    block_on(async {
        let f = fx("launch-task");
        let task = task_fixture(&f, &f._t.0.join("web"));
        let run = f.exec.launch_task(task.id.clone(), None).await.unwrap();
        assert_eq!(run.status, RunStatus::Queued);
        assert_eq!(run.task_id.as_deref(), Some(task.id.as_str()));
        assert_eq!(task_status(&f, &task.id), TaskStatus::InProgress);
        // A second launch is rejected: the task already has a queued run.
        let err = f.exec.launch_task(task.id, None).await.unwrap_err();
        assert!(err.to_string().contains("already has a queued run"), "{err}");
    });
}

#[test]
fn cancel_run_dequeues_a_queued_run_and_frees_the_task() {
    block_on(async {
        let f = fx("cancel-queued");
        let task = task_fixture(&f, &f._t.0.join("web"));
        let run = f.exec.launch_task(task.id.clone(), None).await.unwrap();
        let canceled = f.exec.cancel_run(run.id.clone()).await.unwrap();
        assert_eq!(canceled.status, RunStatus::Canceled);
        assert_eq!(task_status(&f, &task.id), TaskStatus::Todo);
    });
}

#[test]
fn cancel_run_stops_a_launched_run_and_saves_the_patch() {
    block_on(async {
        let f = fx("cancel-launched");
        let task = task_fixture(&f, &f._t.0.join("web"));
        *f.git.diff.lock().unwrap() = ("diff --git a/x b/x\n+hi\n".into(), true);
        let run = {
            let c = f.db.lock().unwrap();
            let mut r = run_of(Executor::Claude, RunKind::Work, false);
            r.id = "u1".into();
            r.task_id = Some(task.id.clone());
            r.repo_id = Some(task.repo_id.clone());
            r.cwd = f._t.0.join("web").to_string_lossy().into_owned();
            r.isolation = Some(Isolation::InPlace);
            r.status = RunStatus::Launched;
            r.claude_run_id = Some("bg-abcd1234".into());
            r.session_id = Some("sess-1".into());
            qruns::insert(&c, &r).unwrap();
            r
        };
        let canceled = f.exec.cancel_run(run.id.clone()).await.unwrap();
        assert_eq!(canceled.status, RunStatus::Canceled);
        assert_eq!(canceled.tokens, Some(42), "session_tokens from the fake SessionFiles");
        assert!(canceled.error.unwrap().contains("Stopped by the user"));
        assert_eq!(f.claude.stopped.lock().unwrap().as_slice(), ["bg-abcd1234"]);
        let patch = f.env.data_dir.join("runs").join(&run.id).join("stopped.patch");
        assert!(patch.is_file(), "the partial patch was written to disk");
    });
}

#[test]
fn confirm_run_requeues_a_legacy_queued_run_from_the_task() {
    block_on(async {
        let f = fx("confirm-legacy");
        let task = task_fixture(&f, &f._t.0.join("web"));
        let legacy = {
            let c = f.db.lock().unwrap();
            let mut r = run_of(Executor::Claude, RunKind::Work, false);
            r.id = "legacy1".into();
            r.task_id = Some(task.id.clone());
            r.repo_id = Some(task.repo_id.clone());
            r.status = RunStatus::Queued;
            r.legacy_label = Some("Logo".into());
            qruns::insert(&c, &r).unwrap();
            r
        };
        let run = f.exec.confirm_run(legacy.id.clone()).await.unwrap();
        assert_eq!(run.status, RunStatus::Queued);
        assert!(run.legacy_label.is_none());
        assert_eq!(task_status(&f, &task.id), TaskStatus::InProgress);
    });
}

#[test]
fn reorder_queue_changes_the_order() {
    block_on(async {
        let f = fx("reorder");
        let task_a = task_fixture(&f, &f._t.0.join("web"));
        let task_b = {
            let mut c = f.db.lock().unwrap();
            let nt: NewTask = serde_json::from_value(json!({
                "projectId": task_a.project_id, "repoId": task_a.repo_id, "title": "Other",
                "plan": {"kind": "text", "text": "# Plan"}
            }))
            .unwrap();
            ops::create_task(&mut c, &f.env, &nt, 3).unwrap()
        };
        let a = f.exec.launch_task(task_a.id.clone(), None).await.unwrap();
        let b = f.exec.launch_task(task_b.id.clone(), None).await.unwrap();
        f.exec.reorder_queue(vec![b.id.clone(), a.id.clone()]).await.unwrap();
        let queue = f.exec.list_queue().await.unwrap();
        assert_eq!(queue.len(), 2);
        assert!(queue[0].queue_position < queue[1].queue_position);
        assert_eq!(queue[0].id, b.id);
        assert_eq!(queue[1].id, a.id);
    });
}

#[test]
fn run_diff_reports_the_fake_patch() {
    block_on(async {
        let f = fx("run-diff");
        let mut task = task_fixture(&f, &f._t.0.join("web"));
        let cwd = f._t.0.join("web").to_string_lossy().into_owned();
        task.worktree = Some(WorktreeRef { path: cwd.clone(), branch: "nodal/pay-1".into(), base: "main".into() });
        {
            let c = f.db.lock().unwrap();
            tasks::update(&c, &task).unwrap();
        }
        *f.git.diff.lock().unwrap() = ("diff --git a/x b/x\n+hi\n".into(), false);
        *f.git.branch.lock().unwrap() = Some("main".into());
        *f.git.commits.lock().unwrap() = vec![CommitInfo {
            sha: "a".repeat(40),
            short_sha: "aaaaaaa".into(),
            subject: "hi".into(),
            author: "me".into(),
            at: 1,
        }];
        let run = {
            let c = f.db.lock().unwrap();
            let mut r = run_of(Executor::Claude, RunKind::Work, false);
            r.id = "u2".into();
            r.task_id = Some(task.id.clone());
            r.repo_id = Some(task.repo_id.clone());
            r.cwd = cwd;
            r.isolation = Some(Isolation::Worktree);
            r.status = RunStatus::Finished;
            qruns::insert(&c, &r).unwrap();
            r
        };
        let diff = f.exec.run_diff(run.id).await.unwrap();
        assert_eq!(diff.branch.as_deref(), Some("main"));
        assert_eq!(diff.commits.len(), 1);
        assert!(!diff.live);
        assert_eq!(diff.patch, "diff --git a/x b/x\n+hi\n");
    });
}

#[test]
fn work_summary_counts_the_queued_run() {
    block_on(async {
        let f = fx("summary");
        let task = task_fixture(&f, &f._t.0.join("web"));
        f.exec.launch_task(task.id.clone(), None).await.unwrap();
        let summary = f.exec.work_summary(None).await.unwrap();
        assert_eq!(summary.queued, 1);
        assert_eq!(summary.pump_error, None);
    });
}

#[test]
fn merge_worktree_marks_the_task_done() {
    block_on(async {
        let f = fx("merge");
        let task = task_fixture(&f, &f._t.0.join("web"));
        let c = f.db.lock().unwrap();
        let mut t = tasks::get(&c, &task.id).unwrap();
        t.worktree = Some(WorktreeRef { path: f._t.0.join("web").to_string_lossy().into_owned(), branch: "nodal/pay-1".into(), base: "main".into() });
        tasks::update(&c, &t).unwrap();
        drop(c);
        let report = f
            .exec
            .merge_worktree(task.id.clone(), MergeInput { squash: false, push: false, cleanup: false })
            .await
            .unwrap();
        assert!(matches!(report.outcome, MergeOutcome::Merged { .. }));
        assert_eq!(report.task.status, TaskStatus::Done);
        assert!(report.push_error.is_none());
        assert!(report.cleanup_error.is_none());
        // The project row still exists (sanity: `apply_task_transition` didn't error out).
        let _ = projects::get(&f.db.lock().unwrap(), &task.project_id).unwrap();
        let _ = repos::get(&f.db.lock().unwrap(), &task.repo_id).unwrap();
    });
}
