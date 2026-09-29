use crate::board::{hidden_executors, projects, repos, tasks};
use crate::execution::runs;
use crate::rows::{insert_project, insert_repo, insert_task};
use crate::{chats, sources, Db};
use nodal_domain::model::*;
use nodal_domain::testutil::{project_of, repo_of, run_of, task_of};

fn seed(c: &crate::Conn) {
    insert_project(c, &project_of("p1", "PAY")).unwrap();
    insert_repo(c, &repo_of("r1", "p1", "/r1")).unwrap();
    insert_task(c, &task_of("t1")).unwrap();
}

fn queued(id: &str, pos: f64) -> Run {
    let mut r = run_of(Executor::Claude, RunKind::Work, false);
    r.id = id.into();
    r.status = RunStatus::Queued;
    r.queue_position = pos;
    r.queued_at = pos as i64;
    r.claude_run_id = None;
    r.session_id = None;
    r
}

#[test]
fn queue_order_reorder_and_pending() {
    let db = Db::open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    seed(&c);
    assert_eq!(runs::next_queue_position(&c).unwrap(), 1.0);
    for (id, pos) in [("a", 1.0), ("b", 2.0), ("c", 3.0)] {
        runs::insert(&c, &queued(id, pos)).unwrap();
    }
    let mut done = queued("d", 0.5);
    done.status = RunStatus::Finished;
    done.finished_at = Some(100);
    runs::insert(&c, &done).unwrap();
    let ids = |v: Vec<Run>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(ids(runs::queue(&c).unwrap()), ["a", "b", "c"]);
    assert_eq!(ids(runs::pending(&c).unwrap()), ["a", "b", "c"]);
    assert_eq!(runs::pending_in_project(&c, "p1").unwrap(), 3);

    runs::reorder_queue(&mut c, &["c".into(), "a".into(), "b".into()]).unwrap();
    assert_eq!(ids(runs::queue(&c).unwrap()), ["c", "a", "b"]);
    // It must be exactly the queued set.
    assert!(runs::reorder_queue(&mut c, &["c".into(), "a".into()]).is_err());
    assert!(runs::reorder_queue(&mut c, &["c".into(), "a".into(), "a".into()]).is_err());
    assert!(runs::reorder_queue(&mut c, &["c".into(), "a".into(), "d".into()]).is_err());

    // Compare-and-set.
    assert!(runs::transition(&c, "a", RunStatus::Queued, RunStatus::Launching).unwrap());
    assert!(!runs::transition(&c, "a", RunStatus::Queued, RunStatus::Launching).unwrap());
    assert_eq!(runs::launching(&c).unwrap().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["a"]);
    assert!(runs::transition(&c, "a", RunStatus::Launching, RunStatus::Failed).unwrap());

    assert_eq!(runs::last_finished(&c, "t1").unwrap().unwrap().id, "d");
    assert_eq!(runs::list_filtered(&c, None, Some("t1")).unwrap().len(), 4);
    assert!(runs::launched_refs(&c).unwrap().is_empty());
    let mut b = runs::get(&c, "b").unwrap();
    b.claude_run_id = Some("abcd1234".into());
    runs::update(&c, &b).unwrap();
    assert_eq!(runs::launched_refs(&c).unwrap(), vec![(Some("abcd1234".into()), None)]);

    // Deleting the task leaves its runs with a NULL task_id.
    tasks::delete(&c, "t1").unwrap();
    assert_eq!(runs::get(&c, "b").unwrap().task_id, None);
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

#[test]
fn runs_filtered_by_project_and_latest_by_task() {
    let db = Db::open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    insert_project(&c, &project_of("p2", "WEB")).unwrap();
    insert_repo(&c, &repo_of("r2", "p2", "/r2")).unwrap();
    let mut t2 = task_of("t2");
    t2.number = 2;
    insert_task(&c, &t2).unwrap();
    let mut t3 = task_of("t3");
    t3.project_id = "p2".into();
    t3.repo_id = "r2".into();
    insert_task(&c, &t3).unwrap();
    let mk = |id: &str, task: Option<&str>, repo: &str, at: i64| {
        let mut r = queued(id, at as f64);
        r.task_id = task.map(Into::into);
        r.repo_id = Some(repo.into());
        r.status = RunStatus::Finished;
        r
    };
    for r in [
        mk("a", Some("t1"), "r1", 1),
        mk("b", Some("t1"), "r1", 3),
        mk("c", Some("t2"), "r1", 2),
        mk("d", Some("t3"), "r2", 4),
        mk("e", None, "r2", 5),
    ] {
        runs::insert(&c, &r).unwrap();
    }
    let ids = |v: Vec<Run>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(ids(runs::list_filtered(&c, None, None).unwrap()), ["e", "d", "b", "c", "a"]);
    assert_eq!(ids(runs::list_filtered(&c, Some("p1"), None).unwrap()), ["b", "c", "a"]);
    // A run without a task counts toward its repo's project.
    assert_eq!(ids(runs::list_filtered(&c, Some("p2"), None).unwrap()), ["e", "d"]);
    assert_eq!(ids(runs::list_filtered(&c, Some("p1"), Some("t2")).unwrap()), ["c"]);
    assert!(runs::list_filtered(&c, Some("p2"), Some("t1")).unwrap().is_empty());

    assert_eq!(ids(runs::latest_by_task(&c, None).unwrap()), ["d", "b", "c"]);
    assert_eq!(ids(runs::latest_by_task(&c, Some("p1")).unwrap()), ["b", "c"]);

    // P09: the `_light` queries (explicit columns, no `prompt`) return the same rows, in the
    // same order, as their `Run` counterparts.
    let light_ids = |v: Vec<RunLight>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(light_ids(runs::list_filtered_light(&c, None, None).unwrap()), ["e", "d", "b", "c", "a"]);
    assert_eq!(light_ids(runs::list_filtered_light(&c, Some("p1"), None).unwrap()), ["b", "c", "a"]);
    assert_eq!(light_ids(runs::list_filtered_light(&c, Some("p2"), None).unwrap()), ["e", "d"]);
    assert_eq!(light_ids(runs::list_filtered_light(&c, Some("p1"), Some("t2")).unwrap()), ["c"]);
    assert!(runs::list_filtered_light(&c, Some("p2"), Some("t1")).unwrap().is_empty());
    assert_eq!(light_ids(runs::latest_by_task_light(&c, None).unwrap()), ["d", "b", "c"]);
    assert_eq!(light_ids(runs::latest_by_task_light(&c, Some("p1")).unwrap()), ["b", "c"]);
    assert_eq!(runs::list_filtered_light(&c, Some("p1"), Some("t2")).unwrap()[0], RunLight::from(runs::get(&c, "c").unwrap()));

    let light = serde_json::to_value(RunLight::from(runs::get(&c, "b").unwrap())).unwrap();
    assert!(light.get("prompt").is_none() && light.get("extraInstructions").is_none());
    assert_eq!(light["taskId"], "t1");
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
        session_id: None,
        launch: LaunchOptions::default(),
        created_at: 1,
        updated_at: updated,
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

    let mut c1 = chats::get(&c, "c1").unwrap();
    c1.title = Some("Plan the login page".into());
    c1.repo_id = Some("r1".into());
    c1.launch.model = Some("sonnet".into());
    c1.updated_at = 30;
    chats::update(&c, &c1).unwrap();
    assert_eq!(chats::get(&c, "c1").unwrap(), c1);
    assert_eq!(ids(chats::list(&c, "p1").unwrap()), ["c1", "c2"]);

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
