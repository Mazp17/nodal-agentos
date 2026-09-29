#![allow(clippy::disallowed_methods)] // builds fixture folders/files directly on disk

use std::path::Path;

use serde_json::json;

use nodal_domain::board::dto::{NewProject, NewRepo, NewTask};
use nodal_domain::execution::prompts::review_isolation;
use nodal_domain::execution::report::ReportStatus;
use nodal_domain::model::*;
use nodal_domain::testutil::run_of;
use nodal_host::testutil::TempDir;
use nodal_store::board::tasks;
use nodal_store::Db;

use crate::board::ops;
use crate::execution::cleaning::{Cleaning, CLEANING_ERR};
use crate::execution::enqueue::{confirm_legacy, enqueue_review, enqueue_work};

use super::*;

struct Fx {
    _t: TempDir,
    env: Env,
    db: Db,
    task: Task,
}

/// Project, repo (folder with `.claude/agents/code-reviewer.md`) and a task with a plan.
fn fx(name: &str) -> Fx {
    let t = TempDir::new(name);
    let repo_dir = t.0.join("web");
    std::fs::create_dir_all(repo_dir.join(".claude/agents")).unwrap();
    std::fs::write(
        repo_dir.join(".claude/agents/code-reviewer.md"),
        "---\nname: code-reviewer\ntools: Read\n---\n",
    )
    .unwrap();
    let env = crate::testutil::env_for(&t.0, Some(t.0.join("claude")));
    let db = Db::open_in_memory().unwrap();
    let task = {
        let mut c = db.lock().unwrap();
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
        let input: NewRepo =
            serde_json::from_value(json!({"path": "x", "defaultIsolation": "in_place"})).unwrap();
        let r = ops::add_repo(&c, &p.id, &input, &repo_dir, 1).unwrap();
        let nt: NewTask = serde_json::from_value(json!({
            "projectId": p.id, "repoId": r.id, "title": "Logo", "plan": {"kind": "text", "text": "# Plan"}
        }))
        .unwrap();
        ops::create_task(&mut c, &env, &nt, 2).unwrap()
    };
    Fx { _t: t, env, db, task }
}

fn launched_run(f: &Fx, executor: Executor, kind: RunKind, review: bool) -> Run {
    let c = f.db.lock().unwrap();
    let mut r = run_of(executor, kind, review);
    r.id = format!("u-{}", qruns::next_queue_position(&c).unwrap());
    r.task_id = Some(f.task.id.clone());
    r.repo_id = Some(f.task.repo_id.clone());
    r.isolation = Some(Isolation::InPlace);
    qruns::insert(&c, &r).unwrap();
    r
}

fn task_status(f: &Fx) -> TaskStatus {
    tasks::get(&f.db.lock().unwrap(), &f.task.id).unwrap().status
}

fn done_readout(msg: &str) -> SessionReadout {
    SessionReadout {
        detail: None,
        last_message: Some(msg.into()),
        blocker: None,
    }
}

