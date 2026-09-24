//! Transiciones de estado de tareas y runs (puras). La cola junta los datos (salida de
//! `claude agents`, archivos de la sesión, catálogo de ejecutores) y aplica lo que se
//! decide acá en una sola transacción.
//!
//! Reglas (plan, sección Transiciones):
//! - al encolar un run de trabajo, la tarea pasa a In Progress;
//! - al terminar un run de trabajo: si `review` está activa (y el ejecutor no revisa solo)
//!   se encola el revisor y la tarea sigue en In Progress; si no, green/yellow o `done`
//!   → In Review, red o `blocked` → Blocked;
//! - al terminar el revisor: pass → In Review; fail → Blocked. Sin reintento automático;
//! - detenido o muerto sin resultado → Blocked. Un launch fallido no cambia la tarea;
//! - Done y Canceled solo a mano (o por pull): una tarea ahí no se toca.

use crate::domain::{Executor, Run, RunKind, RunOutcome, Task, TaskStatus, Verdict};
use crate::runs::SessionReadout;
use crate::util::clip_chars;

use super::queue::EndSignal;
use super::report::{clean_branch, clean_url, parse_agent_report, parse_verdict, ReportStatus};

/// Lo que se sabe de un run que terminó.
#[derive(Debug, Clone, PartialEq)]
pub struct RunEnd {
    pub signal: EndSignal,
    pub outcome: RunOutcome,
    pub summary: Option<String>,
    pub pr: Option<String>,
    pub branch: Option<String>,
    /// Del revisor, o del resultado de un workflow que revisa.
    pub verdict: Option<Verdict>,
    /// Aviso para mostrar en el run (queda en `error`).
    pub note: Option<String>,
    /// Terminó normalmente pero sin el bloque JSON final.
    pub missing_report: bool,
    /// Tokens del transcript (agente/Claude/revisor); `None` = no tocar `runs.tokens`.
    pub tokens: Option<i64>,
}

pub const NOTE_NO_REPORT: &str = "Finished without a report.";
pub const NOTE_NO_VERDICT: &str = "The reviewer finished without a verdict.";
pub const NOTE_STOPPED: &str = "Stopped before finishing.";
pub const NOTE_FAILED: &str = "The session failed.";
pub const NOTE_VANISHED: &str = "The session is no longer listed by `claude agents`.";
pub const NOTE_NO_RESULT: &str = "The workflow finished without a result.";

