use super::*;

fn st(id: &str, name: &str, kind: ExtKind) -> ExternalState {
    ExternalState {
        id: id.into(),
        name: name.into(),
        kind,
        color: Some("#999999".into()),
    }
}

/// Fictional team with the typical states of a Linear workspace.
pub fn team_states() -> Vec<ExternalState> {
    vec![
        st("s-triage", "Triage", ExtKind::Triage),
        st("s-backlog", "Backlog", ExtKind::Backlog),
        st("s-todo", "Todo", ExtKind::Unstarted),
        st("s-progress", "In Progress", ExtKind::Started),
        st("s-review", "In Review", ExtKind::Started),
        st("s-blocked", "Blocked", ExtKind::Started),
        st("s-done", "Done", ExtKind::Completed),
        st("s-canceled", "Canceled", ExtKind::Canceled),
        st("s-dup", "Duplicate", ExtKind::Canceled),
    ]
}

#[test]
fn proposal_for_typical_team() {
    let m = propose(&team_states());
    let pull = |id: &str| m.pull[id];
    assert_eq!(pull("s-triage"), TaskStatus::Backlog);
    assert_eq!(pull("s-backlog"), TaskStatus::Backlog);
    assert_eq!(pull("s-todo"), TaskStatus::Todo);
    assert_eq!(pull("s-progress"), TaskStatus::InProgress);
    assert_eq!(pull("s-review"), TaskStatus::InReview);
    assert_eq!(pull("s-blocked"), TaskStatus::Blocked);
    assert_eq!(pull("s-done"), TaskStatus::Done);
    assert_eq!(pull("s-canceled"), TaskStatus::Canceled);
    assert_eq!(pull("s-dup"), TaskStatus::Canceled);

    assert_eq!(m.push.len(), 4);
    assert_eq!(m.push[&TaskStatus::Todo].as_deref(), Some("s-todo"));
    assert_eq!(
        m.push[&TaskStatus::InProgress].as_deref(),
        Some("s-progress")
    );
    assert_eq!(m.push[&TaskStatus::InReview].as_deref(), Some("s-review"));
    assert_eq!(m.push[&TaskStatus::Blocked].as_deref(), Some("s-blocked"));
    assert!(m.confirmed_at.is_none());
    assert_eq!(m.known_states.len(), 9);
}

#[test]
fn push_falls_back_to_similar_or_no_sync() {
    // No "Blocked" or "In Review": In Progress goes to the closest started state, Blocked
    // and In Review stay on "Don't sync".
    let states = vec![
        st("a", "Todo", ExtKind::Unstarted),
        st("b", "Doing", ExtKind::Started),
        st("c", "Code review", ExtKind::Started),
        st("d", "Done", ExtKind::Completed),
    ];
    assert_eq!(
        propose_push(TaskStatus::InProgress, &states).as_deref(),
        Some("b")
    );
    assert_eq!(
        propose_push(TaskStatus::InReview, &states).as_deref(),
        Some("c")
    );
    assert_eq!(propose_push(TaskStatus::Blocked, &states), None);

    let unstarted = vec![
        st("u", "Ready", ExtKind::Unstarted),
        st("b", "Doing", ExtKind::Started),
    ];
    assert_eq!(
        propose_push(TaskStatus::Todo, &unstarted).as_deref(),
        Some("u")
    );
    assert_eq!(
        propose_push(TaskStatus::Todo, &[st("b", "Doing", ExtKind::Started)]),
        None
    );

    let only_review = vec![st("r", "Peer Review", ExtKind::Started)];
    assert_eq!(propose_push(TaskStatus::InProgress, &only_review), None);
}

#[test]
fn push_matches_names_with_team_prefix() {
    let states = vec![
        st("x", "ENG · Blocked", ExtKind::Started),
        st("y", "ENG · In Progress", ExtKind::Started),
    ];
    assert_eq!(
        propose_push(TaskStatus::Blocked, &states).as_deref(),
        Some("x")
    );
    assert_eq!(
        propose_push(TaskStatus::InProgress, &states).as_deref(),
        Some("y")
    );
}

