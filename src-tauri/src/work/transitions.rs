//! Task and run state transitions (pure). The queue gathers the data (`claude agents`
//! output, session files, executor catalog) and applies what's decided here in a single
//! transaction.
//!
//! Rules (plan, Transitions section):
//! - when a work run is enqueued, the task moves to In Progress;
//! - when a work run finishes: if `review` is on (and the executor doesn't review itself)
//!   the reviewer is enqueued and the task stays In Progress; otherwise, green/yellow or
//!   `done` → In Review, red or `blocked` → Blocked;
//! - when the reviewer finishes: pass → In Review; fail → Blocked. No automatic retry;
//! - stopped or dead without a result → Blocked. A failed launch doesn't change the task;
//! - Done and Canceled only by hand (or by pull): a task there isn't touched.

use crate::domain::{Executor, Run, RunKind, RunOutcome, Task, TaskStatus, Verdict};
use crate::runs::SessionReadout;
use crate::util::clip_chars;

use super::queue::EndSignal;
use super::report::{clean_branch, clean_url, parse_agent_report, parse_verdict, ReportStatus};

/// What's known about a run that finished.
#[derive(Debug, Clone, PartialEq)]
pub struct RunEnd {
    pub signal: EndSignal,
    pub outcome: RunOutcome,
    pub summary: Option<String>,
    pub pr: Option<String>,
    pub branch: Option<String>,
    /// From the reviewer, or from the result of a workflow that reviews.
    pub verdict: Option<Verdict>,
    /// Notice to show on the run (stored in `error`).
    pub note: Option<String>,
    /// Finished normally but without the final JSON block.
    pub missing_report: bool,
    /// Transcript tokens (agent/Claude/reviewer); `None` = leave `runs.tokens` as is.
    pub tokens: Option<i64>,
}

pub const NOTE_NO_REPORT: &str = "Finished without a report.";
pub const NOTE_NO_VERDICT: &str = "The reviewer finished without a verdict.";
pub const NOTE_STOPPED: &str = "Stopped before finishing.";
pub const NOTE_FAILED: &str = "The session failed.";
pub const NOTE_VANISHED: &str = "The session is no longer listed by `claude agents`.";
pub const NOTE_NO_RESULT: &str = "The workflow finished without a result.";

/// Interprets the session output according to the run and executor type.
/// `executor_reviews`: the workflow declares `reviews: true` (its result is the verdict).
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
                // Without JSON, the last message serves as the summary for the next step.
                end.summary = readout.last_message.as_deref().map(|m| clip_chars(m.trim(), 4000));
            }
        },
    }
    end
}

/// What to do when a run finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// New task status (`None`: unchanged).
    pub task_status: Option<TaskStatus>,
    /// Enqueue the reviewer on the same cwd.
    pub enqueue_review: bool,
    /// It closes the step: the comment is left on the provider.
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
    // A workflow that reviews brings its own verdict: if it failed, Blocked.
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

/// Final task status: Done/Canceled (set by hand) aren't overwritten.
pub fn apply_status(current: TaskStatus, next: Option<TaskStatus>) -> Option<TaskStatus> {
    match (current, next) {
        (TaskStatus::Done | TaskStatus::Canceled, _) => None,
        (c, Some(n)) if c != n => Some(n),
        _ => None,
    }
}

/// Task status when a work run is enqueued.
pub fn on_enqueue_work(current: TaskStatus) -> Option<TaskStatus> {
    apply_status(current, Some(TaskStatus::InProgress))
}

/// Statuses Nodal pushes to the provider. Todo covers dequeuing a task's only run (it goes
/// back from In Progress to Todo; otherwise the provider would stay In Progress). The
/// provider resolves it with its `state_map` (without a mapping, it's dropped).
pub const PUSHED_STATUSES: [TaskStatus; 4] =
    [TaskStatus::InProgress, TaskStatus::InReview, TaskStatus::Blocked, TaskStatus::Todo];

