//! La pasada periódica de la cola: completa sessionIds, detecta los runs que terminaron
//! (salen de working/blocked en `claude agents`), lee su resultado, aplica la transición
//! (con el outbox y el revisor en la misma transacción) y lanza lo que entre.

use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;

use crate::db::queries::{repos, runs as qruns};
use crate::db::{rows, with_db};
use crate::domain::{Executor, Run, RunKind, RunStatus, TaskStatus};
use crate::providers::plan::{closing_comment, ClosingInfo};
use crate::runs::{self, SessionReadout};
use crate::util::{blocking, now_ms};

use super::queue::{self, EndSignal};
use super::transitions::{decide, executor_label, read_end, RunEnd};
use super::{executors, launch, ops, Env, Inner};

/// Mensaje de error de lanzamiento con la salida concreta para los casos conocidos.
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

/// Cierra un run que terminó (`final_status`: Finished, o Canceled si lo detuvo el usuario):
/// guarda su resultado, mueve la tarea, deja el outbox y encola el revisor si corresponde,
/// todo en una transacción. Si el run ya no está `launched` (otra pasada lo cerró), no hace
/// nada. Devuelve el revisor encolado, si hubo.
pub fn apply_end(
    conn: &mut Connection,
    env: &Env,
    run: &Run,
    end: &RunEnd,
    final_status: RunStatus,
    manages_source: Option<&str>,
    now: i64,
) -> Result<Option<Run>, String> {
    let mut decision = decide(run, end);
    let task = match &run.task_id {
        Some(id) => rows::get_task(conn, id)?,
        None => None,
    };
    // Una tarea cerrada a mano (Done/Canceled) mientras corría: sin revisor ni comentario.
    let closed = task.as_ref().is_some_and(|t| matches!(t.status, TaskStatus::Done | TaskStatus::Canceled));
    if closed {
        decision.enqueue_review = false;
        decision.closing = false;
    }
    let mut note = end.note.clone();
    // El revisor ve el run ya con su resultado (resumen, rama).
    let mut view = run.clone();
    view.summary = end.summary.clone();
    view.pr_url = end.pr.clone();
    view.branch = end.branch.clone();
    view.outcome = Some(end.outcome);
    let reviewer = match (&task, decision.enqueue_review) {
        (Some(t), true) => match launch::prepare_review(conn, env, t, Some(&view), None, None, now) {
            Ok(r) => Some(r),
            Err(e) => {
                // Sin revisor no hay gate: la tarea queda bloqueada con el motivo.
                decision.enqueue_review = false;
                decision.task_status = Some(TaskStatus::Blocked);
                decision.closing = true;
                note = Some(format!("Couldn't start the reviewer: {e}"));
                None
            }
        },
        _ => None,
    };
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let current = qruns::get(&tx, &run.id)?;
    if current.status != RunStatus::Launched {
        return Ok(None);
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
    if let Some(r) = &reviewer {
        qruns::insert(&tx, r)?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(reviewer)
}

/// Comentario de cierre de un paso. `work` es el run de trabajo del paso (PR/rama cuando
/// cierra el revisor).
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

/// `(reviews, managesSource)` del workflow del run, del catálogo en disco.
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
    let (readout, (reviews, manages)) = blocking(move || {
        let readout = match (signal, session) {
            (EndSignal::Done, Some(sid)) => runs::read_session(&sid, &cwd),
            _ => SessionReadout::default(),
        };
        Ok((readout, workflow_meta(&env2, repo_path.as_deref(), &executor)))
    })
    .await?;
    let end = read_end(&run, signal, &readout, reviews);
    let now = now_ms();
    with_db(&inner.db, move |c| {
        apply_end(c, &env, &run, &end, RunStatus::Finished, manages.as_deref(), now)
            .map_err(crate::db::DbError::Invalid)
    })
    .await?;
    Ok(())
}

/// Una pasada de la cola. Si no hay nada en cola ni lanzado, no consulta `claude agents`.
pub async fn pump(inner: &Arc<Inner>) -> Result<(), String> {
    let _turn = inner.pump.lock().await;
    let mut pending = with_db(&inner.db, |c| qruns::pending(c)).await?;
    if !queue::needs_tick(&pending) {
        return Ok(());
    }
    let live = runs::list_runs().await?;

    let changed: Vec<Run> =
        queue::fill_session_ids(&mut pending, &live).into_iter().map(|i| pending[i].clone()).collect();
    if !changed.is_empty() {
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
        let result = runs::launch_with(run.cwd.clone(), run.prompt.clone(), &run.options, &launch::extra_flags(&run)).await;
        let root = inner.env.worktrees_root.clone();
        with_db(&inner.db, move |c| {
            let mut r = qruns::get(c, &run.id)?;
            if r.status != RunStatus::Launching {
                return Ok(());
            }
            let now = now_ms();
            match result {
                Ok(rr) => {
                    r.status = RunStatus::Launched;
                    r.claude_run_id = Some(rr.id);
                    r.launched_at = Some(now);
                }
                Err(e) => {
                    // El launch fallido no cambia la tarea: el error queda en el run.
                    r.status = RunStatus::Failed;
                    r.finished_at = Some(now);
                    r.error = Some(friendly_launch_error(&e, &r.cwd, &root));
                }
            }
            qruns::update(c, &r)
        })
        .await?;
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

    /// Proyecto, repo (carpeta con `.claude/agents/code-reviewer.md`) y una tarea con plan.
    fn fx(name: &str) -> Fx {
        let t = TempDir::new(name);
        let repo_dir = t.0.join("web");
        std::fs::create_dir_all(repo_dir.join(".claude/agents")).unwrap();
        std::fs::write(repo_dir.join(".claude/agents/code-reviewer.md"), "---\nname: code-reviewer\ntools: Read\n---\n").unwrap();
        let env = Env { data_dir: t.0.join("data"), worktrees_root: t.0.join("wt"), claude_dir: Some(t.0.join("claude")) };
        let db = open_in_memory().unwrap();
        let task = {
            let mut c = db.lock().unwrap();
            let p = ops::create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None }, 1).unwrap();
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
        let end = read_end(&work, EndSignal::Done, &done_readout("{\"status\":\"done\",\"summary\":\"listo\"}"), false);
        let reviewer = apply_end(&mut f.db.lock().unwrap(), &f.env, &work, &end, RunStatus::Finished, None, 10).unwrap().unwrap();
        assert_eq!(reviewer.kind, RunKind::Review);
        assert_eq!(reviewer.parent_run_id.as_deref(), Some(work.id.as_str()));
        assert_eq!(reviewer.executor, Executor::Agent { name: "code-reviewer".into(), source: AgentSource::Repo });
        assert!(reviewer.prompt.contains("frontend-developer did the work and reported: listo"));
        assert_eq!(task_status(&f), TaskStatus::InProgress, "sigue en In Progress mientras revisa");
        let saved = qruns::get(&f.db.lock().unwrap(), &work.id).unwrap();
        assert_eq!((saved.status, saved.outcome, saved.summary.as_deref()), (RunStatus::Finished, Some(RunOutcome::Green), Some("listo")));
        // Aplicar dos veces no duplica (el run ya no está launched).
        assert!(apply_end(&mut f.db.lock().unwrap(), &f.env, &work, &end, RunStatus::Finished, None, 11).unwrap().is_none());

        // El revisor falla → Blocked, sin reintento.
        let mut rev = reviewer.clone();
        rev.status = RunStatus::Launched;
        qruns::update(&f.db.lock().unwrap(), &rev).unwrap();
        let end = read_end(&rev, EndSignal::Done, &done_readout("{\"verdict\":\"fail\",\"unmet\":[\"c1\"],\"nits\":[]}"), false);
        assert!(apply_end(&mut f.db.lock().unwrap(), &f.env, &rev, &end, RunStatus::Finished, None, 12).unwrap().is_none());
        assert_eq!(task_status(&f), TaskStatus::Blocked);
        let saved = qruns::get(&f.db.lock().unwrap(), &rev.id).unwrap();
        assert_eq!(saved.verdict.unwrap().unmet, ["c1"]);
        assert!(qruns::pending(&f.db.lock().unwrap()).unwrap().is_empty());

        // Pass → In Review.
        let rev2 = launched_run(&f, Executor::Agent { name: "code-reviewer".into(), source: AgentSource::Repo }, RunKind::Review, false);
        let end = read_end(&rev2, EndSignal::Done, &done_readout("{\"verdict\":\"pass\",\"unmet\":[],\"nits\":[\"n\"]}"), false);
        apply_end(&mut f.db.lock().unwrap(), &f.env, &rev2, &end, RunStatus::Finished, None, 13).unwrap();
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
        assert!(apply_end(&mut f.db.lock().unwrap(), &f.env, &work, &end, RunStatus::Finished, None, 10).unwrap().is_none());
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
        // El reviewer del repo no está configurado y el global no existe.
        assert!(apply_end(&mut f.db.lock().unwrap(), &f.env, &work, &end, RunStatus::Finished, None, 10).unwrap().is_none());
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
        apply_end(&mut f.db.lock().unwrap(), &f.env, &run, &end, RunStatus::Finished, Some("linear"), 10).unwrap();
        assert_eq!(task_status(&f), TaskStatus::Blocked);
        assert!(outbox_kinds(&f).is_empty(), "managesSource: la app no escribe en el proveedor");

        let run = launched_run(&f, Executor::Claude, RunKind::Work, false);
        let end = read_end(&run, EndSignal::Done, &done_readout("{\"status\":\"done\",\"summary\":\"ok\"}"), false);
        apply_end(&mut f.db.lock().unwrap(), &f.env, &run, &end, RunStatus::Finished, None, 11).unwrap();
        assert_eq!(task_status(&f), TaskStatus::InReview);
        assert_eq!(outbox_kinds(&f), ["set_state", "comment"]);
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
            last_message: Some(r#"{"verdict":"fail","unmet":["c1"],"nits":["n1"],"summary":"falta c1"}"#.into()),
            blocker: None,
        };
        let end = read_end(&review, EndSignal::Done, &readout, false);
        let mut work = run_of(Executor::Claude, RunKind::Work, true);
        work.branch = Some("nodal/pay-1-x".into());
        let c = step_comment(&review, &end, None, Some(&work), TaskStatus::Blocked);
        assert_eq!(
            c,
            "**Nodal** · Blocked · code-reviewer\n\nBranch: `nodal/pay-1-x`\n\nfalta c1\n\n**Unmet criteria**\n- c1\n\n**Nits**\n- n1"
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
            r.prompt = "/plan-task viejo".into();
            r.legacy_label = Some("Logo".into());
            qruns::insert(&c, &r).unwrap();
            r
        };
        let mut c = f.db.lock().unwrap();
        let run = launch::confirm_legacy(&mut c, &f.env, &legacy.id, 10).unwrap();
        assert_eq!((run.status, run.legacy_label.as_deref()), (RunStatus::Queued, None));
        assert_ne!(run.prompt, legacy.prompt, "el prompt se arma de nuevo desde la tarea");
        let old = qruns::get(&c, &legacy.id).unwrap();
        assert_eq!(old.status, RunStatus::Canceled);
        assert!(old.error.unwrap().contains(&run.id));
        assert_eq!(tasks::get(&c, &f.task.id).unwrap().status, TaskStatus::InProgress);
        // Ya no espera confirmación: confirmar de nuevo falla.
        assert!(launch::confirm_legacy(&mut c, &f.env, &legacy.id, 11).is_err());
    }
}
