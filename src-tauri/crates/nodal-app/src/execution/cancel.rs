//! Cancelling a run: dequeues a `queued` one, or stops a launched one, saving whatever it left
//! half-done as a patch and moving the task to Blocked.
//!
//! Moved from `commands::execution::{cancel_run, cancel_run_inner}`. All I/O now goes through
//! the injected ports (`core.claude`, `core.sessions`, `env.git`, `env.plans`) instead of the
//! shell's direct `nodal_host`/`std::fs` calls.

use std::path::Path;

use nodal_domain::execution::queue::EndSignal;
use nodal_domain::execution::transitions::{RunEnd, NOTE_STOPPED};
use nodal_domain::model::{Executor, Run, RunKind, RunOutcome, RunStatus, TaskStatus};
use nodal_domain::util::check_id;
use nodal_store::execution::runs as qruns;
use nodal_store::rows;
use nodal_store::{with_db, StoreError};

use crate::core::{blocking, AppError};

use super::diff::diff_base;
use super::enqueue::lock;
use super::pump::{apply_end, workflow_meta};
use super::Execution;

impl Execution {
    /// Dequeues a `queued` run, or stops a launched one: saves whatever it left half-done as a
    /// patch in `<app_data>/runs/<id>/stopped.patch` and the task moves to Blocked.
    pub async fn cancel_run(&self, run_id: String) -> Result<Run, AppError> {
        let r = self.cancel_run_inner(run_id).await;
        self.core.notifier.notify_all(super::RUN_KINDS, None);
        Ok(r?)
    }

    async fn cancel_run_inner(&self, run_id: String) -> Result<Run, String> {
        check_id(&run_id, "run")?;
        // Holding the queue's turn: the pass can't close this run midway (it would end up
        // `finished`, without the patch note, and even with the reviewer enqueued).
        let _turn = self.pump.lock().await;
        let id = run_id.clone();
        let (run, task, repo_path) = with_db(&self.core.db, move |c| {
            let r = qruns::get(c, &id)?;
            let t = match &r.task_id {
                Some(t) => rows::get_task(c, t)?,
                None => None,
            };
            let repo_path = match &r.repo_id {
                Some(id) => rows::get_repo(c, id)?.map(|r| r.path),
                None => None,
            };
            Ok((r, t, repo_path))
        })
        .await?;
        match run.status {
            RunStatus::Queued => {
                let id = run_id.clone();
                let now = self.core.clock.now_ms();
                with_db(&self.core.db, move |c| {
                    let tx = c.transaction()?;
                    if !qruns::transition(&tx, &id, RunStatus::Queued, RunStatus::Canceled)? {
                        return Err(StoreError::Invalid("The run is no longer queued.".into()));
                    }
                    qruns::set_finished_at(&tx, &id, now)?;
                    // With no other pending runs, the task doesn't stay In Progress: back to
                    // Todo if it was work, or to Blocked if it was the reviewer (the gate
                    // didn't pass).
                    if let Some(t) = task {
                        if t.status == TaskStatus::InProgress && qruns::pending_for_task(&tx, &t.id)?.is_empty() {
                            let next = match run.kind {
                                RunKind::Work => TaskStatus::Todo,
                                RunKind::Review => TaskStatus::Blocked,
                            };
                            crate::board::ops::apply_task_transition(&tx, &t, Some(next), None, None, now)
                                .map_err(StoreError::Invalid)?;
                        }
                    }
                    tx.commit()?;
                    qruns::get(c, &id)
                })
                .await
                .map_err(String::from)
            }
            RunStatus::Launching => Err("The run is launching: wait for it to start and stop it then.".into()),
            RunStatus::Launched => {
                let claude_id = run.claude_run_id.clone().ok_or("The run has no session id yet.")?;
                // Whatever is half-done, before stopping it (workflows work in their own worktree).
                let patch_path = if matches!(run.executor, Executor::Workflow { .. }) {
                    None
                } else {
                    let (cwd, base) = (run.cwd.clone(), diff_base(task.as_ref(), &run));
                    let target = self.core.env.stopped_patch_path(&run.id);
                    let git = self.core.env.git.clone();
                    let plans = self.core.env.plans.clone();
                    blocking(move || {
                        let Ok((patch, _)) = git.diff(Path::new(&cwd), base.as_deref()) else { return Ok(None) };
                        if patch.trim().is_empty() {
                            return Ok(None);
                        }
                        plans.write_atomic(&target, patch.as_bytes()).map_err(|e| e.to_string())?;
                        Ok(Some(target))
                    })
                    .await
                    .unwrap_or_else(|e| {
                        eprintln!("work: couldn't save the partial patch: {e}");
                        None
                    })
                };
                self.core.claude.stop(&claude_id).await?;
                let tokens = match (&run.executor, run.session_id.clone()) {
                    (Executor::Workflow { .. }, _) | (_, None) => None,
                    (_, Some(sid)) => {
                        let cwd = run.cwd.clone();
                        let sessions = self.core.sessions.clone();
                        blocking(move || Ok(sessions.session_tokens(&sid, &cwd))).await.unwrap_or(None)
                    }
                };
                let note = match &patch_path {
                    Some(p) => format!("{NOTE_STOPPED} Stopped by the user; partial changes saved to {}.", p.display()),
                    None => format!("{NOTE_STOPPED} Stopped by the user."),
                };
                let end = RunEnd {
                    signal: EndSignal::Stopped,
                    outcome: RunOutcome::Stopped,
                    summary: None,
                    pr: None,
                    branch: None,
                    verdict: None,
                    note: Some(note),
                    missing_report: false,
                    tokens,
                };
                let env = self.core.env.clone();
                let id = run.id.clone();
                let (env2, executor) = (env.clone(), run.executor.clone());
                let (_, manages) = blocking(move || Ok(workflow_meta(&env2, repo_path.as_deref(), &executor))).await?;
                let db = self.core.db.clone();
                let now = self.core.clock.now_ms();
                blocking(move || {
                    apply_end(&db, &env, &run, &end, RunStatus::Canceled, manages.as_deref(), now)?;
                    Ok(qruns::get(&lock(&db), &id)?)
                })
                .await
            }
            _ => Err("The run already finished.".into()),
        }
    }
}
