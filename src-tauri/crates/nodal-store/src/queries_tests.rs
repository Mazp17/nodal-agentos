use crate::board::{hidden_executors, projects, repos, tasks};
use crate::rows::{insert_project, insert_repo, insert_task};
use crate::{chats, sources, Db};
use nodal_domain::model::*;
use nodal_domain::testutil::{project_of, repo_of, task_of};

fn seed(c: &crate::Conn) {
    insert_project(c, &project_of("p1", "PAY")).unwrap();
    insert_repo(c, &repo_of("r1", "p1", "/r1")).unwrap();
    insert_task(c, &task_of("t1")).unwrap();
}

#[test]
fn task_number_counter_and_status() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    assert_eq!(projects::take_task_number(&c, "p1").unwrap(), 1);
    assert_eq!(projects::take_task_number(&c, "p1").unwrap(), 2);
    assert!(projects::take_task_number(&c, "nope").is_err());
    tasks::set_status(&c, "t1", TaskStatus::Done, 50).unwrap();
    assert_eq!(tasks::get(&c, "t1").unwrap().closed_at, Some(50));
    tasks::set_status(&c, "t1", TaskStatus::Canceled, 60).unwrap();
    assert_eq!(tasks::get(&c, "t1").unwrap().closed_at, Some(50), "keeps the first close");
    tasks::set_status(&c, "t1", TaskStatus::Todo, 70).unwrap();
    assert_eq!(tasks::get(&c, "t1").unwrap().closed_at, None);
    assert!(repos::find_by_path(&c, "/r1").unwrap().is_some());
    assert_eq!(repos::next_position(&c, "p1").unwrap(), 1);
}

/// P09: the project-filtered list queries (`board::tasks::list`, `board::repos::list`,
/// `sources::list_links`/`linked_tasks`/`moved_ids`) now branch into a query with a fixed
/// `project_id = ?1` (or equivalent) rather than a single `?1 IS NULL OR ...` one; both
/// branches must return the same rows the unfiltered/filtered call did before.
#[test]
fn project_scoped_lists_with_and_without_a_filter() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    insert_project(&c, &project_of("p1", "PAY")).unwrap();
    insert_project(&c, &project_of("p2", "WEB")).unwrap();
    insert_repo(&c, &repo_of("r1", "p1", "/r1")).unwrap();
    insert_repo(&c, &repo_of("r2", "p2", "/r2")).unwrap();

    let link = SourceLink {
        id: "s1".into(),
        project_id: "p1".into(),
        provider: "linear".into(),
        scope: ScopeRef { kind: "team".into(), id: "team-x".into(), name: "X".into() },
        default_repo_id: None,
        repo_rules: vec![],
        state_map: StateMap::default(),
        auto_import: false,
        created_at: 1,
        last_synced_at: None,
        last_sync_error: None,
        pending_state_changes: None,
    };
    crate::rows::insert_source_link(&c, &link).unwrap();

    let mut t1 = task_of("t1");
    t1.source = Some(TaskSource {
        provider: "linear".into(),
        link_id: Some("s1".into()),
        external_id: "ext-1".into(),
        identifier: "ENG-1".into(),
        url: "https://example.com/ENG-1".into(),
        external_state: None,
        last_synced_at: None,
        sync_error: None,
        unmapped: false,
        project: None,
        rule_id: None,
        moved: Some(MovedInfo {
            from_project: ExtProject { id: "proj-a".into(), name: "A".into() },
            to_project: None,
            suggested_repo_id: None,
        }),
    });
    insert_task(&c, &t1).unwrap();
    let mut t2 = task_of("t2");
    t2.project_id = "p2".into();
    t2.repo_id = "r2".into();
    t2.number = 1;
    insert_task(&c, &t2).unwrap();

    let ids = |v: Vec<Task>| v.into_iter().map(|t| t.id).collect::<Vec<_>>();
    assert_eq!(ids(tasks::list(&c, None).unwrap()), ["t1", "t2"]);
    assert_eq!(ids(tasks::list(&c, Some("p1")).unwrap()), ["t1"]);
    assert_eq!(ids(tasks::list(&c, Some("p2")).unwrap()), ["t2"]);
    assert!(tasks::list(&c, Some("nope")).unwrap().is_empty());

    let repo_ids = |v: Vec<Repo>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(repo_ids(repos::list(&c, None).unwrap()), ["r1", "r2"]);
    assert_eq!(repo_ids(repos::list(&c, Some("p1")).unwrap()), ["r1"]);

    let link_ids = |v: Vec<SourceLink>| v.into_iter().map(|l| l.id).collect::<Vec<_>>();
    assert_eq!(link_ids(sources::list_links(&c, None).unwrap()), ["s1"]);
    assert_eq!(link_ids(sources::list_links(&c, Some("p1")).unwrap()), ["s1"]);
    assert!(sources::list_links(&c, Some("p2")).unwrap().is_empty());

    let linked_ids = |v: Vec<Task>| v.into_iter().map(|t| t.id).collect::<Vec<_>>();
    assert_eq!(linked_ids(sources::linked_tasks(&c, None, 1_000_000).unwrap()), ["t1"]);
    assert_eq!(linked_ids(sources::linked_tasks(&c, Some("s1"), 1_000_000).unwrap()), ["t1"]);
    assert!(sources::linked_tasks(&c, Some("nope"), 1_000_000).unwrap().is_empty());

    assert_eq!(sources::moved_ids(&c, None).unwrap(), ["t1"]);
    assert_eq!(sources::moved_ids(&c, Some("p1")).unwrap(), ["t1"]);
    assert!(sources::moved_ids(&c, Some("p2")).unwrap().is_empty());
}