#[test]
fn agent_run_then_reviewer_pass_and_fail() {
    let f = fx("pump-review");
    let agent = Executor::Agent {
        name: "frontend-developer".into(),
        source: AgentSource::User,
    };
    let work = launched_run(&f, agent, RunKind::Work, true);
    tasks::set_status(&f.db.lock().unwrap(), &f.task.id, TaskStatus::InProgress, 3).unwrap();
    let mut end = read_end(
        &work,
        EndSignal::Done,
        &done_readout("{\"status\":\"done\",\"summary\":\"all set\"}"),
        false,
    );
    end.tokens = Some(1234);
    let reviewer = apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 10)
        .unwrap()
        .unwrap();
    assert_eq!(reviewer.kind, RunKind::Review);
    assert_eq!(reviewer.parent_run_id.as_deref(), Some(work.id.as_str()));
    assert_eq!(
        reviewer.executor,
        Executor::Agent {
            name: "code-reviewer".into(),
            source: AgentSource::Repo
        }
    );
    assert!(reviewer
        .prompt
        .contains("frontend-developer did the work and reported: all set"));
    assert_eq!(
        task_status(&f),
        TaskStatus::InProgress,
        "stays In Progress while reviewing"
    );
    let saved = qruns::get(&f.db.lock().unwrap(), &work.id).unwrap();
    assert_eq!(
        (saved.status, saved.outcome, saved.summary.as_deref()),
        (
            RunStatus::Finished,
            Some(RunOutcome::Green),
            Some("all set")
        )
    );
    assert_eq!(
        saved.tokens,
        Some(1234),
        "the transcript tokens are stored on the run"
    );
    // Applying twice doesn't duplicate (the run is no longer launched).
    assert!(
        apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 11)
            .unwrap()
            .is_none()
    );

    // The reviewer fails → Blocked, no retry.
    let mut rev = reviewer.clone();
    rev.status = RunStatus::Launched;
    qruns::update(&f.db.lock().unwrap(), &rev).unwrap();
    let end = read_end(
        &rev,
        EndSignal::Done,
        &done_readout("{\"verdict\":\"fail\",\"unmet\":[\"c1\"],\"nits\":[]}"),
        false,
    );
    assert!(
        apply_end(&f.db, &f.env, &rev, &end, RunStatus::Finished, None, 12)
            .unwrap()
            .is_none()
    );
    assert_eq!(task_status(&f), TaskStatus::Blocked);
    let saved = qruns::get(&f.db.lock().unwrap(), &rev.id).unwrap();
    assert_eq!(saved.verdict.unwrap().unmet, ["c1"]);
    assert!(qruns::pending(&f.db.lock().unwrap()).unwrap().is_empty());

    // Pass → In Review.
    let rev2 = launched_run(
        &f,
        Executor::Agent {
            name: "code-reviewer".into(),
            source: AgentSource::Repo,
        },
        RunKind::Review,
        false,
    );
    let end = read_end(
        &rev2,
        EndSignal::Done,
        &done_readout("{\"verdict\":\"pass\",\"unmet\":[],\"nits\":[\"n\"]}"),
        false,
    );
    apply_end(&f.db, &f.env, &rev2, &end, RunStatus::Finished, None, 13).unwrap();
    assert_eq!(task_status(&f), TaskStatus::InReview);
    let _ = ReportStatus::Done;
}

#[test]
fn task_closed_by_hand_gets_no_reviewer_nor_comment() {
    let f = fx("pump-closed");
    link_task(&f);
    let work = launched_run(&f, Executor::Claude, RunKind::Work, true);
    tasks::set_status(&f.db.lock().unwrap(), &f.task.id, TaskStatus::Done, 3).unwrap();
    let end = read_end(
        &work,
        EndSignal::Done,
        &done_readout("{\"status\":\"done\"}"),
        false,
    );
    assert!(
        apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 10)
            .unwrap()
            .is_none()
    );
    assert_eq!(task_status(&f), TaskStatus::Done);
    assert!(outbox_kinds(&f).is_empty());
    assert_eq!(
        qruns::get(&f.db.lock().unwrap(), &work.id).unwrap().status,
        RunStatus::Finished
    );
}

#[test]
fn missing_reviewer_blocks_the_task() {
    let f = fx("pump-no-reviewer");
    {
        let c = f.db.lock().unwrap();
        let mut s = rows::load_settings(&c).unwrap();
        s.reviewer = "no-such-reviewer".into();
        drop(c);
        rows::save_settings(&mut f.db.lock().unwrap(), &s).unwrap();
    }
    let work = launched_run(&f, Executor::Claude, RunKind::Work, true);
    let end = read_end(
        &work,
        EndSignal::Done,
        &done_readout("{\"status\":\"done\"}"),
        false,
    );
    // The repo's reviewer isn't configured and the global one doesn't exist.
    assert!(
        apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 10)
            .unwrap()
            .is_none()
    );
    assert_eq!(task_status(&f), TaskStatus::Blocked);
    let saved = qruns::get(&f.db.lock().unwrap(), &work.id).unwrap();
    assert!(saved.error.unwrap().contains("Couldn't start the reviewer"));
}