/// Interpreta la salida de la sesión según el tipo de run y de ejecutor.
/// `executor_reviews`: el workflow declara `reviews: true` (su resultado es el veredicto).
pub fn read_end(run: &Run, signal: EndSignal, readout: &SessionReadout, executor_reviews: bool) -> RunEnd {
    let mut end = RunEnd {
        signal,
        outcome: RunOutcome::Unknown,
        summary: None,
        pr: None,
        branch: None,
        verdict: None,
        note: None,
        missing_report: false,
        tokens: None,
    };
    match signal {
        EndSignal::Stopped => {
            end.outcome = RunOutcome::Stopped;
            end.note = Some(NOTE_STOPPED.into());
            return end;
        }
        EndSignal::Failed => {
            end.outcome = RunOutcome::Red;
            end.note = Some(NOTE_FAILED.into());
            return end;
        }
        EndSignal::Vanished => {
            end.note = Some(NOTE_VANISHED.into());
            return end;
        }
        EndSignal::Done => {}
    }
    let message = readout.last_message.as_deref().unwrap_or("");
    if run.kind == RunKind::Review {
        match parse_verdict(message) {
            Some(v) => {
                end.outcome = if v.pass { RunOutcome::Green } else { RunOutcome::Red };
                end.summary = v.summary.clone();
                end.verdict = Some(v);
            }
            None => end.note = Some(NOTE_NO_VERDICT.into()),
        }
        return end;
    }
    match &run.executor {
        Executor::Workflow { .. } => {
            let detail = readout.detail.as_ref();
            end.outcome = match detail.and_then(|d| d.result_status.as_deref()) {
                Some("green") => RunOutcome::Green,
                Some("yellow") => RunOutcome::Yellow,
                Some("red") => RunOutcome::Red,
                _ => RunOutcome::Unknown,
            };
            if let Some(res) = detail.and_then(|d| d.result.as_ref()) {
                end.pr = clean_url(res.pr.clone());
                end.branch = clean_branch(res.branch.clone());
                end.summary = res.where_.clone().map(|w| clip_chars(&w, 4000));
                if executor_reviews && end.outcome != RunOutcome::Unknown {
                    end.verdict = Some(Verdict {
                        pass: matches!(end.outcome, RunOutcome::Green | RunOutcome::Yellow),
                        unmet: res.unmet_acceptance.clone().unwrap_or_default(),
                        nits: res.nits.clone().unwrap_or_default(),
                        summary: end.summary.clone(),
                    });
                }
            }
            if end.outcome == RunOutcome::Unknown {
                end.note = Some(match &readout.blocker {
                    Some(wf) => format!(
                        "Claude Code asked to approve the workflow{} in /workflows before running it; approve it and launch again.",
                        wf.as_deref().map(|w| format!(" \"{w}\"")).unwrap_or_default()
                    ),
                    None => NOTE_NO_RESULT.into(),
                });
            }
        }
        Executor::Agent { .. } | Executor::Claude => match parse_agent_report(message) {
            Some(r) => {
                end.outcome = match r.status {
                    ReportStatus::Done => RunOutcome::Green,
                    ReportStatus::Blocked => RunOutcome::Red,
                };
                end.summary = r.summary;
                end.pr = r.pr;
                end.branch = r.branch;
            }
            None => {
                end.missing_report = true;
                end.note = Some(NOTE_NO_REPORT.into());
                // Sin JSON, el último mensaje sirve de resumen para el siguiente paso.
                end.summary = readout.last_message.as_deref().map(|m| clip_chars(m.trim(), 4000));
            }
        },
    }
    end
}

/// Qué hacer cuando termina un run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// Estado nuevo de la tarea (`None`: no cambia).
    pub task_status: Option<TaskStatus>,
    /// Encolar el revisor sobre el mismo cwd.
    pub enqueue_review: bool,
    /// Es el cierre del paso: se deja el comentario en el proveedor.
    pub closing: bool,
}

pub fn decide(run: &Run, end: &RunEnd) -> Decision {
    let blocked = Decision { task_status: Some(TaskStatus::Blocked), enqueue_review: false, closing: true };
    let in_review = Decision { task_status: Some(TaskStatus::InReview), enqueue_review: false, closing: true };
    if end.signal == EndSignal::Stopped {
        return blocked;
    }
    if run.kind == RunKind::Review {
        return match &end.verdict {
            Some(v) if v.pass => in_review,
            _ => blocked,
        };
    }
    let is_workflow = matches!(run.executor, Executor::Workflow { .. });
    let dead = matches!(end.signal, EndSignal::Failed | EndSignal::Vanished)
        || end.outcome == RunOutcome::Red
        || (is_workflow && end.outcome == RunOutcome::Unknown);
    if dead {
        return blocked;
    }
    // Un workflow que revisa trae su veredicto: si falló, Blocked.
    if let Some(v) = &end.verdict {
        if !v.pass {
            return blocked;
        }
    }
    if run.review {
        return Decision { task_status: None, enqueue_review: true, closing: false };
    }
    in_review
}

/// Estado final de la tarea: Done/Canceled (puestos a mano) no se pisan.
pub fn apply_status(current: TaskStatus, next: Option<TaskStatus>) -> Option<TaskStatus> {
    match (current, next) {
        (TaskStatus::Done | TaskStatus::Canceled, _) => None,
        (c, Some(n)) if c != n => Some(n),
        _ => None,
    }
}

