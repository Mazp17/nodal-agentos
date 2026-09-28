use super::*;

fn session(id: &str, cwd: &str, started: i64) -> RunSummary {
    RunSummary {
        id: id.into(),
        session_id: format!("sess-{id}"),
        cwd: Some(cwd.into()),
        name: None,
        started_at: Some(started),
        pid: None,
        status: None,
        state: Some("working".into()),
        waiting_for: None,
    }
}

#[test]
fn adopts_only_an_unambiguous_new_session_in_the_cwd() {
    let live = vec![session("old", "/r/web", 100), session("other", "/r/api", 10_000), session("new", "/r/web/", 10_000)];
    assert_eq!(adoptable("/r/web", 9_000, &live, &[]).map(|s| s.id.as_str()), Some("new"));
    // Another run already has it.
    assert!(adoptable("/r/web", 9_000, &live, &["new".into()]).is_none());
    // Two candidates: no guessing.
    let mut two = live.clone();
    two.push(session("new2", "/r/web", 11_000));
    assert!(adoptable("/r/web", 9_000, &two, &[]).is_none());
    // Before the launch (outside the slack).
    assert!(adoptable("/r/web", 20_000, &live, &[]).is_none());
}
