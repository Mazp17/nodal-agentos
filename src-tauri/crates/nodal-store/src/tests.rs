use std::collections::BTreeMap;

use crate::rows::*;
use crate::schema::{migrate, user_version, MIGRATIONS};
use crate::Conn as Connection;
use crate::*;
use nodal_domain::model::*;

/// A static multi-thread tokio runtime, reused across tests (only `with_db_runs_off_the_async_thread`
/// needs it).
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::sync::OnceLock;
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().unwrap()).block_on(fut)
}

fn conn() -> Db {
    Db::open_in_memory().unwrap()
}

fn project(id: &str, key: &str) -> Project {
    Project {
        id: id.into(),
        name: format!("Acme {key}"),
        key: key.into(),
        next_task_number: 1,
        color: "oklch(0.74 0.15 55)".into(),
        created_at: 1_700_000_000_000,
        archived_at: None,
        root_path: None,
        description: None,
    }
}

fn repo(id: &str, project_id: &str) -> Repo {
    Repo {
        id: id.into(),
        project_id: project_id.into(),
        path: format!("/Users/me/Code/acme-{id}"),
        name: format!("acme-{id}"),
        launch: LaunchOptions { model: Some("opus".into()), effort: None, permission_mode: Some("acceptEdits".into()) },
        position: 2,
        created_at: 1_700_000_000_001,
    }
}

fn local_task(id: &str, project_id: &str, repo_id: &str, number: i64) -> Task {
    Task {
        id: id.into(),
        project_id: project_id.into(),
        repo_id: repo_id.into(),
        number,
        title: "Website rebrand".into(),
        status: TaskStatus::Todo,
        priority: Priority::High,
        labels: vec!["frontend".into(), "brand".into()],
        position: 1.5,
        plan: PlanRef::Text,
        plan_overridden: false,
        acceptance: vec!["The new logo shows up in the header".into()],
        worktree: None,
        source: None,
        created_at: 10,
        updated_at: 11,
        pr: None,
        closed_at: None,
    }
}

fn imported_task(id: &str, project_id: &str, repo_id: &str, number: i64, ext: &str) -> Task {
    Task {
        plan: PlanRef::File { path: "/Users/me/Code/acme-r1/docs/plan.md".into() },
        plan_overridden: true,
        worktree: Some(WorktreeRef {
            path: "/Users/me/.nodal/worktrees/acme-r1/eng-142".into(),
            branch: "nodal/eng-142".into(),
            base: "main".into(),
        }),
        source: Some(TaskSource {
            provider: "linear".into(),
            link_id: None,
            external_id: ext.into(),
            identifier: "ENG-142".into(),
            url: "https://linear.app/acme/issue/ENG-142".into(),
            external_state: Some(ExternalState {
                id: "s3".into(),
                name: "In Review".into(),
                kind: ExtKind::Started,
                color: Some("#0f783c".into()),
            }),
            last_synced_at: Some(99),
            sync_error: Some("rate limited".into()),
            unmapped: false,
            project: Some(ExtProject { id: "proj-web".into(), name: "Website".into() }),
            rule_id: Some("rule-1".into()),
            moved: Some(MovedInfo {
                from_project: ExtProject { id: "proj-old".into(), name: "Old site".into() },
                to_project: Some(ExtProject { id: "proj-web".into(), name: "Website".into() }),
                suggested_repo_id: None,
            }),
        }),
        status: TaskStatus::InReview,
        pr: None,
        closed_at: Some(123),
        ..local_task(id, project_id, repo_id, number)
    }
}

fn seed(c: &Connection) {
    insert_project(c, &project("p1", "PAY")).unwrap();
    insert_repo(c, &repo("r1", "p1")).unwrap();
}

