use super::*;
use crate::model::*;
use crate::testutil::task_of;

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