fn link_task(f: &Fx) {
    let c = f.db.lock().unwrap();
    let mut map = StateMap {
        confirmed_at: Some(1),
        ..Default::default()
    };
    map.push.insert(TaskStatus::InReview, Some("s-rev".into()));
    map.push.insert(TaskStatus::Blocked, Some("s-blk".into()));
    let link = SourceLink {
        id: "l1".into(),
        project_id: f.task.project_id.clone(),
        provider: "linear".into(),
        scope: ScopeRef {
            kind: "team".into(),
            id: "tm".into(),
            name: "Eng".into(),
        },
        default_repo_id: None,
        repo_rules: vec![],
        state_map: map,
        auto_import: false,
        created_at: 1,
        last_synced_at: None,
        last_sync_error: None,
        pending_state_changes: None,
    };
    rows::insert_source_link(&c, &link).unwrap();
    c.execute(
        "UPDATE tasks SET src_provider='linear', src_link_id='l1', src_external_id='e1', src_identifier='ENG-1', src_url='https://linear.app/acme/issue/ENG-1' WHERE id=?1",
        [&f.task.id],
    )
    .unwrap();
}

fn outbox_kinds(f: &Fx) -> Vec<String> {
    let c = f.db.lock().unwrap();
    let mut stmt = c
        .prepare("SELECT kind FROM sync_outbox ORDER BY id")
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.map(|r| r.unwrap()).collect()
}

#[test]
fn outbox_written_in_the_same_transaction_unless_manages_source() {
    let f = fx("pump-outbox");
    link_task(&f);
    let wf = Executor::Workflow {
        name: "linear-issue".into(),
    };
    let run = launched_run(&f, wf.clone(), RunKind::Work, false);
    let ro = SessionReadout {
        detail: None,
        last_message: None,
        blocker: None,
    };
    let end = read_end(&run, EndSignal::Done, &ro, true);
    apply_end(
        &f.db,
        &f.env,
        &run,
        &end,
        RunStatus::Finished,
        Some("linear"),
        10,
    )
    .unwrap();
    assert_eq!(task_status(&f), TaskStatus::Blocked);
    assert!(
        outbox_kinds(&f).is_empty(),
        "managesSource: the app doesn't write to the provider"
    );

    let run = launched_run(&f, Executor::Claude, RunKind::Work, false);
    let end = read_end(
        &run,
        EndSignal::Done,
        &done_readout("{\"status\":\"done\",\"summary\":\"ok\"}"),
        false,
    );
    apply_end(&f.db, &f.env, &run, &end, RunStatus::Finished, None, 11).unwrap();
    assert_eq!(task_status(&f), TaskStatus::InReview);
    assert_eq!(outbox_kinds(&f), ["set_state", "comment"]);
}

