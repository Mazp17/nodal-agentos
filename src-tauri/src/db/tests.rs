use std::collections::BTreeMap;

use rusqlite::Connection;

use super::rows::*;
use super::schema::{migrate, user_version, MIGRATIONS};
use super::*;
use crate::domain::*;

fn conn() -> Db {
    open_in_memory().unwrap()
}

fn project(id: &str, key: &str) -> Project {
    Project {
        id: id.into(),
        name: format!("Acme {key}"),
        key: key.into(),
        next_task_number: 1,
        color: "oklch(0.74 0.15 55)".into(),
        default_executor: Some(Executor::Agent { name: "frontend-developer".into(), source: AgentSource::User }),
        reviewer: Some("code-reviewer".into()),
        created_at: 1_700_000_000_000,
        archived_at: None,
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
        default_executor: Some(Executor::Workflow { name: "plan-task".into() }),
        default_isolation: Isolation::InPlace,
        default_finish: Finish::Commit,
        default_review: false,
        reviewer: None,
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
        title: "Rebrand de la web".into(),
        status: TaskStatus::Todo,
        priority: Priority::High,
        labels: vec!["frontend".into(), "brand".into()],
        position: 1.5,
        plan: PlanRef::Text,
        plan_overridden: false,
        acceptance: vec!["El logo nuevo aparece en el header".into()],
        assignee: Some(Executor::Claude),
        isolation: None,
        finish: Some(Finish::Pr),
        review: Some(true),
        worktree: None,
        source: None,
        created_at: 10,
        updated_at: 11,
        closed_at: None,
    }
}

fn imported_task(id: &str, project_id: &str, repo_id: &str, number: i64, ext: &str) -> Task {
    Task {
        plan: PlanRef::File { path: "/Users/me/Code/acme-r1/docs/plan.md".into() },
        plan_overridden: true,
        assignee: None,
        isolation: Some(Isolation::Worktree),
        review: None,
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
        "projects", "repos", "tasks", "task_relations", "runs", "source_links", "sync_outbox", "settings",
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
        let db = open(&path).unwrap();
        let c = db.lock().unwrap();
        let mode: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
    }
    // Reabrir no re-aplica nada ni falla.
    let db = open(&path).unwrap();
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
    let mut c = Connection::open_in_memory().unwrap();
    c.pragma_update(None, "foreign_keys", "ON").unwrap();
    // Base v1 con datos, escrita a mano (las columnas v2 todavía no existen).
    c.execute_batch(MIGRATIONS[0]).unwrap();
    c.pragma_update(None, "user_version", 1).unwrap();
    c.execute_batch(
        r#"INSERT INTO projects (id, name, key, color, created_at) VALUES ('p1', 'Pay', 'PAY', '#fff', 1);
         INSERT INTO repos (id, project_id, path, name, created_at) VALUES ('r1', 'p1', '/r1', 'web', 1);
         INSERT INTO source_links (id, project_id, provider, scope_kind, scope_id, scope_name, state_map_json, created_at)
           VALUES ('l1', 'p1', 'linear', 'team', 'tm', 'Eng', '{"pull":{},"push":{},"confirmedAt":null,"knownStates":[]}', 1);
         INSERT INTO tasks (id, project_id, repo_id, number, title, status, plan_kind, created_at, updated_at,
                            src_provider, src_link_id, src_external_id, src_identifier, src_url)
           VALUES ('t1', 'p1', 'r1', 1, 'x', 'todo', 'text', 1, 1, 'linear', 'l1', 'e1', 'ENG-1', 'https://example.com/1');
         INSERT INTO runs (id, cwd, executor_json, kind, prompt, finish, status, queue_position, queued_at)
           VALUES ('u1', '/r1', '{"kind":"claude"}', 'work', 'p', 'pr', 'finished', 1, 1);
         INSERT INTO settings (key, value_json) VALUES ('concurrency', '2');"#,
    )
    .unwrap();
    migrate(&mut c).unwrap();
    assert_eq!(user_version(&c).unwrap(), MIGRATIONS.len() as i64);
    assert_eq!(get_project(&c, "p1").unwrap().unwrap().description, None);
    let l = get_source_link(&c, "l1").unwrap().unwrap();
    assert_eq!((l.last_synced_at, l.last_sync_error, l.pending_state_changes), (None, None, None));
    assert!(!get_task(&c, "t1").unwrap().unwrap().source.unwrap().unmapped);
    assert_eq!(get_run(&c, "u1").unwrap().unwrap().tokens, None);
    let mut s = load_settings(&c).unwrap();
    assert_eq!((s.concurrency, s.default_executor.clone()), (2, None));

    // Los campos nuevos se guardan y se leen.
    let changes = StateChanges {
        added: vec![ExternalState { id: "s9".into(), name: "QA".into(), kind: ExtKind::Started, color: None }],
        removed: vec![],
    };
    c.execute(
        "UPDATE source_links SET pending_state_changes = ?1 WHERE id = 'l1'",
        [serde_json::to_string(&changes).unwrap()],
    )
    .unwrap();
    assert_eq!(get_source_link(&c, "l1").unwrap().unwrap().pending_state_changes, Some(changes));
    s.default_executor = Some(Executor::Workflow { name: "plan-task".into() });
    save_settings(&mut c, &s).unwrap();
    assert_eq!(load_settings(&c).unwrap().default_executor, s.default_executor);
}