/// Estado de la tarea al encolar un run de trabajo.
pub fn on_enqueue_work(current: TaskStatus) -> Option<TaskStatus> {
    apply_status(current, Some(TaskStatus::InProgress))
}

/// Estados que Nodal empuja al proveedor. Todo cubre sacar de la cola el único run de una
/// tarea (vuelve de In Progress a Todo; si no, el proveedor quedaría en In Progress). El
/// proveedor lo resuelve con su `state_map` (sin mapeo, se descarta).
pub const PUSHED_STATUSES: [TaskStatus; 4] =
    [TaskStatus::InProgress, TaskStatus::InReview, TaskStatus::Blocked, TaskStatus::Todo];

/// Qué escribir en el proveedor por un cambio hecho en Nodal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboxOp {
    Status(TaskStatus),
    Comment(String),
}

/// Escrituras al proveedor por un cambio de la tarea. Nada si la tarea es local o si el
/// ejecutor sincroniza el proveedor por su cuenta (`managesSource` del mismo proveedor).
/// El mapeo (pendiente, "No sincronizar", estados desaparecidos) lo resuelve el proveedor
/// al drenar.
pub fn outbox_ops(
    task: &Task,
    new_status: Option<TaskStatus>,
    comment: Option<String>,
    manages_source: Option<&str>,
) -> Vec<OutboxOp> {
    let Some(src) = &task.source else { return Vec::new() };
    if manages_source == Some(src.provider.as_str()) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(s) = new_status.filter(|s| PUSHED_STATUSES.contains(s)) {
        out.push(OutboxOp::Status(s));
    }
    if let Some(body) = comment {
        out.push(OutboxOp::Comment(body));
    }
    out
}

