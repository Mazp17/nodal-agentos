//! The periodic queue pass: fills in sessionIds, detects runs that finished (they leave
//! working/blocked in `claude agents`), reads their result, applies the transition (with
//! the outbox and the reviewer in the same transaction) and launches whatever fits.

use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;

use crate::db::queries::{repos, runs as qruns};
use crate::db::{rows, with_db, Db};
use crate::domain::{Executor, Run, RunKind, RunStatus, Task, TaskStatus};
use crate::providers::plan::{closing_comment, ClosingInfo};
use crate::runs::{self, SessionReadout};
use crate::util::{blocking, now_ms};

use super::queue::{self, EndSignal};
use super::transitions::{decide, executor_label, read_end, RunEnd};
use super::{executors, launch, ops, Env, Inner};

/// Launch error message with the concrete way out for known cases.
pub fn friendly_launch_error(e: &str, cwd: &str, worktrees_root: &Path) -> String {
    if e.contains("Workspace not trusted") {
        let hint = if Path::new(cwd).starts_with(worktrees_root) {
            format!(
                " Task worktrees live in {}: open Terminal there, run `claude` once and accept the trust prompt (it covers every worktree inside).",
                worktrees_root.display()
            )
        } else {
            " Open Terminal in that folder, run `claude` once and accept the trust prompt.".to_string()
        };
        return format!("{e}{hint}");
    }
    e.to_string()
}