fn count(c: &Connection, table: &str) -> i64 {
    c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

#[test]
fn schema_applies_in_memory_with_pragmas() {
    let db = conn();
    let c = db.lock().unwrap();
    assert_eq!(user_version(&c).unwrap(), MIGRATIONS.len() as i64);
    let fk: i64 = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
    assert_eq!(fk, 1);
    for t in [
        "projects", "repos", "tasks", "task_relations", "source_links", "sync_outbox", "settings",
        "legacy_imports",
    ] {
        assert_eq!(count(&c, t), 0, "{t}");
    }
    let fk_violations: i64 = c.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
    assert_eq!(fk_violations, 0);
}

#[test]
fn file_db_uses_wal() {
    let dir = std::env::temp_dir().join(format!("nodal-db-test-{}", std::process::id()));
    let path = dir.join("sub").join(DB_FILE);
    {
        let db = Db::open(&path).unwrap();
        let c = db.lock().unwrap();
        let mode: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
    }
    // Reopening neither re-applies anything nor fails.
    let db = Db::open(&path).unwrap();
    assert_eq!(user_version(&db.lock().unwrap()).unwrap(), MIGRATIONS.len() as i64);
    drop(db);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn migrate_is_idempotent_and_rejects_newer_schema() {
    let db = conn();
    let mut c = db.lock().unwrap();
    seed(&c);
    migrate(&mut c).unwrap();
    migrate(&mut c).unwrap();
    assert_eq!(user_version(&c).unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(count(&c, "projects"), 1);

    c.pragma_update(None, "user_version", 99).unwrap();
    let err = migrate(&mut c).unwrap_err().to_string();
    assert!(err.contains("newer version"), "{err}");
}

#[test]
fn migrates_v1_data_to_v2() {
    let mut raw = rusqlite::Connection::open_in_memory().unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    // v1 database with data, written by hand (the v2 columns don't exist yet).
    raw.execute_batch(MIGRATIONS[0]).unwrap();
    raw.pragma_update(None, "user_version", 1).unwrap();
    raw.execute_batch(
        r#"INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);
         INSERT INTO repos (id, project_id, path, name, created_at) VALUES ('r1', 'p1', '/r1', 'web', 1);
         INSERT INTO source_links (id, project_id, provider, scope_kind, scope_id, scope_name, state_map_json, created_at)
           VALUES ('l1', 'p1', 'linear', 'team', 'tm', 'Eng', '{"pull":{},"push":{},"confirmedAt":null,"knownStates":[]}', 1);
         INSERT INTO tasks (id, project_id, repo_id, number, title, status, plan_kind, created_at, updated_at,
                            src_provider, src_link_id, src_external_id, src_identifier, src_url)
           VALUES ('t1', 'p1', 'r1', 1, 'x', 'todo', 'text', 1, 1, 'linear', 'l1', 'e1', 'ENG-1', 'https://example.com/1');
         INSERT INTO settings (key, value_json) VALUES ('concurrency', '2');"#,
    )
    .unwrap();
    let c = Conn::wrap_mut(&mut raw);
    migrate(c).unwrap();
    assert_eq!(user_version(c).unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(get_project(c, "p1").unwrap().unwrap().description, None);
    let l = get_source_link(c, "l1").unwrap().unwrap();
    assert_eq!((l.last_synced_at, l.last_sync_error, l.pending_state_changes), (None, None, None));
    assert!(!get_task(c, "t1").unwrap().unwrap().source.unwrap().unmapped);
    assert_eq!(load_settings(c).unwrap(), Settings::default());

    // The new fields are saved and read back.
    let changes = StateChanges {
        added: vec![ExternalState { id: "s9".into(), name: "QA".into(), kind: ExtKind::Started, color: None }],
        removed: vec![],
    };
    c.execute(
        "UPDATE source_links SET pending_state_changes = ?1 WHERE id = 'l1'",
        [serde_json::to_string(&changes).unwrap()],
    )
    .unwrap();
    assert_eq!(get_source_link(c, "l1").unwrap().unwrap().pending_state_changes, Some(changes));
}

#[test]
fn migrates_v2_data_to_v3() {
    let mut raw = rusqlite::Connection::open_in_memory().unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    raw.execute_batch(MIGRATIONS[0]).unwrap();
    raw.execute_batch(MIGRATIONS[1]).unwrap();
    raw.pragma_update(None, "user_version", 2).unwrap();
    // v2 database: rules with the old JSON and a linked task without project columns.
    raw.execute_batch(
        r#"INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);
         INSERT INTO repos (id, project_id, path, name, created_at) VALUES ('r1', 'p1', '/r1', 'web', 1);
         INSERT INTO source_links (id, project_id, provider, scope_kind, scope_id, scope_name, repo_rules_json,
                                   state_map_json, created_at)
           VALUES ('l1', 'p1', 'linear', 'team', 'tm', 'Eng', '[{"label":"frontend","repoId":"r1"}]',
                   '{"pull":{},"push":{},"confirmedAt":null,"knownStates":[]}', 7);
         INSERT INTO tasks (id, project_id, repo_id, number, title, status, plan_kind, created_at, updated_at,
                            src_provider, src_link_id, src_external_id, src_identifier, src_url)
           VALUES ('t1', 'p1', 'r1', 1, 'x', 'todo', 'text', 1, 1, 'linear', 'l1', 'e1', 'ENG-1', 'https://example.com/1');"#,
    )
    .unwrap();
    let c = Conn::wrap_mut(&mut raw);
    migrate(c).unwrap();
    assert_eq!(user_version(c).unwrap(), MIGRATIONS.len() as i64);
    let src = get_task(c, "t1").unwrap().unwrap().source.unwrap();
    assert_eq!((src.project, src.rule_id, src.moved), (None, None, None));
    let rules = get_source_link(c, "l1").unwrap().unwrap().repo_rules;
    assert_eq!(rules.len(), 1);
    assert_eq!((rules[0].kind, rules[0].value.as_str(), rules[0].created_at), (RuleKind::Label, "frontend", 7));

    // The new columns are written and read back.
    let moved = MovedInfo {
        from_project: ExtProject { id: "pa".into(), name: "A".into() },
        to_project: None,
        suggested_repo_id: Some("r1".into()),
    };
    c.execute(
        "UPDATE tasks SET src_project_id = 'pb', src_project_name = 'B', src_rule_id = 'rule-0', src_moved = ?1",
        [serde_json::to_string(&moved).unwrap()],
    )
    .unwrap();
    let src = get_task(c, "t1").unwrap().unwrap().source.unwrap();
    assert_eq!(src.project, Some(ExtProject { id: "pb".into(), name: "B".into() }));
    assert_eq!((src.rule_id.as_deref(), src.moved.as_ref()), (Some("rule-0"), Some(&moved)));
    let json: serde_json::Value = serde_json::to_value(&src).unwrap();
    assert_eq!(json["moved"]["fromProject"]["name"], "A");
    assert!(json["moved"]["toProject"].is_null());
    assert_eq!(json["moved"]["suggestedRepoId"], "r1");
}

#[test]
fn migrates_v3_data_to_v4() {
    let mut raw = rusqlite::Connection::open_in_memory().unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    for sql in &MIGRATIONS[..3] {
        raw.execute_batch(sql).unwrap();
    }
    raw.pragma_update(None, "user_version", 3).unwrap();
    raw.execute_batch(
        r#"INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);
         INSERT INTO repos (id, project_id, path, name, created_at) VALUES ('r1', 'p1', '/r1', 'web', 1);"#,
    )
    .unwrap();
    let c = Conn::wrap_mut(&mut raw);
    migrate(c).unwrap();
    assert_eq!(user_version(c).unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(count(c, "chats"), 0);
    assert_eq!(get_repo(c, "r1").unwrap().unwrap().name, "web");

    let chat = Chat {
        id: "c1".into(),
        project_id: "p1".into(),
        repo_id: Some("r1".into()),
        title: Some("Plan the login page".into()),
        session_title: None,
        agent: None,
        task_id: None,
        worktree: None,
        session_id: Some("00000000-0000-4000-8000-000000000001".into()),
        launch: LaunchOptions { model: Some("opus".into()), effort: Some("high".into()), permission_mode: Some("plan".into()) },
        created_at: 5,
        updated_at: 6,
        pinned_at: None,
    };
    insert_chat(c, &chat).unwrap();
    assert_eq!(get_chat(c, "c1").unwrap(), Some(chat));
}

#[test]
fn migrates_v5_data_to_v6() {
    let mut raw = rusqlite::Connection::open_in_memory().unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    for sql in &MIGRATIONS[..5] {
        raw.execute_batch(sql).unwrap();
    }
    raw.pragma_update(None, "user_version", 5).unwrap();
    raw.execute_batch("INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);")
        .unwrap();
    let c = Conn::wrap_mut(&mut raw);
    migrate(c).unwrap();
    assert_eq!(user_version(c).unwrap(), MIGRATIONS.len() as i64);
    let p = get_project(c, "p1").unwrap().unwrap();
    assert_eq!((p.name.as_str(), p.root_path), ("Pay", None));

    let q = Project { root_path: Some("/Users/me/Code/acme".into()), ..project("p2", "ACME") };
    insert_project(c, &q).unwrap();
    assert_eq!(get_project(c, "p2").unwrap(), Some(q));
}

#[test]
fn migrates_v6_data_to_v7() {
    let mut raw = rusqlite::Connection::open_in_memory().unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    for sql in &MIGRATIONS[..6] {
        raw.execute_batch(sql).unwrap();
    }
    raw.pragma_update(None, "user_version", 6).unwrap();
    raw.execute_batch(
        r#"INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);
         INSERT INTO repos (id, project_id, path, name, created_at) VALUES ('r1', 'p1', '/r1', 'web', 1);"#,
    )
    .unwrap();
    let c = Conn::wrap_mut(&mut raw);
    migrate(c).unwrap();
    assert_eq!(user_version(c).unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(count(c, "project_hidden_executors"), 0);
    assert_eq!(get_repo(c, "r1").unwrap().unwrap().name, "web");
    c.execute(
        "INSERT INTO project_hidden_executors (project_id, kind, source, name, repo_id)
         VALUES ('p1', 'agent', 'repo', 'code-reviewer', 'r1')",
        [],
    )
    .unwrap();
    assert_eq!(count(c, "project_hidden_executors"), 1);
}

#[test]
fn migrates_v8_chats_to_v9_without_a_task() {
    let mut raw = rusqlite::Connection::open_in_memory().unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    for sql in &MIGRATIONS[..8] {
        raw.execute_batch(sql).unwrap();
    }
    raw.pragma_update(None, "user_version", 8).unwrap();
    raw.execute_batch(
        r#"INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);
         INSERT INTO chats (id, project_id, created_at, updated_at) VALUES ('c1', 'p1', 1, 1);"#,
    )
    .unwrap();
    let c = Conn::wrap_mut(&mut raw);
    migrate(c).unwrap();
    assert_eq!(user_version(c).unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(get_chat(c, "c1").unwrap().unwrap().task_id, None);
}

#[test]
fn migrates_v9_chats_to_v10_without_a_worktree() {
    let mut raw = rusqlite::Connection::open_in_memory().unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    for sql in &MIGRATIONS[..9] {
        raw.execute_batch(sql).unwrap();
    }
    raw.pragma_update(None, "user_version", 9).unwrap();
    raw.execute_batch(
        r#"INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);
         INSERT INTO chats (id, project_id, created_at, updated_at) VALUES ('c1', 'p1', 1, 1);"#,
    )
    .unwrap();
    let c = Conn::wrap_mut(&mut raw);
    migrate(c).unwrap();
    assert_eq!(user_version(c).unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(get_chat(c, "c1").unwrap().unwrap().worktree, None);
}

#[test]
fn migrates_v10_tasks_to_v11_without_a_pr_then_links_one() {
    let mut raw = rusqlite::Connection::open_in_memory().unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    for sql in &MIGRATIONS[..10] {
        raw.execute_batch(sql).unwrap();
    }
    raw.pragma_update(None, "user_version", 10).unwrap();
    raw.execute_batch(
        r#"INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);
         INSERT INTO repos (id, project_id, path, name, created_at) VALUES ('r1', 'p1', '/r1', 'web', 1);
         INSERT INTO tasks (id, project_id, repo_id, number, title, status, plan_kind, created_at, updated_at)
             VALUES ('t1', 'p1', 'r1', 1, 'T', 'todo', 'text', 1, 1);"#,
    )
    .unwrap();
    let c = Conn::wrap_mut(&mut raw);
    migrate(c).unwrap();
    assert_eq!(user_version(c).unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(get_task(c, "t1").unwrap().unwrap().pr, None);
    assert_eq!(board::tasks::pr_checked_at(c, "t1").unwrap(), None);

    // `None` only stamps the check time and keeps a linked PR.
    let pr = TaskPr { url: "https://github.com/o/r/pull/7".into(), number: 7, state: PrState::Draft, head: Some("abc123".into()) };
    board::tasks::set_pr(c, "t1", Some(&pr), 5).unwrap();
    board::tasks::set_pr(c, "t1", None, 9).unwrap();
    assert_eq!(get_task(c, "t1").unwrap().unwrap().pr, Some(pr));
    assert_eq!(board::tasks::pr_checked_at(c, "t1").unwrap(), Some(9));
}

#[test]
fn migrates_v7_data_to_v8() {
    let mut raw = rusqlite::Connection::open_in_memory().unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    for sql in &MIGRATIONS[..7] {
        raw.execute_batch(sql).unwrap();
    }
    raw.pragma_update(None, "user_version", 7).unwrap();
    raw.execute_batch(
        r#"INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);
         INSERT INTO chats (id, project_id, created_at, updated_at) VALUES ('c1', 'p1', 1, 1);"#,
    )
    .unwrap();
    let c = Conn::wrap_mut(&mut raw);
    migrate(c).unwrap();
    assert_eq!(user_version(c).unwrap(), MIGRATIONS.len() as i64);
    let mut chat = get_chat(c, "c1").unwrap().unwrap();
    assert_eq!(chat.agent, None);
    chat.agent = Some("planner".into());
    crate::chats::update(c, &chat).unwrap();
    assert_eq!(get_chat(c, "c1").unwrap().unwrap().agent.as_deref(), Some("planner"));
}

#[test]
fn hidden_executors_follow_their_project_and_repo() {
    let db = conn();
    let c = db.lock().unwrap();
    seed(&c);
    insert_project(&c, &project("p2", "OPS")).unwrap();
    let ins = |project: &str, kind: &str, source: &str, repo: Option<&str>| {
        c.execute(
            "INSERT INTO project_hidden_executors (project_id, kind, source, name, repo_id) VALUES (?1, ?2, ?3, 'x', ?4)",
            rusqlite::params![project, kind, source, repo],
        )
    };
    ins("p1", "agent", "repo", Some("r1")).unwrap();
    ins("p1", "agent", "user", None).unwrap();
    ins("p1", "workflow", "user", None).unwrap();
    // A NULL repo doesn't let the same entry repeat.
    assert!(ins("p1", "agent", "user", None).is_err());
    // The repo must belong to the project, and only repo-level entries carry one.
    assert!(ins("p2", "agent", "repo", Some("r1")).is_err());
    assert!(ins("p1", "agent", "user", Some("r1")).is_err());
    assert!(ins("p1", "agent", "repo", None).is_err());
    assert!(ins("p1", "claude", "user", None).is_err());
    assert!(ins("missing", "agent", "user", None).is_err());

    c.execute("DELETE FROM repos WHERE id = 'r1'", []).unwrap();
    assert_eq!(count(&c, "project_hidden_executors"), 2);
    c.execute("DELETE FROM projects WHERE id = 'p1'", []).unwrap();
    assert_eq!(count(&c, "project_hidden_executors"), 0);
}

#[test]
fn chats_follow_their_project_and_widen_when_their_repo_goes() {
    let db = conn();
    let c = db.lock().unwrap();
    seed(&c);
    let chat = |id: &str, repo: Option<&str>| Chat {
        id: id.into(),
        project_id: "p1".into(),
        repo_id: repo.map(Into::into),
        title: None,
        session_title: None,
        agent: None,
        task_id: None,
        worktree: None,
        session_id: None,
        launch: LaunchOptions::default(),
        created_at: 1,
        updated_at: 1,
        pinned_at: None,
    };
    insert_chat(&c, &chat("c1", Some("r1"))).unwrap();
    insert_chat(&c, &chat("c2", None)).unwrap();
    assert!(insert_chat(&c, &Chat { project_id: "missing".into(), ..chat("c3", None) }).is_err());

    // A chat doesn't block deleting its repo: it goes back to the whole project.
    c.execute("DELETE FROM repos WHERE id = 'r1'", []).unwrap();
    assert_eq!(get_chat(&c, "c1").unwrap().unwrap().repo_id, None);

    c.execute("DELETE FROM projects WHERE id = 'p1'", []).unwrap();
    assert_eq!(count(&c, "chats"), 0);
}

#[test]
fn deleting_project_cascades_repos_tasks_and_links() {
    let db = conn();
    let c = db.lock().unwrap();
    seed(&c);
    insert_repo(&c, &Repo { path: "/Users/me/Code/acme-r2".into(), ..repo("r2", "p1") }).unwrap();
    insert_task(&c, &local_task("t1", "p1", "r1", 1)).unwrap();
    insert_task(&c, &local_task("t2", "p1", "r2", 2)).unwrap();
    insert_relation(&c, &TaskRelation { task_id: "t1".into(), other_id: "t2".into(), kind: RelationKind::Blocks })
        .unwrap();
    insert_source_link(&c, &link("l1", "p1", Some("r1"))).unwrap();
    let mut linked = imported_task("t3", "p1", "r2", 3, "ext-1");
    linked.source.as_mut().unwrap().link_id = Some("l1".into());
    insert_task(&c, &linked).unwrap();

    c.execute("DELETE FROM projects WHERE id = 'p1'", []).unwrap();
    for t in ["projects", "repos", "tasks", "task_relations", "source_links"] {
        assert_eq!(count(&c, t), 0, "{t}");
    }
}

#[test]
fn deleting_repo_with_tasks_is_restricted() {
    let db = conn();
    let c = db.lock().unwrap();
    seed(&c);
    insert_task(&c, &local_task("t1", "p1", "r1", 1)).unwrap();
    let err = c.execute("DELETE FROM repos WHERE id = 'r1'", []).unwrap_err();
    assert!(err.to_string().contains("FOREIGN KEY"), "{err}");
    assert_eq!(count(&c, "repos"), 1);

    c.execute("DELETE FROM tasks", []).unwrap();
    c.execute("DELETE FROM repos WHERE id = 'r1'", []).unwrap();
    assert_eq!(count(&c, "repos"), 0);
}

#[test]
fn foreign_keys_are_enforced_on_insert() {
    let db = conn();
    let c = db.lock().unwrap();
    assert!(insert_repo(&c, &repo("r1", "missing")).is_err());
    seed(&c);
    assert!(insert_task(&c, &local_task("t1", "p1", "missing", 1)).is_err());
}

#[test]
fn external_ids_are_unique_per_provider() {
    let db = conn();
    let c = db.lock().unwrap();
    seed(&c);
    insert_task(&c, &imported_task("t1", "p1", "r1", 1, "ext-1")).unwrap();
    let err = insert_task(&c, &imported_task("t2", "p1", "r1", 2, "ext-1")).unwrap_err();
    assert!(err.to_string().contains("UNIQUE"), "{err}");

    // Another provider with the same external id is allowed.
    let mut other = imported_task("t3", "p1", "r1", 3, "ext-1");
    other.source.as_mut().unwrap().provider = "asana".into();
    insert_task(&c, &other).unwrap();

    // Local ones (NULL, NULL) don't clash with each other.
    insert_task(&c, &local_task("t4", "p1", "r1", 4)).unwrap();
    insert_task(&c, &local_task("t5", "p1", "r1", 5)).unwrap();

    // The number is unique within the project.
    assert!(insert_task(&c, &local_task("t6", "p1", "r1", 5)).is_err());
}

fn link(id: &str, project_id: &str, default_repo: Option<&str>) -> SourceLink {
    let mut pull = BTreeMap::new();
    pull.insert("s1".to_string(), TaskStatus::Backlog);
    pull.insert("s3".to_string(), TaskStatus::InReview);
    let mut push = BTreeMap::new();
    push.insert(TaskStatus::InReview, Some("s3".to_string()));
    push.insert(TaskStatus::Blocked, None);
    SourceLink {
        id: id.into(),
        project_id: project_id.into(),
        provider: "linear".into(),
        scope: ScopeRef { kind: "team".into(), id: "team-1".into(), name: "Engineering".into() },
        default_repo_id: default_repo.map(String::from),
        repo_rules: vec![RepoRule::label("frontend", "r1")],
        state_map: StateMap {
            pull,
            push,
            confirmed_at: Some(5),
            known_states: vec![ExternalState { id: "s1".into(), name: "Backlog".into(), kind: ExtKind::Backlog, color: None }],
        },
        auto_import: true,
        created_at: 7,
        last_synced_at: None,
        last_sync_error: None,
        pending_state_changes: None,
    }
}

#[test]
fn round_trips() {
    let db = conn();
    let c = db.lock().unwrap();

    let p = project("p1", "PAY");
    insert_project(&c, &p).unwrap();
    assert_eq!(get_project(&c, "p1").unwrap().unwrap(), p);
    let bare = Project { id: "p2".into(), key: "WEB".into(), archived_at: Some(9), ..p };
    insert_project(&c, &bare).unwrap();
    assert_eq!(get_project(&c, "p2").unwrap().unwrap(), bare);
    assert_eq!(get_project(&c, "nope").unwrap(), None);

    let r = repo("r1", "p1");
    insert_repo(&c, &r).unwrap();
    assert_eq!(get_repo(&c, "r1").unwrap().unwrap(), r);

    let l = link("l1", "p1", Some("r1"));
    insert_source_link(&c, &l).unwrap();
    assert_eq!(get_source_link(&c, "l1").unwrap().unwrap(), l);

    let t = local_task("t1", "p1", "r1", 1);
    insert_task(&c, &t).unwrap();
    assert_eq!(get_task(&c, "t1").unwrap().unwrap(), t);

    let mut imp = imported_task("t2", "p1", "r1", 2, "ext-1");
    imp.source.as_mut().unwrap().link_id = Some("l1".into());
    insert_task(&c, &imp).unwrap();
    assert_eq!(get_task(&c, "t2").unwrap().unwrap(), imp);

    let rel = TaskRelation { task_id: "t1".into(), other_id: "t2".into(), kind: RelationKind::Related };
    insert_relation(&c, &rel).unwrap();
    assert_eq!(relations_of(&c, "t2").unwrap(), vec![rel]);

    let o = OutboxItem {
        id: 0,
        task_id: "t2".into(),
        provider: "linear".into(),
        payload: OutboxPayload::SetState { state_id: "s3".into() },
        attempts: 2,
        next_attempt_at: 50,
        last_error: Some("503".into()),
        created_at: 40,
    };
    let id = insert_outbox(&c, &o).unwrap();
    assert_eq!(get_outbox(&c, id).unwrap().unwrap(), OutboxItem { id, ..o });
    let kind: String = c.query_row("SELECT kind FROM sync_outbox WHERE id = ?1", [id], |r| r.get(0)).unwrap();
    assert_eq!(kind, "set_state");
}

#[test]
fn settings_round_trip_and_defaults() {
    let db = conn();
    let mut c = db.lock().unwrap();
    assert_eq!(load_settings(&c).unwrap(), Settings::default());
    let s = Settings { editor: Some("cursor".into()), chat_system_prompt: Some("Be brief.".into()) };
    save_settings(&mut c, &s).unwrap();
    assert_eq!(load_settings(&c).unwrap(), s);
    // Overwriting updates instead of duplicating.
    let s2 = Settings { editor: None, chat_system_prompt: None };
    save_settings(&mut c, &s2).unwrap();
    assert_eq!(load_settings(&c).unwrap(), s2);
    assert_eq!(count(&c, "settings"), 2);
}

#[test]
fn corrupt_settings_fall_back_per_field() {
    let db = conn();
    let c = db.lock().unwrap();
    c.execute_batch(
        "INSERT INTO settings VALUES ('concurrency', '999');
         INSERT INTO settings VALUES ('editor', '{not json');
         INSERT INTO settings VALUES ('reviewer', '42');
         INSERT INTO settings VALUES ('unknownKey', '\"x\"');",
    )
    .unwrap();
    let s = load_settings(&c).unwrap();
    assert_eq!(s.editor, None);
}

#[test]
fn task_repo_must_belong_to_task_project() {
    let db = conn();
    let c = db.lock().unwrap();
    seed(&c);
    insert_project(&c, &project("p2", "WEB")).unwrap();
    let err = insert_task(&c, &local_task("t1", "p2", "r1", 1)).unwrap_err();
    assert!(err.to_string().contains("FOREIGN KEY"), "{err}");
}

#[test]
fn source_link_with_linked_tasks_cannot_be_deleted() {
    let db = conn();
    let c = db.lock().unwrap();
    seed(&c);
    insert_source_link(&c, &link("l1", "p1", Some("r1"))).unwrap();
    let mut t = imported_task("t1", "p1", "r1", 1, "ext-1");
    t.source.as_mut().unwrap().link_id = Some("l1".into());
    insert_task(&c, &t).unwrap();
    assert!(c.execute("DELETE FROM source_links WHERE id = 'l1'", []).is_err());

    // Unlink (the task becomes local) and then delete.
    c.execute(
        "UPDATE tasks SET src_provider = NULL, src_link_id = NULL, src_external_id = NULL,
                src_identifier = NULL, src_url = NULL, src_state_json = NULL WHERE id = 't1'",
        [],
    )
    .unwrap();
    c.execute("DELETE FROM source_links WHERE id = 'l1'", []).unwrap();
    assert_eq!(get_task(&c, "t1").unwrap().unwrap().source, None);
}

#[test]
fn inconsistent_task_rows_are_rejected() {
    let db = conn();
    let c = db.lock().unwrap();
    seed(&c);
    insert_task(&c, &local_task("t1", "p1", "r1", 1)).unwrap();
    for bad in [
        "UPDATE tasks SET plan_kind = 'file' WHERE id = 't1'",
        "UPDATE tasks SET plan_kind = 'other' WHERE id = 't1'",
        "UPDATE tasks SET wt_path = '/x' WHERE id = 't1'",
        "UPDATE tasks SET src_provider = 'linear', src_external_id = 'e' WHERE id = 't1'",
        "UPDATE projects SET key = 'pay' WHERE id = 'p1'",
    ] {
        assert!(c.execute(bad, []).is_err(), "{bad}");
    }
}

#[test]
fn related_relations_are_stored_once() {
    let db = conn();
    let c = db.lock().unwrap();
    seed(&c);
    insert_task(&c, &local_task("t1", "p1", "r1", 1)).unwrap();
    insert_task(&c, &local_task("t2", "p1", "r1", 2)).unwrap();
    insert_relation(&c, &TaskRelation { task_id: "t2".into(), other_id: "t1".into(), kind: RelationKind::Related })
        .unwrap();
    // The inverse is the same relation.
    assert!(insert_relation(&c, &TaskRelation { task_id: "t1".into(), other_id: "t2".into(), kind: RelationKind::Related })
        .is_err());
    // `blocks` is directed: stored as given.
    insert_relation(&c, &TaskRelation { task_id: "t2".into(), other_id: "t1".into(), kind: RelationKind::Blocks })
        .unwrap();
    let rels = relations_of(&c, "t1").unwrap();
    assert_eq!(rels.len(), 2);
    assert!(rels.contains(&TaskRelation { task_id: "t1".into(), other_id: "t2".into(), kind: RelationKind::Related }));
}

#[test]
fn legacy_imports_key_is_unique() {
    let db = conn();
    let c = db.lock().unwrap();
    let sql = "INSERT INTO legacy_imports (source_key, kind, target_id, imported_at) VALUES ('task:1', 'task', 't1', 1)";
    c.execute(sql, []).unwrap();
    assert!(c.execute(sql, []).is_err());
}

#[test]
fn with_db_runs_off_the_async_thread() {
    let db = conn();
    let n = block_on(with_db(&db, |c| {
        insert_project(c, &project("p1", "PAY"))?;
        Ok(count(c, "projects"))
    }))
    .unwrap();
    assert_eq!(n, 1);
    let err = block_on(with_db(&db, |_| -> Result<(), DbError> { Err(DbError::Invalid("nope".into())) })).unwrap_err();
    assert_eq!(String::from(err), "nope");
}