pub fn executor_label(e: &Executor) -> String {
    match e {
        Executor::Agent { name, .. } => name.clone(),
        Executor::Workflow { name } => format!("/{name}"),
        Executor::Claude => "Claude".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;
    use crate::runs::types::{DetailSource, RunDetail, RunResult};
    use crate::work::testutil::{run_of, task_of};

    fn readout(msg: &str) -> SessionReadout {
        SessionReadout { detail: None, last_message: Some(msg.into()), blocker: None }
    }

    fn wf_readout(status: Option<&str>, result: Option<RunResult>) -> SessionReadout {
        SessionReadout {
            detail: Some(RunDetail {
                workflow_id: "wf_1".into(),
                workflow_name: Some("plan-task".into()),
                source: DetailSource::Final,
                status: Some("completed".into()),
                phases: vec![],
                current_phase: None,
                current_phase_index: None,
                agents: vec![],
                agent_count: 0,
                total_tokens: None,
                total_tool_calls: None,
                duration_ms: None,
                result_status: status.map(String::from),
                result,
                workflow_count: 1,
            }),
            last_message: None,
            blocker: None,
        }
    }

    fn agent() -> Executor {
        Executor::Agent { name: "frontend-developer".into(), source: AgentSource::User }
    }

    const DONE: &str = "ok ```json\n{\"status\":\"done\",\"summary\":\"hecho\",\"branch\":\"nodal/pay-1-x\"}\n```";
    const BLOCKED: &str = "{\"status\":\"blocked\",\"summary\":\"falta acceso\"}";

    #[test]
    fn agent_done_without_review_goes_to_in_review() {
        let run = run_of(agent(), RunKind::Work, false);
        let end = read_end(&run, EndSignal::Done, &readout(DONE), false);
        assert_eq!(end.outcome, RunOutcome::Green);
        assert_eq!(end.branch.as_deref(), Some("nodal/pay-1-x"));
        let d = decide(&run, &end);
        assert_eq!(d, Decision { task_status: Some(TaskStatus::InReview), enqueue_review: false, closing: true });
    }

    #[test]
    fn agent_done_with_review_enqueues_reviewer() {
        let run = run_of(agent(), RunKind::Work, true);
        let end = read_end(&run, EndSignal::Done, &readout(DONE), false);
        let d = decide(&run, &end);
        assert_eq!(d, Decision { task_status: None, enqueue_review: true, closing: false });
        // Sin reporte también lo decide el revisor.
        let end = read_end(&run, EndSignal::Done, &readout("terminé"), false);
        assert!(end.missing_report);
        assert_eq!(end.summary.as_deref(), Some("terminé"));
        assert!(decide(&run, &end).enqueue_review);
    }

    #[test]
    fn agent_blocked_or_dead_goes_to_blocked() {
        let run = run_of(agent(), RunKind::Work, true);
        let end = read_end(&run, EndSignal::Done, &readout(BLOCKED), false);
        assert_eq!(end.outcome, RunOutcome::Red);
        assert_eq!(decide(&run, &end).task_status, Some(TaskStatus::Blocked));
        for sig in [EndSignal::Stopped, EndSignal::Failed, EndSignal::Vanished] {
            let end = read_end(&run, sig, &readout(DONE), false);
            let d = decide(&run, &end);
            assert_eq!(d.task_status, Some(TaskStatus::Blocked), "{sig:?}");
            assert!(!d.enqueue_review);
        }
        assert_eq!(read_end(&run, EndSignal::Stopped, &readout(""), false).outcome, RunOutcome::Stopped);
    }

    #[test]
    fn no_report_without_reviewer_goes_to_in_review_with_warning() {
        let run = run_of(Executor::Claude, RunKind::Work, false);
        let end = read_end(&run, EndSignal::Done, &readout("listo, sin json"), false);
        assert_eq!(end.outcome, RunOutcome::Unknown);
        assert_eq!(end.note.as_deref(), Some(NOTE_NO_REPORT));
        assert_eq!(decide(&run, &end).task_status, Some(TaskStatus::InReview));
    }

    #[test]
    fn reviewer_pass_and_fail() {
        let review = run_of(Executor::Agent { name: "code-reviewer".into(), source: AgentSource::User }, RunKind::Review, false);
        let pass = read_end(&review, EndSignal::Done, &readout("{\"verdict\":\"pass\",\"unmet\":[],\"nits\":[\"n1\"],\"summary\":\"bien\"}"), false);
        assert_eq!(pass.outcome, RunOutcome::Green);
        assert_eq!(decide(&review, &pass).task_status, Some(TaskStatus::InReview));
        let fail = read_end(&review, EndSignal::Done, &readout("{\"verdict\":\"fail\",\"unmet\":[\"c1\"],\"nits\":[],\"summary\":\"no\"}"), false);
        assert_eq!(fail.outcome, RunOutcome::Red);
        let d = decide(&review, &fail);
        assert_eq!(d.task_status, Some(TaskStatus::Blocked));
        assert!(!d.enqueue_review, "sin reintento automático");
        let none = read_end(&review, EndSignal::Done, &readout("no sé"), false);
        assert_eq!(none.note.as_deref(), Some(NOTE_NO_VERDICT));
        assert_eq!(decide(&review, &none).task_status, Some(TaskStatus::Blocked));
    }

    #[test]
    fn workflow_with_reviews_skips_gate_and_uses_its_result() {
        // `review` ya viene resuelto en false al encolar un workflow que revisa.
        let run = run_of(Executor::Workflow { name: "plan-task".into() }, RunKind::Work, false);
        let res = RunResult {
            pr: Some("https://example.com/acme/web/pull/3".into()),
            branch: Some("plan/x".into()),
            nits: Some(vec!["n".into()]),
            ..Default::default()
        };
        let end = read_end(&run, EndSignal::Done, &wf_readout(Some("yellow"), Some(res)), true);
        assert_eq!(end.outcome, RunOutcome::Yellow);
        assert_eq!(end.pr.as_deref(), Some("https://example.com/acme/web/pull/3"));
        let v = end.verdict.clone().unwrap();
        assert!(v.pass);
        assert_eq!(v.nits, ["n"]);
        assert_eq!(decide(&run, &end).task_status, Some(TaskStatus::InReview));

        let red = read_end(&run, EndSignal::Done, &wf_readout(Some("red"), Some(RunResult::default())), true);
        assert_eq!(decide(&run, &red).task_status, Some(TaskStatus::Blocked));
    }

    #[test]
    fn workflow_without_result_is_blocked_and_explains_blocker() {
        let run = run_of(Executor::Workflow { name: "plan-task".into() }, RunKind::Work, true);
        let mut ro = readout("");
        ro.blocker = Some(Some("plan-task".into()));
        let end = read_end(&run, EndSignal::Done, &ro, true);
        assert!(end.note.unwrap().contains("approve the workflow \"plan-task\""));
        let end = read_end(&run, EndSignal::Done, &wf_readout(None, None), true);
        assert_eq!(end.note.as_deref(), Some(NOTE_NO_RESULT));
        let d = decide(&run, &end);
        assert_eq!(d.task_status, Some(TaskStatus::Blocked));
        assert!(!d.enqueue_review);
    }

    #[test]
    fn workflow_without_reviews_goes_through_the_gate() {
        let run = run_of(Executor::Workflow { name: "demo-board".into() }, RunKind::Work, true);
        let end = read_end(&run, EndSignal::Done, &wf_readout(Some("green"), Some(RunResult::default())), false);
        assert_eq!(end.verdict, None);
        assert!(decide(&run, &end).enqueue_review);
    }

    #[test]
    fn manual_done_is_not_overwritten() {
        assert_eq!(apply_status(TaskStatus::Done, Some(TaskStatus::Blocked)), None);
        assert_eq!(apply_status(TaskStatus::Canceled, Some(TaskStatus::InReview)), None);
        assert_eq!(apply_status(TaskStatus::InProgress, Some(TaskStatus::InReview)), Some(TaskStatus::InReview));
        assert_eq!(apply_status(TaskStatus::InReview, Some(TaskStatus::InReview)), None);
        assert_eq!(on_enqueue_work(TaskStatus::Todo), Some(TaskStatus::InProgress));
        assert_eq!(on_enqueue_work(TaskStatus::InProgress), None);
    }

    fn linked_task() -> Task {
        let mut t = task_of("t1");
        t.source = Some(TaskSource {
            provider: "linear".into(),
            link_id: Some("l1".into()),
            external_id: "e1".into(),
            identifier: "ENG-1".into(),
            url: "https://linear.app/acme/issue/ENG-1".into(),
            external_state: None,
            last_synced_at: None,
            sync_error: None,
            unmapped: false,
        });
        t
    }

    #[test]
    fn outbox_ops_for_linked_tasks() {
        let t = linked_task();
        let ops = outbox_ops(&t, Some(TaskStatus::InReview), Some("c".into()), None);
        assert_eq!(ops, vec![OutboxOp::Status(TaskStatus::InReview), OutboxOp::Comment("c".into())]);
        // Solo In Progress, In Review y Blocked se empujan.
        assert_eq!(outbox_ops(&t, Some(TaskStatus::Done), None, None), vec![]);
        assert_eq!(outbox_ops(&t, Some(TaskStatus::Blocked), None, None), vec![OutboxOp::Status(TaskStatus::Blocked)]);
        // managesSource del mismo proveedor: nada.
        assert!(outbox_ops(&t, Some(TaskStatus::InReview), Some("c".into()), Some("linear")).is_empty());
        // Otro proveedor en managesSource: sí.
        assert_eq!(outbox_ops(&t, Some(TaskStatus::InProgress), None, Some("asana")).len(), 1);
        // Tarea local: nada.
        assert!(outbox_ops(&task_of("t2"), Some(TaskStatus::InReview), Some("c".into()), None).is_empty());
        assert_eq!(outbox_ops(&t, Some(TaskStatus::Todo), None, None), vec![OutboxOp::Status(TaskStatus::Todo)]);
    }
}
