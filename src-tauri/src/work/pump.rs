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
mod tests;