#[test]
fn chats_crud_and_order() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    let mk = |id: &str, updated: i64| Chat {
        id: id.into(),
        project_id: "p1".into(),
        repo_id: None,
        title: None,
        session_title: None,
        agent: None,
        task_id: None,
        worktree: None,
        session_id: None,
        launch: LaunchOptions::default(),
        created_at: 1,
        updated_at: updated,
        pinned_at: None,
    };
    chats::insert(&c, &mk("c1", 10)).unwrap();
    chats::insert(&c, &mk("c2", 20)).unwrap();
    let ids = |v: Vec<Chat>| v.into_iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(chats::list(&c, "p1").unwrap()), ["c2", "c1"]);
    assert!(chats::list(&c, "p2").unwrap().is_empty());
    insert_project(&c, &project_of("p2", "WEB")).unwrap();
    chats::insert(&c, &Chat { project_id: "p2".into(), ..mk("c3", 15) }).unwrap();
    assert_eq!(ids(chats::list_all(&c).unwrap()), ["c2", "c3", "c1"]);
    assert_eq!(ids(chats::list(&c, "p1").unwrap()), ["c2", "c1"]);
    chats::delete(&c, "c3").unwrap();
    chats::insert(&c, &Chat { worktree: Some("/wt".into()), ..mk("c4", 5) }).unwrap();
    assert_eq!(chats::get(&c, "c4").unwrap().worktree.as_deref(), Some("/wt"));
    chats::delete(&c, "c4").unwrap();

    let mut c1 = chats::get(&c, "c1").unwrap();
    c1.title = Some("Plan the login page".into());
    c1.repo_id = Some("r1".into());
    c1.launch.model = Some("sonnet".into());
    c1.updated_at = 30;
    chats::update(&c, &c1).unwrap();
    assert_eq!(chats::get(&c, "c1").unwrap(), c1);
    assert_eq!(ids(chats::list(&c, "p1").unwrap()), ["c1", "c2"]);

    // Pinned chats come first, oldest pin first; pinning doesn't touch `updated_at`.
    chats::insert(&c, &mk("c5", 40)).unwrap();
    assert_eq!(ids(chats::list(&c, "p1").unwrap()), ["c5", "c1", "c2"]);
    chats::set_pinned(&c, "c2", true, 100).unwrap();
    chats::set_pinned(&c, "c1", true, 200).unwrap();
    chats::set_pinned(&c, "c2", true, 300).unwrap(); // re-pinning keeps the first pin time
    assert_eq!(chats::get(&c, "c2").unwrap().pinned_at, Some(100));
    assert_eq!(chats::get(&c, "c2").unwrap().updated_at, 20);
    assert_eq!(ids(chats::list(&c, "p1").unwrap()), ["c2", "c1", "c5"]);
    assert_eq!(ids(chats::list_all(&c).unwrap()), ["c2", "c1", "c5"]);
    chats::set_pinned(&c, "c2", false, 400).unwrap();
    assert_eq!(chats::get(&c, "c2").unwrap().pinned_at, None);
    assert_eq!(ids(chats::list(&c, "p1").unwrap()), ["c1", "c5", "c2"]);
    chats::set_pinned(&c, "c1", false, 400).unwrap();
    chats::delete(&c, "c5").unwrap();
    assert!(chats::set_pinned(&c, "nope", true, 1).is_err());

    assert!(chats::set_session(&c, "c1", "s-1").unwrap());
    assert!(!chats::set_session(&c, "c1", "s-1").unwrap(), "same id: no change");
    assert_eq!(chats::get(&c, "c1").unwrap().session_id.as_deref(), Some("s-1"));
    assert!(chats::set_session_title(&c, "c1", "Login page plan").unwrap());
    assert!(!chats::set_session_title(&c, "c1", "Login page plan").unwrap(), "same title: no change");
    // `update` doesn't touch the session or its title.
    chats::update(&c, &c1).unwrap();
    let saved = chats::get(&c, "c1").unwrap();
    assert_eq!((saved.session_id.as_deref(), saved.session_title.as_deref()), (Some("s-1"), Some("Login page plan")));

    chats::delete(&c, "c1").unwrap();
    assert!(chats::get(&c, "c1").is_err());
    assert!(chats::delete(&c, "c1").is_err());
    assert!(chats::update(&c, &c1).is_err());
}