/// Closes a run that finished (`final_status`: Finished, or Canceled if the user stopped it):
/// saves its result, moves the task, writes the outbox and enqueues the reviewer if needed,
/// all in one transaction. If the run is no longer `launched` (another pass closed it), it
/// does nothing. Returns the enqueued reviewer, if any.
///
/// The reviewer is built (disk and git) without holding the database; the transaction
/// re-reads the task and, if it was closed by hand in the meantime, doesn't enqueue it.
pub fn apply_end(
    db: &Db,
    env: &Env,
    run: &Run,
    end: &RunEnd,
    final_status: RunStatus,
    manages_source: Option<&str>,
    now: i64,
) -> Result<Option<Run>, String> {
    let mut decision = decide(run, end);
    let is_closed = |t: &Task| matches!(t.status, TaskStatus::Done | TaskStatus::Canceled);
    // 1. Read.
    let (task, inputs) = {
        let conn = launch::lock(db);
        let task = match &run.task_id {
            Some(id) => rows::get_task(&conn, id)?,
            None => None,
        };
        let inputs = match &task {
            Some(t) if decision.enqueue_review && !is_closed(t) => Some(launch::ReviewInputs::read(&conn, t)),
            _ => None,
        };
        (task, inputs)
    };
    // 2. The reviewer, without the database.
    let mut note = end.note.clone();
    // The reviewer sees the run already with its result (summary, branch).
    let mut view = run.clone();
    view.summary = end.summary.clone();
    view.pr_url = end.pr.clone();
    view.branch = end.branch.clone();
    view.outcome = Some(end.outcome);
    let built = match (&task, inputs) {
        (Some(t), Some(inputs)) => {
            Some(inputs.and_then(|i| launch::build_review(env, &i, t, Some(&view), None, None, now)))
        }
        _ => None,
    };
    let mut reviewer = match built {
        Some(Ok(r)) => Some(r),
        Some(Err(e)) => {
            // Without a reviewer there's no gate: the task is left blocked with the reason.
            decision.enqueue_review = false;
            decision.task_status = Some(TaskStatus::Blocked);
            decision.closing = true;
            note = Some(format!("Couldn't start the reviewer: {e}"));
            None
        }
        None => None,
    };

    // 3. Transaction.
    let mut conn = launch::lock(db);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let current = qruns::get(&tx, &run.id)?;
    if current.status != RunStatus::Launched {
        return Ok(None);
    }
    let task = match &run.task_id {
        Some(id) => rows::get_task(&tx, id)?,
        None => None,
    };
    // A task closed by hand (Done/Canceled) while it ran: no reviewer and no comment.
    if task.as_ref().is_some_and(is_closed) {
        reviewer = None;
        decision.closing = false;
    }
    let mut done = current;
    done.status = final_status;
    done.finished_at = Some(now);
    done.outcome = Some(end.outcome);
    done.summary = end.summary.clone();
    done.pr_url = end.pr.clone();
    done.branch = end.branch.clone();
    done.verdict = end.verdict.clone();
    done.error = note.clone();
    if end.tokens.is_some() {
        done.tokens = end.tokens;
    }
    qruns::update(&tx, &done)?;
    if let Some(t) = &task {
        let comment = match (decision.closing, decision.task_status) {
            (true, Some(status)) => {
                let work = match run.kind {
                    RunKind::Review => run.parent_run_id.as_deref().map(|p| qruns::get(&tx, p)).transpose()?,
                    RunKind::Work => None,
                };
                Some(step_comment(&done, end, note.as_deref(), work.as_ref(), status))
            }
            _ => None,
        };
        ops::apply_task_transition(&tx, t, decision.task_status, comment, manages_source, now)?;
    }
    if let Some(r) = &mut reviewer {
        r.queue_position = qruns::next_queue_position(&tx)?;
        qruns::insert(&tx, r)?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(reviewer)
}

/// Closing comment of a step. `work` is the step's work run (PR/branch when the reviewer
/// closes it).
fn step_comment(run: &Run, end: &RunEnd, note: Option<&str>, work: Option<&Run>, status: TaskStatus) -> String {
    let label = executor_label(&run.executor);
    closing_comment(&ClosingInfo {
        status: Some(status),
        executor: Some(&label),
        summary: end.summary.as_deref(),
        pr_url: end.pr.as_deref().or(work.and_then(|w| w.pr_url.as_deref())),
        branch: end.branch.as_deref().or(work.and_then(|w| w.branch.as_deref())),
        verdict: end.verdict.as_ref(),
        note,
    })
}

pub const NOTE_APP_CLOSED: &str = "The app closed while the run was launching; check the runs list before retrying.";
pub const NOTE_STALE_LAUNCH: &str =
    "The launch result couldn't be saved; check the runs list (the session may be running) before retrying.";
/// Retries when saving a launch's result.
const RECORD_TRIES: u32 = 3;
const RECORD_RETRY: std::time::Duration = std::time::Duration::from_millis(500);

/// A run that never launched moves to `failed` with the reason and, if its task was left In
/// Progress with no other pending runs, the task moves to Blocked with a comment (like a
/// step's closing), all in one transaction. `false` if the run was no longer `launching`.
pub fn fail_launch(conn: &mut Connection, run_id: &str, error: &str, now: i64) -> Result<bool, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut r = qruns::get(&tx, run_id)?;
    if r.status != RunStatus::Launching {
        return Ok(false);
    }
    r.status = RunStatus::Failed;
    r.finished_at = Some(now);
    r.error = Some(error.to_string());
    qruns::update(&tx, &r)?;
    if let Some(t) = r.task_id.as_deref().map(|id| rows::get_task(&tx, id)).transpose()?.flatten() {
        if t.status == TaskStatus::InProgress && qruns::pending_for_task(&tx, &t.id)?.is_empty() {
            let label = executor_label(&r.executor);
            let note = format!("Couldn't launch: {error}");
            let comment = closing_comment(&ClosingInfo {
                status: Some(TaskStatus::Blocked),
                executor: Some(&label),
                summary: None,
                pr_url: None,
                branch: None,
                verdict: None,
                note: Some(&note),
            });
            ops::apply_task_transition(&tx, &t, Some(TaskStatus::Blocked), Some(comment), None, now)?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(true)
}

/// The launch went through (or the session was adopted): `launched` with its id. `false` if
/// the run was no longer `launching`.
pub fn mark_launched(conn: &Connection, run_id: &str, claude_id: &str, session: Option<String>, now: i64) -> Result<bool, String> {
    let mut r = qruns::get(conn, run_id)?;
    if r.status != RunStatus::Launching {
        return Ok(false);
    }
    r.status = RunStatus::Launched;
    r.claude_run_id = Some(claude_id.to_string());
    r.session_id = session.or(r.session_id);
    r.launched_at = Some(now);
    qruns::update(conn, &r)?;
    Ok(true)
}

/// Closes with `fail_launch` the runs left `launching`: only the queue pass sets that and
/// clears it in the same pass, holding its turn, so one seen when a pass starts (or when the
/// app starts) is orphaned. Returns how many it closed.
pub fn fail_stale_launches(conn: &mut Connection, note: &str, now: i64) -> Result<usize, String> {
    let mut n = 0;
    for r in qruns::launching(conn)? {
        if fail_launch(conn, &r.id, note, now)? {
            n += 1;
        }
    }
    Ok(n)
}

/// Saves a launch's result, with retries: if it isn't saved, the run stays `launching` and
/// the next pass closes it (`fail_stale_launches`).
async fn record_launch(inner: &Arc<Inner>, run: &Run, outcome: Result<(String, Option<String>), String>) -> Result<(), String> {
    let root = inner.env.worktrees_root.clone();
    let mut last = String::new();
    for attempt in 0..RECORD_TRIES {
        if attempt > 0 {
            tokio::time::sleep(RECORD_RETRY).await;
        }
        let (id, cwd, outcome, root) = (run.id.clone(), run.cwd.clone(), outcome.clone(), root.clone());
        let saved = with_db(&inner.db, move |c| {
            let now = now_ms();
            match outcome {
                Ok((claude_id, session)) => mark_launched(c, &id, &claude_id, session, now),
                Err(e) => fail_launch(c, &id, &friendly_launch_error(&e, &cwd, &root), now),
            }
            .map_err(crate::db::DbError::Invalid)
        })
        .await;
        match saved {
            Ok(_) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("Couldn't save the launch of run {}: {last}", run.id))
}

/// After a `claude --bg` timeout: the session that started anyway, if it can be recognized
/// unambiguously in `claude agents` (`queue::adoptable`).
async fn adopt_after_timeout(inner: &Arc<Inner>, run: &Run, since: i64) -> Option<(String, Option<String>)> {
    let live = runs::list_runs().await.ok()?;
    let claimed: Vec<String> = with_db(&inner.db, |c| qruns::launched_refs(c))
        .await
        .ok()?
        .into_iter()
        .filter_map(|(id, _)| id)
        .collect();
    queue::adoptable(&run.cwd, since, &live, &claimed).map(|s| (s.id.clone(), Some(s.session_id.clone())))
}

/// `(reviews, managesSource)` of the run's workflow, from the on-disk catalog.
pub fn workflow_meta(env: &Env, repo_path: Option<&str>, executor: &Executor) -> (bool, Option<String>) {
    let Executor::Workflow { name } = executor else { return (false, None) };
    match executors::find_workflow(env.claude_dir.as_deref(), repo_path.map(Path::new), name) {
        Some(i) => (i.reviews, i.manages_source),
        None => (false, None),
    }
}

async fn finish_run(inner: &Arc<Inner>, run: Run, signal: EndSignal) -> Result<(), String> {
    let env = inner.env.clone();
    let repo_id = run.repo_id.clone();
    let repo_path = match repo_id {
        Some(id) => with_db(&inner.db, move |c| Ok(repos::get(c, &id).ok().map(|r| r.path))).await?,
        None => None,
    };
    let (session, cwd) = (run.session_id.clone(), run.cwd.clone());
    let (executor, env2) = (run.executor.clone(), env.clone());
    let (readout, (reviews, manages), tokens) = blocking(move || {
        // Workflows split the work into subagents with their own transcripts: no tokens.
        let tokens = match (&executor, &session) {
            (Executor::Workflow { .. }, _) | (_, None) => None,
            (_, Some(sid)) => runs::session_tokens(sid, &cwd),
        };
        let readout = match (signal, session) {
            (EndSignal::Done, Some(sid)) => runs::read_session(&sid, &cwd),
            _ => SessionReadout::default(),
        };
        Ok((readout, workflow_meta(&env2, repo_path.as_deref(), &executor), tokens))
    })
    .await?;
    let mut end = read_end(&run, signal, &readout, reviews);
    end.tokens = tokens;
    let now = now_ms();
    let db = inner.db.clone();
    blocking(move || apply_end(&db, &env, &run, &end, RunStatus::Finished, manages.as_deref(), now)).await?;
    Ok(())
}

/// One queue pass. If nothing is queued or launched, it doesn't query `claude agents`.
pub async fn pump(inner: &Arc<Inner>) -> Result<(), String> {
    let mut touched = false;
    let r = pump_pass(inner, &mut touched).await;
    *inner.pump_error.lock().unwrap_or_else(|p| p.into_inner()) = r.as_ref().err().cloned();
    if touched {
        use crate::events::Kind;
        inner.events.notify_all(&[Kind::Runs, Kind::Queue, Kind::Tasks], None);
    }
    r
}

/// `touched`: the pass changed some run (to notify the UI even if it fails afterwards).
async fn pump_pass(inner: &Arc<Inner>, touched: &mut bool) -> Result<(), String> {
    let _turn = inner.pump.lock().await;
    let stale = with_db(&inner.db, |c| {
        fail_stale_launches(c, NOTE_STALE_LAUNCH, now_ms()).map_err(crate::db::DbError::Invalid)
    })
    .await?;
    *touched |= stale > 0;
    let mut pending = with_db(&inner.db, |c| qruns::pending(c)).await?;
    if !queue::needs_tick(&pending) {
        return Ok(());
    }
    let live = runs::list_runs().await?;

    let changed: Vec<Run> =
        queue::fill_session_ids(&mut pending, &live).into_iter().map(|i| pending[i].clone()).collect();
    if !changed.is_empty() {
        *touched = true;
        with_db(&inner.db, move |c| {
            for r in &changed {
                c.execute(
                    "UPDATE runs SET session_id = ?2 WHERE id = ?1 AND session_id IS NULL",
                    rusqlite::params![r.id, r.session_id],
                )?;
            }
            Ok(())
        })
        .await?;
    }

    for (id, signal) in queue::ended(&pending, &live, now_ms()) {
        let Some(run) = pending.iter().find(|r| r.id == id).cloned() else { continue };
        *touched = true;
        if let Err(e) = finish_run(inner, run, signal).await {
            eprintln!("work: couldn't close run {id}: {e}");
        }
    }

    let (pending, settings) = with_db(&inner.db, |c| Ok((qruns::pending(c)?, rows::load_settings(c)?))).await?;
    for id in queue::next_to_launch(&pending, &live, settings.concurrency, now_ms()) {
        let claimed = {
            let id = id.clone();
            with_db(&inner.db, move |c| {
                if qruns::transition(c, &id, RunStatus::Queued, RunStatus::Launching)? {
                    Ok(Some(qruns::get(c, &id)?))
                } else {
                    Ok(None)
                }
            })
            .await?
        };
        let Some(run) = claimed else { continue };
        *touched = true;
        let tests = match run.kind {
            RunKind::Review => {
                let cwd = run.cwd.clone();
                blocking(move || Ok(launch::test_commands(Path::new(&cwd)))).await.unwrap_or_default()
            }
            RunKind::Work => Vec::new(),
        };
        let flags = launch::extra_flags(&run, &tests);
        let since = now_ms();
        let outcome = match runs::launch_bg(run.cwd.clone(), run.prompt.clone(), &launch::launch_options(&run), &flags).await {
            Ok(rr) => Ok((rr.id, None)),
            Err(e) if e.timed_out => adopt_after_timeout(inner, &run, since).await.ok_or(e.message),
            Err(e) => Err(e.message),
        };
        record_launch(inner, &run, outcome).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::db::queries::tasks;
    use crate::domain::*;
    use crate::util::paths::tests::TempDir;
    use crate::work::dto::{NewProject, NewRepo, NewTask};
    use crate::work::report::ReportStatus;
    use serde_json::json;

    struct Fx {
        _t: TempDir,
        env: Env,
        db: crate::db::Db,
        task: Task,
    }

    /// Project, repo (folder with `.claude/agents/code-reviewer.md`) and a task with a plan.
    fn fx(name: &str) -> Fx {
        let t = TempDir::new(name);
        let repo_dir = t.0.join("web");
        std::fs::create_dir_all(repo_dir.join(".claude/agents")).unwrap();
        std::fs::write(repo_dir.join(".claude/agents/code-reviewer.md"), "---\nname: code-reviewer\ntools: Read\n---\n").unwrap();
        let env = Env { data_dir: t.0.join("data"), worktrees_root: t.0.join("wt"), claude_dir: Some(t.0.join("claude")) };
        let db = open_in_memory().unwrap();
        let task = {
            let mut c = db.lock().unwrap();
            let p = ops::create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None, description: None }, 1).unwrap();
            let input: NewRepo = serde_json::from_value(json!({"path": "x", "defaultIsolation": "in_place"})).unwrap();
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
        let mut r = crate::work::testutil::run_of(executor, kind, review);
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
        SessionReadout { detail: None, last_message: Some(msg.into()), blocker: None }
    }

    #[test]
    fn agent_run_then_reviewer_pass_and_fail() {
        let f = fx("pump-review");
        let agent = Executor::Agent { name: "frontend-developer".into(), source: AgentSource::User };
        let work = launched_run(&f, agent, RunKind::Work, true);
        tasks::set_status(&f.db.lock().unwrap(), &f.task.id, TaskStatus::InProgress, 3).unwrap();
        let mut end = read_end(&work, EndSignal::Done, &done_readout("{\"status\":\"done\",\"summary\":\"all set\"}"), false);
        end.tokens = Some(1234);
        let reviewer = apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 10).unwrap().unwrap();
        assert_eq!(reviewer.kind, RunKind::Review);
        assert_eq!(reviewer.parent_run_id.as_deref(), Some(work.id.as_str()));
        assert_eq!(reviewer.executor, Executor::Agent { name: "code-reviewer".into(), source: AgentSource::Repo });
        assert!(reviewer.prompt.contains("frontend-developer did the work and reported: all set"));
        assert_eq!(task_status(&f), TaskStatus::InProgress, "stays In Progress while reviewing");
        let saved = qruns::get(&f.db.lock().unwrap(), &work.id).unwrap();
        assert_eq!((saved.status, saved.outcome, saved.summary.as_deref()), (RunStatus::Finished, Some(RunOutcome::Green), Some("all set")));
        assert_eq!(saved.tokens, Some(1234), "the transcript tokens are stored on the run");
        // Applying twice doesn't duplicate (the run is no longer launched).
        assert!(apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 11).unwrap().is_none());

        // The reviewer fails → Blocked, no retry.
        let mut rev = reviewer.clone();
        rev.status = RunStatus::Launched;
        qruns::update(&f.db.lock().unwrap(), &rev).unwrap();
        let end = read_end(&rev, EndSignal::Done, &done_readout("{\"verdict\":\"fail\",\"unmet\":[\"c1\"],\"nits\":[]}"), false);
        assert!(apply_end(&f.db, &f.env, &rev, &end, RunStatus::Finished, None, 12).unwrap().is_none());
        assert_eq!(task_status(&f), TaskStatus::Blocked);
        let saved = qruns::get(&f.db.lock().unwrap(), &rev.id).unwrap();
        assert_eq!(saved.verdict.unwrap().unmet, ["c1"]);
        assert!(qruns::pending(&f.db.lock().unwrap()).unwrap().is_empty());

        // Pass → In Review.
        let rev2 = launched_run(&f, Executor::Agent { name: "code-reviewer".into(), source: AgentSource::Repo }, RunKind::Review, false);
        let end = read_end(&rev2, EndSignal::Done, &done_readout("{\"verdict\":\"pass\",\"unmet\":[],\"nits\":[\"n\"]}"), false);
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
        let end = read_end(&work, EndSignal::Done, &done_readout("{\"status\":\"done\"}"), false);
        assert!(apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 10).unwrap().is_none());
        assert_eq!(task_status(&f), TaskStatus::Done);
        assert!(outbox_kinds(&f).is_empty());
        assert_eq!(qruns::get(&f.db.lock().unwrap(), &work.id).unwrap().status, RunStatus::Finished);
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
        let end = read_end(&work, EndSignal::Done, &done_readout("{\"status\":\"done\"}"), false);
        // The repo's reviewer isn't configured and the global one doesn't exist.
        assert!(apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 10).unwrap().is_none());
        assert_eq!(task_status(&f), TaskStatus::Blocked);
        let saved = qruns::get(&f.db.lock().unwrap(), &work.id).unwrap();
        assert!(saved.error.unwrap().contains("Couldn't start the reviewer"));
    }

    fn link_task(f: &Fx) {
        let c = f.db.lock().unwrap();
        let mut map = StateMap { confirmed_at: Some(1), ..Default::default() };
        map.push.insert(TaskStatus::InReview, Some("s-rev".into()));
        map.push.insert(TaskStatus::Blocked, Some("s-blk".into()));
        let link = SourceLink {
            id: "l1".into(),
            project_id: f.task.project_id.clone(),
            provider: "linear".into(),
            scope: ScopeRef { kind: "team".into(), id: "tm".into(), name: "Eng".into() },
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
        let mut stmt = c.prepare("SELECT kind FROM sync_outbox ORDER BY id").unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    #[test]
    fn outbox_written_in_the_same_transaction_unless_manages_source() {
        let f = fx("pump-outbox");
        link_task(&f);
        let wf = Executor::Workflow { name: "linear-issue".into() };
        let run = launched_run(&f, wf.clone(), RunKind::Work, false);
        let ro = SessionReadout { detail: None, last_message: None, blocker: None };
        let end = read_end(&run, EndSignal::Done, &ro, true);
        apply_end(&f.db, &f.env, &run, &end, RunStatus::Finished, Some("linear"), 10).unwrap();
        assert_eq!(task_status(&f), TaskStatus::Blocked);
        assert!(outbox_kinds(&f).is_empty(), "managesSource: the app doesn't write to the provider");

        let run = launched_run(&f, Executor::Claude, RunKind::Work, false);
        let end = read_end(&run, EndSignal::Done, &done_readout("{\"status\":\"done\",\"summary\":\"ok\"}"), false);
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
            .query_row("SELECT payload_json FROM sync_outbox WHERE kind = 'set_state'", [], |r| r.get(0))
            .unwrap();
        assert!(state.contains("todo"), "{state}");
    }

    #[test]
    fn friendly_trust_error() {
        let root = Path::new("/Users/me/.nodal/worktrees");
        let e = friendly_launch_error("`claude --bg` exited ...: Workspace not trusted. Run `claude` in x", "/Users/me/.nodal/worktrees/web/pay-1", root);
        assert!(e.contains("it covers every worktree inside"));
        let e = friendly_launch_error("Workspace not trusted.", "/Users/me/Code/web", root);
        assert!(e.contains("Open Terminal in that folder"));
        assert_eq!(friendly_launch_error("other", "/x", root), "other");
    }

    #[test]
    fn step_comment_lists_findings_and_work_branch() {
        use crate::runs::SessionReadout;
        use crate::work::testutil::run_of;
        let reviewer = Executor::Agent { name: "code-reviewer".into(), source: AgentSource::User };
        let review = run_of(reviewer, RunKind::Review, false);
        let readout = SessionReadout {
            detail: None,
            last_message: Some(r#"{"verdict":"fail","unmet":["c1"],"nits":["n1"],"summary":"c1 is missing"}"#.into()),
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
            let mut r = crate::work::testutil::run_of(Executor::Claude, RunKind::Work, false);
            r.id = "lrun_1".into();
            r.task_id = Some(f.task.id.clone());
            r.repo_id = Some(f.task.repo_id.clone());
            r.status = RunStatus::Queued;
            r.prompt = "/plan-task old".into();
            r.legacy_label = Some("Logo".into());
            qruns::insert(&c, &r).unwrap();
            r
        };
        let cleaning = crate::work::Cleaning::default();
        let run = launch::confirm_legacy(&f.db, &f.env, &cleaning, &legacy.id, 10).unwrap();
        let c = f.db.lock().unwrap();
        assert_eq!((run.status, run.legacy_label.as_deref()), (RunStatus::Queued, None));
        assert_ne!(run.prompt, legacy.prompt, "the prompt is rebuilt from the task");
        let old = qruns::get(&c, &legacy.id).unwrap();
        assert_eq!(old.status, RunStatus::Canceled);
        assert!(old.error.unwrap().contains(&run.id));
        assert_eq!(tasks::get(&c, &f.task.id).unwrap().status, TaskStatus::InProgress);
        // No longer awaiting confirmation: confirming again fails.
        drop(c);
        assert!(launch::confirm_legacy(&f.db, &f.env, &cleaning, &legacy.id, 11).is_err());
    }

    fn legacy_queued(f: &Fx) -> Run {
        let c = f.db.lock().unwrap();
        let mut r = crate::work::testutil::run_of(Executor::Claude, RunKind::Work, false);
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
        let cleaning = crate::work::Cleaning::default();
        let _guard = cleaning.mark(&f.task.id).unwrap();
        let err = launch::confirm_legacy(&f.db, &f.env, &cleaning, &legacy.id, 10).unwrap_err();
        assert_eq!(err, crate::work::CLEANING_ERR);
        let c = f.db.lock().unwrap();
        let old = qruns::get(&c, &legacy.id).unwrap();
        assert!(queue::awaiting_confirmation(&old), "still awaiting confirmation");
        assert_eq!(qruns::pending(&c).unwrap().len(), 1, "nothing was enqueued");
    }

    #[test]
    fn enqueue_is_rejected_while_the_worktree_is_being_cleaned() {
        let f = fx("enqueue-cleaning");
        let cleaning = crate::work::Cleaning::default();
        let input = crate::work::dto::LaunchInput::default();
        {
            let _guard = cleaning.mark(&f.task.id).unwrap();
            assert!(cleaning.mark(&f.task.id).is_none(), "one cleanup at a time");
            let err = launch::enqueue_work(&f.db, &f.env, &cleaning, &f.task.id, &input, false, 10).unwrap_err();
            assert_eq!(err, crate::work::CLEANING_ERR);
            let err = launch::enqueue_review(&f.db, &f.env, &cleaning, &f.task.id, None, 10).unwrap_err();
            assert_eq!(err, crate::work::CLEANING_ERR);
        }
        // Once the mark is released, it enqueues; and a second attempt sees the pending one.
        let run = launch::enqueue_work(&f.db, &f.env, &cleaning, &f.task.id, &input, false, 11).unwrap();
        assert!(run.queue_position > 0.0);
        assert_eq!(task_status(&f), TaskStatus::InProgress);
        let err = launch::enqueue_work(&f.db, &f.env, &cleaning, &f.task.id, &input, false, 12).unwrap_err();
        assert!(err.contains("already has a queued run"), "{err}");
    }

    #[test]
    fn failed_launch_blocks_the_task_with_a_comment() {
        let f = fx("pump-launch-fail");
        link_task(&f);
        let cleaning = crate::work::Cleaning::default();
        let input = crate::work::dto::LaunchInput::default();
        let run = launch::enqueue_work(&f.db, &f.env, &cleaning, &f.task.id, &input, false, 10).unwrap();
        assert_eq!(task_status(&f), TaskStatus::InProgress);
        let mut c = f.db.lock().unwrap();
        // Without `launching` it does nothing.
        assert!(!fail_launch(&mut c, &run.id, "boom", 11).unwrap());
        assert!(qruns::transition(&c, &run.id, RunStatus::Queued, RunStatus::Launching).unwrap());
        assert!(fail_launch(&mut c, &run.id, "Workspace not trusted.", 12).unwrap());
        let saved = qruns::get(&c, &run.id).unwrap();
        assert_eq!((saved.status, saved.error.as_deref()), (RunStatus::Failed, Some("Workspace not trusted.")));
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
        assert_eq!((saved.status, saved.claude_run_id.as_deref(), saved.session_id.as_deref()), (RunStatus::Launched, Some("bg1"), Some("sess-1")));
        assert_eq!(fail_stale_launches(&mut c, NOTE_STALE_LAUNCH, 6).unwrap(), 1);
        assert_eq!(qruns::get(&c, &a.id).unwrap().status, RunStatus::Failed);
        assert_eq!(fail_stale_launches(&mut c, NOTE_STALE_LAUNCH, 7).unwrap(), 0);
    }

    #[test]
    fn reviewer_isolation_follows_its_cwd() {
        assert_eq!(launch::review_isolation("/r/web", "/r/web/"), Isolation::InPlace);
        assert_eq!(launch::review_isolation("/wt/web/pay-1", "/r/web"), Isolation::Worktree);
        // The work ran "in worktree" but the task has no live one: the reviewer runs in the
        // repo folder and takes its lock.
        let f = fx("review-isolation");
        let mut work = launched_run(&f, Executor::Claude, RunKind::Work, true);
        work.isolation = Some(Isolation::Worktree);
        let end = read_end(&work, EndSignal::Done, &done_readout("{\"status\":\"done\"}"), false);
        let reviewer = apply_end(&f.db, &f.env, &work, &end, RunStatus::Finished, None, 10).unwrap().unwrap();
        assert_eq!(reviewer.isolation, Some(Isolation::InPlace));
    }
}
