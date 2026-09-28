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
mod tests;