#[test]
fn hidden_executors_toggle_per_source_and_repo() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    insert_repo(&c, &repo_of("r2", "p1", "/r2")).unwrap();
    let key = |kind, source, repo: Option<&str>| HiddenExecutor {
        kind,
        source,
        name: "code-reviewer".into(),
        repo_id: repo.map(Into::into),
    };
    let user = key(HiddenKind::Agent, AgentSource::User, None);
    let r1 = key(HiddenKind::Agent, AgentSource::Repo, Some("r1"));
    let r2 = key(HiddenKind::Agent, AgentSource::Repo, Some("r2"));
    let wf = key(HiddenKind::Workflow, AgentSource::User, None);

    hidden_executors::set(&c, "p1", &r1, true).unwrap();
    hidden_executors::set(&c, "p1", &r1, true).unwrap();
    assert_eq!(hidden_executors::list(&c, "p1").unwrap(), vec![r1.clone()]);
    hidden_executors::set(&c, "p1", &user, true).unwrap();
    hidden_executors::set(&c, "p1", &wf, true).unwrap();
    hidden_executors::set(&c, "p1", &r2, true).unwrap();
    assert_eq!(hidden_executors::list(&c, "p1").unwrap(), vec![r1.clone(), r2.clone(), user.clone(), wf.clone()]);

    // Showing one leaves its same-name siblings hidden.
    hidden_executors::set(&c, "p1", &r1, false).unwrap();
    hidden_executors::set(&c, "p1", &user, false).unwrap();
    hidden_executors::set(&c, "p1", &user, false).unwrap();
    assert_eq!(hidden_executors::list(&c, "p1").unwrap(), vec![r2.clone(), wf.clone()]);

    assert!(hidden_executors::set(&c, "p1", &key(HiddenKind::Agent, AgentSource::Repo, None), true).is_err());
    assert!(hidden_executors::set(&c, "p1", &key(HiddenKind::Agent, AgentSource::Plugin, Some("r1")), true).is_err());
    assert!(hidden_executors::set(&c, "p1", &HiddenExecutor { name: " ".into(), ..user.clone() }, true).is_err());
    assert!(hidden_executors::set(&c, "p1", &HiddenExecutor { name: "x".repeat(201), ..user.clone() }, true).is_err());
    assert!(hidden_executors::set(&c, "p1", &key(HiddenKind::Agent, AgentSource::Repo, Some("nope")), true).is_err());
    assert!(hidden_executors::set(&c, "nope", &user, true).is_err());
    assert!(hidden_executors::list(&c, "nope").unwrap().is_empty());
}

#[test]
fn hidden_executor_rejects_claude_and_unknown_fields() {
    let ok: HiddenExecutor =
        serde_json::from_str(r#"{"kind":"workflow","source":"repo","name":"plan-task","repoId":"r1"}"#).unwrap();
    assert_eq!(ok.repo_id.as_deref(), Some("r1"));
    let bare: HiddenExecutor = serde_json::from_str(r#"{"kind":"agent","source":"plugin","name":"kit:sec"}"#).unwrap();
    assert_eq!(bare.repo_id, None);
    assert!(serde_json::from_str::<HiddenExecutor>(r#"{"kind":"claude","source":"user","name":"claude"}"#).is_err());
    assert!(serde_json::from_str::<HiddenExecutor>(r#"{"kind":"agent","source":"user","name":"a","extra":1}"#).is_err());
}

#[test]
fn chat_task_link_round_trips_and_unlinks_with_the_task() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    let mk = |id: &str, task: Option<&str>| Chat {
        id: id.into(),
        project_id: "p1".into(),
        repo_id: Some("r1".into()),
        title: None,
        session_title: None,
        agent: None,
        task_id: task.map(Into::into),
        worktree: None,
        session_id: None,
        launch: LaunchOptions::default(),
        created_at: 1,
        updated_at: 1,
        pinned_at: None,
    };
    chats::insert(&c, &mk("c1", Some("t1"))).unwrap();
    chats::insert(&c, &mk("c2", None)).unwrap();
    assert_eq!(chats::get(&c, "c1").unwrap().task_id.as_deref(), Some("t1"));
    assert_eq!(chats::for_task(&c, "t1").unwrap().unwrap().id, "c1");
    assert!(chats::for_task(&c, "nope").unwrap().is_none());
    assert!(chats::insert(&c, &mk("c3", Some("missing"))).is_err());

    c.execute("DELETE FROM tasks WHERE id = 't1'", []).unwrap();
    assert_eq!(chats::get(&c, "c1").unwrap().task_id, None);
}
