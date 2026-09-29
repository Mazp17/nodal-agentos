use super::*;
use crate::model::claude::{DetailSource, RunDetail, RunResult};
use crate::model::*;
use crate::testutil::{run_of, task_of};

fn readout(msg: &str) -> SessionReadout {
    SessionReadout {
        detail: None,
        last_message: Some(msg.into()),
        blocker: None,
    }
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
    Executor::Agent {
        name: "frontend-developer".into(),
        source: AgentSource::User,
    }
}

const DONE: &str =
    "ok ```json\n{\"status\":\"done\",\"summary\":\"done\",\"branch\":\"nodal/pay-1-x\"}\n```";
const BLOCKED: &str = "{\"status\":\"blocked\",\"summary\":\"no access\"}";

#[test]
fn agent_done_without_review_goes_to_in_review() {
    let run = run_of(agent(), RunKind::Work, false);
    let end = read_end(&run, EndSignal::Done, &readout(DONE), false);
    assert_eq!(end.outcome, RunOutcome::Green);
    assert_eq!(end.branch.as_deref(), Some("nodal/pay-1-x"));
    let d = decide(&run, &end);
    assert_eq!(
        d,
        Decision {
            task_status: Some(TaskStatus::InReview),
            enqueue_review: false,
            closing: true
        }
    );
}

#[test]
fn agent_done_with_review_enqueues_reviewer() {
    let run = run_of(agent(), RunKind::Work, true);
    let end = read_end(&run, EndSignal::Done, &readout(DONE), false);
    let d = decide(&run, &end);
    assert_eq!(
        d,
        Decision {
            task_status: None,
            enqueue_review: true,
            closing: false
        }
    );
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
    assert_eq!(
        read_end(&run, EndSignal::Stopped, &readout(""), false).outcome,
        RunOutcome::Stopped
    );
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
    let review = run_of(
        Executor::Agent {
            name: "code-reviewer".into(),
            source: AgentSource::User,
        },
        RunKind::Review,
        false,
    );
    let pass = read_end(
        &review,
        EndSignal::Done,
        &readout("{\"verdict\":\"pass\",\"unmet\":[],\"nits\":[\"n1\"],\"summary\":\"good\"}"),
        false,
    );
    assert_eq!(pass.outcome, RunOutcome::Green);
    assert_eq!(
        decide(&review, &pass).task_status,
        Some(TaskStatus::InReview)
    );
    let fail = read_end(
        &review,
        EndSignal::Done,
        &readout("{\"verdict\":\"fail\",\"unmet\":[\"c1\"],\"nits\":[],\"summary\":\"no\"}"),
        false,
    );
    assert_eq!(fail.outcome, RunOutcome::Red);
    let d = decide(&review, &fail);
    assert_eq!(d.task_status, Some(TaskStatus::Blocked));
    assert!(!d.enqueue_review, "no automatic retry");
    let none = read_end(&review, EndSignal::Done, &readout("not sure"), false);
    assert_eq!(none.note.as_deref(), Some(NOTE_NO_VERDICT));
    assert_eq!(
        decide(&review, &none).task_status,
        Some(TaskStatus::Blocked)
    );
}

#[test]
fn workflow_with_reviews_skips_gate_and_uses_its_result() {
    // `review` is already resolved to false when enqueueing a workflow that reviews.
    let run = run_of(
        Executor::Workflow {
            name: "plan-task".into(),
        },
        RunKind::Work,
        false,
    );
    let res = RunResult {
        pr: Some("https://example.com/acme/web/pull/3".into()),
        branch: Some("plan/x".into()),
        nits: Some(vec!["n".into()]),
        ..Default::default()
    };
    let end = read_end(
        &run,
        EndSignal::Done,
        &wf_readout(Some("yellow"), Some(res)),
        true,
    );
    assert_eq!(end.outcome, RunOutcome::Yellow);
    assert_eq!(
        end.pr.as_deref(),
        Some("https://example.com/acme/web/pull/3")
    );
    let v = end.verdict.clone().unwrap();
    assert!(v.pass);
    assert_eq!(v.nits, ["n"]);
    assert_eq!(decide(&run, &end).task_status, Some(TaskStatus::InReview));

    let red = read_end(
        &run,
        EndSignal::Done,
        &wf_readout(Some("red"), Some(RunResult::default())),
        true,
    );
    assert_eq!(decide(&run, &red).task_status, Some(TaskStatus::Blocked));
}

#[test]
fn workflow_without_result_is_blocked_and_explains_blocker() {
    let run = run_of(
        Executor::Workflow {
            name: "plan-task".into(),
        },
        RunKind::Work,
        true,
    );
    let mut ro = readout("");
    ro.blocker = Some(Some("plan-task".into()));
    let end = read_end(&run, EndSignal::Done, &ro, true);
    assert!(end
        .note
        .unwrap()
        .contains("approve the workflow \"plan-task\""));
    let end = read_end(&run, EndSignal::Done, &wf_readout(None, None), true);
    assert_eq!(end.note.as_deref(), Some(NOTE_NO_RESULT));
    let d = decide(&run, &end);
    assert_eq!(d.task_status, Some(TaskStatus::Blocked));
    assert!(!d.enqueue_review);
}

#[test]
fn workflow_without_reviews_goes_through_the_gate() {
    let run = run_of(
        Executor::Workflow {
            name: "demo-board".into(),
        },
        RunKind::Work,
        true,
    );
    let end = read_end(
        &run,
        EndSignal::Done,
        &wf_readout(Some("green"), Some(RunResult::default())),
        false,
    );
    assert_eq!(end.verdict, None);
    assert!(decide(&run, &end).enqueue_review);
}

#[test]
fn manual_done_is_not_overwritten() {
    assert_eq!(
        apply_status(TaskStatus::Done, Some(TaskStatus::Blocked)),
        None
    );
    assert_eq!(
        apply_status(TaskStatus::Canceled, Some(TaskStatus::InReview)),
        None
    );
    assert_eq!(
        apply_status(TaskStatus::InProgress, Some(TaskStatus::InReview)),
        Some(TaskStatus::InReview)
    );
    assert_eq!(
        apply_status(TaskStatus::InReview, Some(TaskStatus::InReview)),
        None
    );
    assert_eq!(
        on_enqueue_work(TaskStatus::Todo),
        Some(TaskStatus::InProgress)
    );
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
    assert_eq!(
        ops,
        vec![
            OutboxOp::Status(TaskStatus::InReview),
            OutboxOp::Comment("c".into())
        ]
    );
    // Only In Progress, In Review and Blocked are pushed.
    assert_eq!(outbox_ops(&t, Some(TaskStatus::Done), None, None), vec![]);
    assert_eq!(
        outbox_ops(&t, Some(TaskStatus::Blocked), None, None),
        vec![OutboxOp::Status(TaskStatus::Blocked)]
    );
    // managesSource of the same provider: nothing.
    assert!(outbox_ops(
        &t,
        Some(TaskStatus::InReview),
        Some("c".into()),
        Some("linear")
    )
    .is_empty());
    // Another provider in managesSource: yes.
    assert_eq!(
        outbox_ops(&t, Some(TaskStatus::InProgress), None, Some("asana")).len(),
        1
    );
    // Local task: nothing.
    assert!(outbox_ops(
        &task_of("t2"),
        Some(TaskStatus::InReview),
        Some("c".into()),
        None
    )
    .is_empty());
    assert_eq!(
        outbox_ops(&t, Some(TaskStatus::Todo), None, None),
        vec![OutboxOp::Status(TaskStatus::Todo)]
    );
}