#[test]
fn migrates_v2_data_to_v3() {
    let mut c = Connection::open_in_memory().unwrap();
    c.pragma_update(None, "foreign_keys", "ON").unwrap();
    c.execute_batch(MIGRATIONS[0]).unwrap();
    c.execute_batch(MIGRATIONS[1]).unwrap();
    c.pragma_update(None, "user_version", 2).unwrap();
    // Base v2: reglas con el JSON viejo y una tarea vinculada sin columnas de proyecto.
    c.execute_batch(
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
    migrate(&mut c).unwrap();
    assert_eq!(user_version(&c).unwrap(), 3);
    let src = get_task(&c, "t1").unwrap().unwrap().source.unwrap();
    assert_eq!((src.project, src.rule_id, src.moved), (None, None, None));
    let rules = get_source_link(&c, "l1").unwrap().unwrap().repo_rules;
    assert_eq!(rules.len(), 1);
    assert_eq!((rules[0].kind, rules[0].value.as_str(), rules[0].created_at), (RuleKind::Label, "frontend", 7));

    // Las columnas nuevas se escriben y se leen.
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
    let src = get_task(&c, "t1").unwrap().unwrap().source.unwrap();
    assert_eq!(src.project, Some(ExtProject { id: "pb".into(), name: "B".into() }));
    assert_eq!((src.rule_id.as_deref(), src.moved.as_ref()), (Some("rule-0"), Some(&moved)));
    let json: serde_json::Value = serde_json::to_value(&src).unwrap();
    assert_eq!(json["moved"]["fromProject"]["name"], "A");
    assert!(json["moved"]["toProject"].is_null());
    assert_eq!(json["moved"]["suggestedRepoId"], "r1");
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
    insert_run(&c, &run("run1", Some("t1"), Some("r1"))).unwrap();

    c.execute("DELETE FROM projects WHERE id = 'p1'", []).unwrap();
    for t in ["projects", "repos", "tasks", "task_relations", "source_links"] {
        assert_eq!(count(&c, t), 0, "{t}");
    }
    // El historial de runs se conserva, desvinculado.
    let r = get_run(&c, "run1").unwrap().unwrap();
    assert_eq!((r.task_id, r.repo_id), (None, None));
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

    // Otro proveedor con el mismo id externo sí puede.
    let mut other = imported_task("t3", "p1", "r1", 3, "ext-1");
    other.source.as_mut().unwrap().provider = "asana".into();
    insert_task(&c, &other).unwrap();

    // Las locales (NULL, NULL) no chocan entre sí.
    insert_task(&c, &local_task("t4", "p1", "r1", 4)).unwrap();
    insert_task(&c, &local_task("t5", "p1", "r1", 5)).unwrap();

    // El número es único dentro del proyecto.
    assert!(insert_task(&c, &local_task("t6", "p1", "r1", 5)).is_err());
}

fn run(id: &str, task: Option<&str>, repo: Option<&str>) -> Run {
    Run {
        id: id.into(),
        task_id: task.map(String::from),
        repo_id: repo.map(String::from),
        cwd: "/Users/me/.nodal/worktrees/acme-r1/pay-1".into(),
        executor: Executor::Agent { name: "code-reviewer".into(), source: AgentSource::Plugin },
        kind: RunKind::Review,
        parent_run_id: None,
        prompt: "Revisá contra los criterios".into(),
        extra_instructions: Some("Mirá también los tests".into()),
        options: LaunchOptions { model: Some("sonnet".into()), ..Default::default() },
        finish: Finish::Changes,
        isolation: Some(Isolation::Worktree),
        review: true,
        verdict: Some(Verdict {
            pass: false,
            unmet: vec!["Falta el logo".into()],
            nits: vec!["Nombre de variable".into()],
            summary: Some("Casi".into()),
        }),
        status: RunStatus::Finished,
        queue_position: 3.25,
        claude_run_id: Some("abc123".into()),
        session_id: Some("11111111-2222-3333-4444-555555555555".into()),
        queued_at: 1,
        launched_at: Some(2),
        finished_at: Some(3),
        outcome: Some(RunOutcome::Red),
        summary: Some("Reporte".into()),
        pr_url: Some("https://github.com/acme/web/pull/1".into()),
        branch: Some("nodal/pay-1".into()),
        error: None,
        legacy_label: Some("ENG-7".into()),
        tokens: None,
    }
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
    let bare = Project { id: "p2".into(), key: "WEB".into(), default_executor: None, reviewer: None, archived_at: Some(9), ..p };
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

    let work = Run {
        kind: RunKind::Work,
        executor: Executor::Workflow { name: "plan-task".into() },
        verdict: None,
        outcome: None,
        status: RunStatus::Queued,
        launched_at: None,
        finished_at: None,
        claude_run_id: None,
        session_id: None,
        extra_instructions: None,
        options: LaunchOptions::default(),
        legacy_label: None,
        isolation: None,
        review: false,
        ..run("run1", Some("t1"), Some("r1"))
    };
    insert_run(&c, &work).unwrap();
    assert_eq!(get_run(&c, "run1").unwrap().unwrap(), work);
    let review = Run { parent_run_id: Some("run1".into()), ..run("run2", Some("t1"), Some("r1")) };
    insert_run(&c, &review).unwrap();
    assert_eq!(get_run(&c, "run2").unwrap().unwrap(), review);

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
    let s = Settings { concurrency: 5, editor: Some("cursor".into()), reviewer: "strict-reviewer".into(), default_executor: None };
    save_settings(&mut c, &s).unwrap();
    assert_eq!(load_settings(&c).unwrap(), s);
    // Sobrescribir actualiza en lugar de duplicar.
    let s2 = Settings { editor: None, ..s };
    save_settings(&mut c, &s2).unwrap();
    assert_eq!(load_settings(&c).unwrap(), s2);
    assert_eq!(count(&c, "settings"), 4);
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
    assert_eq!(s.concurrency, MAX_CONCURRENCY);
    assert_eq!(s.editor, None);
    assert_eq!(s.reviewer, DEFAULT_REVIEWER);
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

    // Desvincular (la tarea queda local) y después borrar.
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
    // El inverso es la misma relación.
    assert!(insert_relation(&c, &TaskRelation { task_id: "t1".into(), other_id: "t2".into(), kind: RelationKind::Related })
        .is_err());
    // `blocks` es dirigida: se guarda como vino.
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
    let n = tauri::async_runtime::block_on(with_db(&db, |c| {
        insert_project(c, &project("p1", "PAY"))?;
        Ok(count(c, "projects"))
    }))
    .unwrap();
    assert_eq!(n, 1);
    let err = tauri::async_runtime::block_on(with_db(&db, |_| -> Result<(), DbError> {
        Err(DbError::Invalid("nope".into()))
    }))
    .unwrap_err();
    assert_eq!(String::from(err), "nope");
}