#[test]
fn unknown_kinds_go_by_name() {
    let pull = |n: &str| propose_pull(&st("x", n, ExtKind::Unknown));
    assert_eq!(pull("Done"), TaskStatus::Done);
    assert_eq!(pull("Waiting for review"), TaskStatus::InReview);
    assert_eq!(pull("Blocked"), TaskStatus::Blocked);
    assert_eq!(pull("Doing"), TaskStatus::InProgress);
    assert_eq!(pull("To do"), TaskStatus::Todo);
    assert_eq!(pull("Ideas"), TaskStatus::Backlog);
}

#[test]
fn report_pending_map_is_all_suggested() {
    let states = team_states();
    let saved = propose(&states);
    let r = report(&saved, states.clone());
    assert!(r.pull_origin.values().all(|o| *o == MapOrigin::Suggested));
    assert!(r.push_origin.values().all(|o| *o == MapOrigin::Suggested));
    assert!(r.added.is_empty() && r.removed.is_empty());
    assert_eq!(r.proposal.pull, saved.pull);
}

#[test]
fn report_detects_new_and_removed_states() {
    let mut states = team_states();
    let mut saved = propose(&states);
    saved.confirmed_at = Some(1);
    // The user chose not to sync Blocked.
    saved.push.insert(TaskStatus::Blocked, None);

    // "QA" appears, "In Review" disappears.
    states.retain(|s| s.id != "s-review");
    states.push(st("s-qa", "QA", ExtKind::Started));
    let r = report(&saved, states);

    assert_eq!(
        r.added.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["s-qa"]
    );
    assert_eq!(
        r.removed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["s-review"]
    );
    assert_eq!(r.pull_origin["s-qa"], MapOrigin::Unmapped);
    assert_eq!(r.proposal.pull["s-qa"], TaskStatus::InReview);
    assert_eq!(r.pull_origin["s-todo"], MapOrigin::Confirmed);
    assert!(!r.proposal.pull.contains_key("s-review"));
    // Push for In Review pointed to a state that is gone: unmapped, with a new proposal.
    assert_eq!(r.push_origin[&TaskStatus::InReview], MapOrigin::Unmapped);
    assert_eq!(
        r.proposal.push[&TaskStatus::InReview].as_deref(),
        Some("s-qa")
    );
    assert_eq!(r.push_origin[&TaskStatus::Blocked], MapOrigin::Confirmed);
    assert_eq!(r.proposal.push[&TaskStatus::Blocked], None);
    assert_eq!(r.proposal.known_states, saved.known_states);
}

#[test]
fn push_target_rules() {
    let states = team_states();
    let mut m = propose(&states);
    assert_eq!(
        push_target(&m, TaskStatus::InReview, None),
        PushTarget::Skip(SkipReason::Pending)
    );
    m.confirmed_at = Some(1);
    assert_eq!(
        push_target(&m, TaskStatus::InReview, Some(&states)),
        PushTarget::Push("s-review".into())
    );
    assert_eq!(
        push_target(&m, TaskStatus::Done, Some(&states)),
        PushTarget::Skip(SkipReason::NotMapped)
    );
    m.push.insert(TaskStatus::Blocked, None);
    assert_eq!(
        push_target(&m, TaskStatus::Blocked, None),
        PushTarget::Skip(SkipReason::NoSync)
    );
    let gone: Vec<_> = states
        .iter()
        .filter(|s| s.id != "s-review")
        .cloned()
        .collect();
    assert_eq!(
        push_target(&m, TaskStatus::InReview, Some(&gone)),
        PushTarget::Skip(SkipReason::TargetGone("s-review".into()))
    );
    // Without known current states it pushes anyway.
    assert_eq!(
        push_target(&m, TaskStatus::InReview, None),
        PushTarget::Push("s-review".into())
    );
}

#[test]
fn validate_rejects_missing_targets() {
    let states = team_states();
    let mut m = propose(&states);
    assert!(validate(&m, &states).is_ok());
    m.push.insert(TaskStatus::InReview, Some("nope".into()));
    assert!(validate(&m, &states).unwrap_err().contains("In Review"));
}
