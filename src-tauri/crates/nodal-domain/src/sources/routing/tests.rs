use super::*;
use crate::model::{Priority, ScopeRef};

/// Copy of `providers::plan::tests::item` (shared fixture builder), kept here so this
/// module's tests don't reach into the shell crate.
fn item(n: u32, closed_at: Option<String>) -> ExternalItem {
    ExternalItem {
        external_id: format!("uuid-{n}"),
        identifier: format!("ENG-{n}"),
        url: format!("https://linear.app/acme/issue/ENG-{n}/x"),
        title: format!("Issue {n}"),
        description_md: None,
        state: ExternalState {
            id: "s-todo".into(),
            name: "Todo".into(),
            kind: ExtKind::Unstarted,
            color: None,
        },
        scopes: vec![ScopeRef {
            kind: "team".into(),
            id: "team-eng".into(),
            name: "Engineering".into(),
        }],
        parent: None,
        children: Vec::new(),
        labels: Vec::new(),
        assignee: None,
        priority: Priority::None,
        updated_at: "2026-09-20T10:00:00.000Z".into(),
        created_at: Some("2026-09-01T10:00:00.000Z".into()),
        closed_at,
    }
}

#[test]
fn backfill_keeps_open_and_recently_closed() {
    let now = 1_790_000_000_000; // 2026-09-21
    let day = 86_400_000;
    let closed = |kind: ExtKind, ago_days: Option<i64>| {
        let mut it = item(1, None);
        it.state.kind = kind;
        it.closed_at = ago_days.map(|d| iso_from_ms(now - d * day));
        it
    };
    assert!(backfill_keeps(&closed(ExtKind::Backlog, None), now));
    assert!(backfill_keeps(&closed(ExtKind::Triage, None), now));
    assert!(backfill_keeps(&closed(ExtKind::Completed, Some(3)), now));
    assert!(backfill_keeps(&closed(ExtKind::Canceled, Some(13)), now));
    assert!(!backfill_keeps(&closed(ExtKind::Completed, Some(15)), now));
    assert!(!backfill_keeps(&closed(ExtKind::Canceled, Some(40)), now));
    assert!(
        backfill_keeps(&closed(ExtKind::Completed, None), now),
        "no date: trust the provider"
    );
    assert!(!backfill_keeps(&closed(ExtKind::Unknown, None), now));
}