/// What to write to the provider for a change made in Nodal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboxOp {
    Status(TaskStatus),
    Comment(String),
}

/// Provider writes for a task change. Nothing if the task is local or if the executor syncs
/// the provider on its own (`managesSource` of the same provider). The mapping (pending,
/// "Don't sync", vanished states) is resolved by the provider when draining.
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

    const DONE: &str = "ok ```json\n{\"status\":\"done\",\"summary\":\"done\",\"branch\":\"nodal/pay-1-x\"}\n```";
    const BLOCKED: &str = "{\"status\":\"blocked\",\"summary\":\"no access\"}";

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
        // Without a report the reviewer decides too.
        let end = read_end(&run, EndSignal::Done, &readout("finished"), false);
        assert!(end.missing_report);
        assert_eq!(end.summary.as_deref(), Some("finished"));
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
        let end = read_end(&run, EndSignal::Done, &readout("done, no json"), false);
        assert_eq!(end.outcome, RunOutcome::Unknown);
        assert_eq!(end.note.as_deref(), Some(NOTE_NO_REPORT));
        assert_eq!(decide(&run, &end).task_status, Some(TaskStatus::InReview));
    }

    #[test]
    fn reviewer_pass_and_fail() {
        let review = run_of(Executor::Agent { name: "code-reviewer".into(), source: AgentSource::User }, RunKind::Review, false);
        let pass = read_end(&review, EndSignal::Done, &readout("{\"verdict\":\"pass\",\"unmet\":[],\"nits\":[\"n1\"],\"summary\":\"good\"}"), false);
        assert_eq!(pass.outcome, RunOutcome::Green);
        assert_eq!(decide(&review, &pass).task_status, Some(TaskStatus::InReview));
        let fail = read_end(&review, EndSignal::Done, &readout("{\"verdict\":\"fail\",\"unmet\":[\"c1\"],\"nits\":[],\"summary\":\"no\"}"), false);
        assert_eq!(fail.outcome, RunOutcome::Red);
        let d = decide(&review, &fail);
        assert_eq!(d.task_status, Some(TaskStatus::Blocked));
        assert!(!d.enqueue_review, "no automatic retry");
        let none = read_end(&review, EndSignal::Done, &readout("not sure"), false);
        assert_eq!(none.note.as_deref(), Some(NOTE_NO_VERDICT));
        assert_eq!(decide(&review, &none).task_status, Some(TaskStatus::Blocked));
    }

    #[test]
    fn workflow_with_reviews_skips_gate_and_uses_its_result() {
        // `review` is already resolved to false when enqueueing a workflow that reviews.
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
            project: None,
            rule_id: None,
            moved: None,
        });
        t
    }

    #[test]
    fn outbox_ops_for_linked_tasks() {
        let t = linked_task();
        let ops = outbox_ops(&t, Some(TaskStatus::InReview), Some("c".into()), None);
        assert_eq!(ops, vec![OutboxOp::Status(TaskStatus::InReview), OutboxOp::Comment("c".into())]);
        // Only In Progress, In Review and Blocked are pushed.
        assert_eq!(outbox_ops(&t, Some(TaskStatus::Done), None, None), vec![]);
        assert_eq!(outbox_ops(&t, Some(TaskStatus::Blocked), None, None), vec![OutboxOp::Status(TaskStatus::Blocked)]);
        // managesSource of the same provider: nothing.
        assert!(outbox_ops(&t, Some(TaskStatus::InReview), Some("c".into()), Some("linear")).is_empty());
        // Another provider in managesSource: yes.
        assert_eq!(outbox_ops(&t, Some(TaskStatus::InProgress), None, Some("asana")).len(), 1);
        // Local task: nothing.
        assert!(outbox_ops(&task_of("t2"), Some(TaskStatus::InReview), Some("c".into()), None).is_empty());
        assert_eq!(outbox_ops(&t, Some(TaskStatus::Todo), None, None), vec![OutboxOp::Status(TaskStatus::Todo)]);
    }
}