#[test]
fn back_to_todo_is_pushed_to_the_provider() {
    let f = fx("pump-todo");
    link_task(&f);
    let c = f.db.lock().unwrap();
    tasks::set_status(&c, &f.task.id, TaskStatus::InProgress, 3).unwrap();
    let t = tasks::get(&c, &f.task.id).unwrap();
    // What cancelling the task's only queued run does.
    ops::apply_task_transition(&c, &t, Some(TaskStatus::Todo), None, None, 4).unwrap();
    let state: String = c
        .query_row(
            "SELECT payload_json FROM sync_outbox WHERE kind = 'set_state'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(state.contains("todo"), "{state}");
}

#[test]
fn friendly_trust_error() {
    let root = Path::new("/Users/me/.nodal/worktrees");
    let e = friendly_launch_error(
        "`claude --bg` exited ...: Workspace not trusted. Run `claude` in x",
        "/Users/me/.nodal/worktrees/web/pay-1",
        root,
    );
    assert!(e.contains("it covers every worktree inside"));
    let e = friendly_launch_error("Workspace not trusted.", "/Users/me/Code/web", root);
    assert!(e.contains("Open Terminal in that folder"));
    assert_eq!(friendly_launch_error("other", "/x", root), "other");
}

#[test]
fn step_comment_lists_findings_and_work_branch() {
    let reviewer = Executor::Agent {
        name: "code-reviewer".into(),
        source: AgentSource::User,
    };
    let review = run_of(reviewer, RunKind::Review, false);
    let readout = SessionReadout {
        detail: None,
        last_message: Some(
            r#"{"verdict":"fail","unmet":["c1"],"nits":["n1"],"summary":"c1 is missing"}"#.into(),
        ),
        blocker: None,
    };
    let end = read_end(&review, EndSignal::Done, &readout, false);
    let mut work = run_of(Executor::Claude, RunKind::Work, true);
    work.branch = Some("nodal/pay-1-x".into());
    let c = step_comment(&review, &end, None, Some(&work), TaskStatus::Blocked);
    assert_eq!(
        c,
        "**Nodal** · Blocked · code-reviewer\n\nBranch: `nodal/pay-1-x`\n\nc1 is missing\n\n**Unmet criteria**\n- c1\n\n**Nits**\n- n1"
    );
}

#[test]
fn confirming_a_migrated_queued_run_requeues_it_from_the_task() {
    let f = fx("confirm-legacy");
    let legacy = {
        let c = f.db.lock().unwrap();
        let mut r = run_of(Executor::Claude, RunKind::Work, false);
        r.id = "lrun_1".into();
        r.task_id = Some(f.task.id.clone());
        r.repo_id = Some(f.task.repo_id.clone());
        r.status = RunStatus::Queued;
        r.prompt = "/plan-task old".into();
        r.legacy_label = Some("Logo".into());
        qruns::insert(&c, &r).unwrap();
        r
    };
    let cleaning = Cleaning::default();
    let run = confirm_legacy(&f.db, &f.env, &cleaning, &legacy.id, 10).unwrap();
    let c = f.db.lock().unwrap();
    assert_eq!(
        (run.status, run.legacy_label.as_deref()),
        (RunStatus::Queued, None)
    );
    assert_ne!(
        run.prompt, legacy.prompt,
        "the prompt is rebuilt from the task"
    );
    let old = qruns::get(&c, &legacy.id).unwrap();
    assert_eq!(old.status, RunStatus::Canceled);
    assert!(old.error.unwrap().contains(&run.id));
    assert_eq!(
        tasks::get(&c, &f.task.id).unwrap().status,
        TaskStatus::InProgress
    );
    // No longer awaiting confirmation: confirming again fails.
    drop(c);
    assert!(confirm_legacy(&f.db, &f.env, &cleaning, &legacy.id, 11).is_err());
}

fn legacy_queued(f: &Fx) -> Run {
    let c = f.db.lock().unwrap();
    let mut r = run_of(Executor::Claude, RunKind::Work, false);
    r.id = "lrun_2".into();
    r.task_id = Some(f.task.id.clone());
    r.repo_id = Some(f.task.repo_id.clone());
    r.status = RunStatus::Queued;
    r.legacy_label = Some("Logo".into());
    qruns::insert(&c, &r).unwrap();
    r
}

#[test]
fn confirm_legacy_is_atomic_when_the_requeue_fails() {
    let f = fx("confirm-atomic");
    let legacy = legacy_queued(&f);
    let cleaning = Cleaning::default();
    let _guard = cleaning.mark(&f.task.id).unwrap();
    let err = confirm_legacy(&f.db, &f.env, &cleaning, &legacy.id, 10).unwrap_err();
    assert_eq!(err, CLEANING_ERR);
    let c = f.db.lock().unwrap();
    let old = qruns::get(&c, &legacy.id).unwrap();
    assert!(
        queue::awaiting_confirmation(&old),
        "still awaiting confirmation"
    );
    assert_eq!(qruns::pending(&c).unwrap().len(), 1, "nothing was enqueued");
}

#[test]
fn enqueue_is_rejected_while_the_worktree_is_being_cleaned() {
    let f = fx("enqueue-cleaning");
    let cleaning = Cleaning::default();
    let input = nodal_domain::board::dto::LaunchInput::default();
    {
        let _guard = cleaning.mark(&f.task.id).unwrap();
        assert!(cleaning.mark(&f.task.id).is_none(), "one cleanup at a time");
        let err = enqueue_work(&f.db, &f.env, &cleaning, &f.task.id, &input, false, 10)
            .unwrap_err();
        assert_eq!(err, CLEANING_ERR);
        let err =
            enqueue_review(&f.db, &f.env, &cleaning, &f.task.id, None, 10).unwrap_err();
        assert_eq!(err, CLEANING_ERR);
    }
    // Once the mark is released, it enqueues; and a second attempt sees the pending one.
    let run =
        enqueue_work(&f.db, &f.env, &cleaning, &f.task.id, &input, false, 11).unwrap();
    assert!(run.queue_position > 0.0);
    assert_eq!(task_status(&f), TaskStatus::InProgress);
    let err =
        enqueue_work(&f.db, &f.env, &cleaning, &f.task.id, &input, false, 12).unwrap_err();
    assert!(err.contains("already has a queued run"), "{err}");
}

#[test]
fn failed_launch_blocks_the_task_with_a_comment() {
    let f = fx("pump-launch-fail");
    link_task(&f);
    let cleaning = Cleaning::default();
    let input = nodal_domain::board::dto::LaunchInput::default();
    let run =
        enqueue_work(&f.db, &f.env, &cleaning, &f.task.id, &input, false, 10).unwrap();
    assert_eq!(task_status(&f), TaskStatus::InProgress);
    let mut c = f.db.lock().unwrap();
    // Without `launching` it does nothing.
    assert!(!fail_launch(&mut c, &run.id, "boom", 11).unwrap());
    assert!(qruns::transition(&c, &run.id, RunStatus::Queued, RunStatus::Launching).unwrap());
    assert!(fail_launch(&mut c, &run.id, "Workspace not trusted.", 12).unwrap());
    let saved = qruns::get(&c, &run.id).unwrap();
    assert_eq!(
        (saved.status, saved.error.as_deref()),
        (RunStatus::Failed, Some("Workspace not trusted."))
    );
    drop(c);
    assert_eq!(task_status(&f), TaskStatus::Blocked);
    // The In Progress set_state is replaced by the Blocked one; plus the comment.
    assert_eq!(outbox_kinds(&f), ["set_state", "comment"]);
}

#[test]
fn stale_launching_runs_are_failed_and_adopted_ones_launched() {
    let f = fx("pump-stale");
    let a = launched_run(&f, Executor::Claude, RunKind::Work, false);
    let b = launched_run(&f, Executor::Claude, RunKind::Review, false);
    let mut c = f.db.lock().unwrap();
    for r in [&a, &b] {
        let mut r = qruns::get(&c, &r.id).unwrap();
        r.status = RunStatus::Launching;
        qruns::update(&c, &r).unwrap();
    }
    assert!(mark_launched(&c, &b.id, "bg1", Some("sess-1".into()), 5).unwrap());
    let saved = qruns::get(&c, &b.id).unwrap();
    assert_eq!(
        (
            saved.status,
            saved.claude_run_id.as_deref(),
            saved.session_id.as_deref()
        ),
        (RunStatus::Launched, Some("bg1"), Some("sess-1"))
    );
    assert_eq!(
        fail_stale_launches(&mut c, NOTE_STALE_LAUNCH, 6).unwrap(),
        1
    );
    assert_eq!(qruns::get(&c, &a.id).unwrap().status, RunStatus::Failed);
    assert_eq!(
        fail_stale_launches(&mut c, NOTE_STALE_LAUNCH, 7).unwrap(),
        0
    );
}

#[test]
fn reviewer_isolation_follows_its_cwd() {
    assert_eq!(
        review_isolation("/r/web", "/r/web/"),
        Isolation::InPlace
    );
    assert_eq!(
        review_isolation("/wt/web/pay-1", "/r/web"),
        Isolation::Worktree
    );
    // The work ran "in worktree" but the task has no live one: the reviewer runs in the
    // repo folder and takes its lock.
    let f = fx("review-isolation");
    let mut work = launched_run(&f, Executor::Claude, RunKind::Work, true);
    work.isolation = Some(Isolation::Worktree);
    let end = read_end(
        &work,
        EndSignal::Done,
        &done_readout("{\"status\":\"done\"}"),
        false,
    );
    let reviewer = apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 10)
        .unwrap()
        .unwrap();
    assert_eq!(reviewer.isolation, Some(Isolation::InPlace));
}
