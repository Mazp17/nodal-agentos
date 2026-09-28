use super::*;
use crate::db::open_in_memory;
use crate::util::paths::tests::TempDir;
use serde_json::json;

struct Fx {
    _t: TempDir,
    env: Env,
    repo_dir: PathBuf,
}

fn fx(name: &str) -> Fx {
    let t = TempDir::new(name);
    let repo_dir = t.0.join("web");
    std::fs::create_dir_all(repo_dir.join("docs")).unwrap();
    std::fs::write(repo_dir.join("docs/plan.md"), "# Repo plan").unwrap();
    let env = Env { data_dir: t.0.join("data"), worktrees_root: t.0.join("wt"), claude_dir: None };
    Fx { _t: t, env, repo_dir }
}

fn new_task(project: &Project, repo: &Repo, title: &str) -> NewTask {
    serde_json::from_value(json!({
        "projectId": project.id, "repoId": repo.id, "title": title,
        "plan": {"kind": "text", "text": "# Plan\ndo something"},
        "acceptance": ["The logo shows up", "  "], "labels": ["ui"], "priority": "high"
    }))
    .unwrap()
}

#[test]
fn projects_crud_and_unique_keys() {
    let db = open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    let p = create_project(&c, &NewProject { name: "Payments".into(), key: "pay".into(), color: None, description: None, root_path: None }, 10).unwrap();
    assert_eq!(p.key, "PAY");
    assert_eq!(p.color, validate::PALETTE[0]);
    let err = create_project(&c, &NewProject { name: "Other".into(), key: "PAY".into(), color: None, description: None, root_path: None }, 11).unwrap_err();
    assert!(err.contains("already used"), "{err}");
    let q = create_project(&c, &NewProject { name: "Web".into(), key: "WEB".into(), color: Some("#123456".into()), description: None, root_path: None }, 12).unwrap();
    let err = update_project(&c, &q.id, &ProjectPatch { key: Some("pay".into()), ..Default::default() }, 13).unwrap_err();
    assert!(err.contains("already used"));
    let patch: ProjectPatch = serde_json::from_value(json!({"name": "Web 2", "reviewer": "code-reviewer", "archived": true})).unwrap();
    let q = update_project(&c, &q.id, &patch, 14).unwrap();
    assert_eq!((q.name.as_str(), q.reviewer.as_deref(), q.archived_at), ("Web 2", Some("code-reviewer"), Some(14)));
    let patch: ProjectPatch = serde_json::from_value(json!({"reviewer": null, "archived": false})).unwrap();
    let q = update_project(&c, &q.id, &patch, 15).unwrap();
    assert_eq!((q.reviewer, q.archived_at), (None, None));
    let patch: ProjectPatch = serde_json::from_value(json!({"description": "  Payments and billing  "})).unwrap();
    assert_eq!(update_project(&c, &q.id, &patch, 15).unwrap().description.as_deref(), Some("Payments and billing"));
    let patch: ProjectPatch = serde_json::from_value(json!({"description": null})).unwrap();
    assert_eq!(update_project(&c, &q.id, &patch, 15).unwrap().description, None);
    let d = NewProject { name: "Api".into(), key: "API".into(), color: None, description: Some("Backend".into()), root_path: None };
    assert_eq!(create_project(&c, &d, 15).unwrap().description.as_deref(), Some("Backend"));
    assert_eq!(projects::list(&c, false).unwrap().len(), 3);
    assert!(delete_project(&mut c, &q.id).unwrap().is_empty());
    assert!(update_project(&c, &q.id, &ProjectPatch::default(), 16).unwrap_err().contains("no longer exists"));
}

#[test]
fn project_root_path_is_an_existing_canonical_folder_and_clearable() {
    let t = TempDir::new("ops-root-path");
    let root = t.0.join("acme");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(t.0.join("notes.md"), "# Notes").unwrap();
    let db = open_in_memory().unwrap();
    let c = db.lock().unwrap();
    let input = |root_path: Option<String>| NewProject {
        name: "Acme".into(),
        key: "ACME".into(),
        color: None,
        description: None,
        root_path,
    };
    let messy = format!("  {}/docs/..  ", root.display());
    let p = create_project(&c, &input(Some(messy)), 1).unwrap();
    let canonical = root.to_string_lossy().into_owned();
    assert_eq!(p.root_path.as_deref(), Some(canonical.as_str()));
    assert_eq!(projects::get(&c, &p.id).unwrap().root_path.as_deref(), Some(canonical.as_str()));
    let err = create_project(&c, &input(Some("/no/such/acme".into())), 2).unwrap_err();
    assert!(err.contains("doesn't exist"), "{err}");

    let patch = |v: serde_json::Value| -> ProjectPatch { serde_json::from_value(json!({ "rootPath": v })).unwrap() };
    let file = t.0.join("notes.md").to_string_lossy().into_owned();
    assert!(update_project(&c, &p.id, &patch(json!(file)), 3).unwrap_err().contains("doesn't exist"));
    assert!(update_project(&c, &p.id, &patch(json!("relative/acme")), 3).unwrap_err().contains("absolute"));
    assert_eq!(projects::get(&c, &p.id).unwrap().root_path.as_deref(), Some(canonical.as_str()));
    let untouched = update_project(&c, &p.id, &ProjectPatch { name: Some("Acme 2".into()), ..Default::default() }, 4).unwrap();
    assert_eq!(untouched.root_path.as_deref(), Some(canonical.as_str()));
    assert_eq!(update_project(&c, &p.id, &patch(json!(null)), 5).unwrap().root_path, None);
    assert_eq!(projects::get(&c, &p.id).unwrap().root_path, None);
    let docs = root.join("docs").to_string_lossy().into_owned();
    assert_eq!(update_project(&c, &p.id, &patch(json!(docs)), 6).unwrap().root_path.as_deref(), Some(docs.as_str()));
    assert_eq!(update_project(&c, &p.id, &patch(json!("  ")), 7).unwrap().root_path, None);
}

#[test]
fn repos_unique_path_options_and_delete_guard() {
    let f = fx("ops-repos");
    let db = open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    let p = create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None, description: None, root_path: None }, 1).unwrap();
    let q = create_project(&c, &NewProject { name: "Web".into(), key: "WEB".into(), color: None, description: None, root_path: None }, 1).unwrap();
    let input: NewRepo = serde_json::from_value(json!({"path": "x", "model": "opus", "effort": "turbo"})).unwrap();
    assert!(add_repo(&c, &p.id, &input, &f.repo_dir, 2).unwrap_err().contains("Invalid effort"));
    let input: NewRepo = serde_json::from_value(json!({"path": "x", "model": " opus ", "defaultIsolation": "in_place"})).unwrap();
    let r = add_repo(&c, &p.id, &input, &f.repo_dir, 2).unwrap();
    assert_eq!((r.name.as_str(), r.launch.model.as_deref(), r.default_isolation), ("web", Some("opus"), Isolation::InPlace));
    assert!(r.default_review);
    let err = add_repo(&c, &q.id, &input, &f.repo_dir, 3).unwrap_err();
    assert!(err.contains("already added to Pay"), "{err}");
    let patch: RepoPatch = serde_json::from_value(json!({"model": null, "reviewer": "sec-reviewer", "defaultFinish": "commit"})).unwrap();
    let r = update_repo(&c, &r.id, &patch).unwrap();
    assert_eq!((r.launch.model.as_deref(), r.reviewer.as_deref(), r.default_finish), (None, Some("sec-reviewer"), Finish::Commit));
    let t = create_task(&mut c, &f.env, &new_task(&p, &r, "One"), 4).unwrap();
    assert!(delete_repo(&c, &r.id).unwrap_err().contains("has 1 task"));
    // With a worktree neither the task nor the project is deleted (they'd be orphaned).
    let mut with_wt = tasks::get(&c, &t.id).unwrap();
    with_wt.worktree = Some(WorktreeRef { path: "/wt/pay-1".into(), branch: "nodal/pay-1".into(), base: "main".into() });
    tasks::update(&c, &with_wt).unwrap();
    assert!(delete_task(&c, &f.env, &t.id).unwrap_err().contains("clean up the worktree first"));
    assert!(delete_project(&mut c, &p.id).unwrap_err().contains("Clean up the worktree"));
    with_wt.worktree = None;
    tasks::update(&c, &with_wt).unwrap();
    delete_task(&c, &f.env, &t.id).unwrap();
    delete_repo(&c, &r.id).unwrap();
}

#[test]
fn tasks_numbering_plan_move_and_patch() {
    let f = fx("ops-tasks");
    let db = open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    let p = create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None, description: None, root_path: None }, 1).unwrap();
    let r = add_repo(&c, &p.id, &NewRepo::default(), &f.repo_dir, 2).unwrap();
    let other_dir = f.repo_dir.parent().unwrap().join("api");
    std::fs::create_dir_all(&other_dir).unwrap();
    let r2 = add_repo(&c, &p.id, &NewRepo::default(), &other_dir, 2).unwrap();
    let t1 = create_task(&mut c, &f.env, &new_task(&p, &r, "First"), 3).unwrap();
    let t2 = create_task(&mut c, &f.env, &new_task(&p, &r, "Second"), 4).unwrap();
    assert_eq!((t1.number, t2.number), (1, 2));
    assert_eq!(t1.acceptance, ["The logo shows up"]);
    assert_eq!((t1.status, t1.priority), (TaskStatus::Todo, Priority::High));
    assert!(t2.position > t1.position);
    assert_eq!(read_task_plan(&c, &f.env, &t1.id).unwrap(), "# Plan\ndo something");
    assert_eq!(projects::get(&c, &p.id).unwrap().next_task_number, 3);

    // Plan as a repo file (relative), and back to text.
    let patch: TaskPatch = serde_json::from_value(json!({"plan": {"kind": "file", "path": "docs/plan.md"}})).unwrap();
    let t = update_task(&mut c, &f.env, &t1.id, &patch, 5).unwrap();
    assert!(matches!(&t.plan, PlanRef::File { path } if path.ends_with("docs/plan.md")));
    assert!(!f.env.text_plan_path(&t1.id).exists());
    assert_eq!(read_task_plan(&c, &f.env, &t1.id).unwrap(), "# Repo plan");
    let bad: TaskPatch = serde_json::from_value(json!({"plan": {"kind": "file", "path": "../api/x.md"}})).unwrap();
    assert!(update_task(&mut c, &f.env, &t1.id, &bad, 6).is_err());

    // Moving repos: with a plan from the old repo, another plan must be given.
    let mv: TaskPatch = serde_json::from_value(json!({"repoId": r2.id})).unwrap();
    assert!(update_task(&mut c, &f.env, &t1.id, &mv, 7).unwrap_err().contains("old repo"));
    let mv: TaskPatch = serde_json::from_value(json!({"repoId": r2.id, "plan": {"kind": "text", "text": "new"}})).unwrap();
    let t = update_task(&mut c, &f.env, &t1.id, &mv, 8).unwrap();
    assert_eq!((t.repo_id.as_str(), &t.plan), (r2.id.as_str(), &PlanRef::Text));
    assert_eq!(read_task_plan(&c, &f.env, &t1.id).unwrap(), "new");

    // Options with null = back to the repo default.
    let opts: TaskPatch = serde_json::from_value(json!({"isolation": "in_place", "review": false, "assignee": {"kind": "workflow", "name": "plan-task"}})).unwrap();
    let t = update_task(&mut c, &f.env, &t1.id, &opts, 9).unwrap();
    assert_eq!((t.isolation, t.review), (Some(Isolation::InPlace), Some(false)));
    let clear: TaskPatch = serde_json::from_value(json!({"isolation": null, "assignee": null})).unwrap();
    let t = update_task(&mut c, &f.env, &t1.id, &clear, 10).unwrap();
    assert_eq!((t.isolation, t.assignee, t.review), (None, None, Some(false)));

    // Manual status and closing.
    let t = move_task(&mut c, &t1.id, TaskStatus::Done, 0.5, 11).unwrap();
    assert_eq!((t.status, t.position, t.closed_at), (TaskStatus::Done, 0.5, Some(11)));
    let t = move_task(&mut c, &t1.id, TaskStatus::Todo, 2.0, 12).unwrap();
    assert_eq!(t.closed_at, None);
    assert!(move_task(&mut c, &t1.id, TaskStatus::Todo, f64::NAN, 13).is_err());

    // Reordering the column: renumbers and puts the unlisted ones after.
    let t3 = create_task(&mut c, &f.env, &new_task(&p, &r, "Third"), 14).unwrap();
    let pos = |c: &Connection, id: &str| tasks::get(c, id).unwrap().position;
    let got = reorder_tasks(&mut c, TaskStatus::Todo, &[t3.id.clone(), t1.id.clone()], 15).unwrap();
    assert_eq!(got.as_deref(), Some(p.id.as_str()));
    assert_eq!((pos(&c, &t3.id), pos(&c, &t1.id), pos(&c, &t2.id)), (1.0, 2.0, 3.0));
    // Repeating the same order touches no row.
    reorder_tasks(&mut c, TaskStatus::Todo, &[t3.id.clone(), t1.id.clone()], 99).unwrap();
    assert_eq!(tasks::get(&c, &t2.id).unwrap().updated_at, 15);
    assert!(reorder_tasks(&mut c, TaskStatus::Todo, &[t1.id.clone(), t1.id.clone()], 16).unwrap_err().contains("twice"));
    assert!(reorder_tasks(&mut c, TaskStatus::Done, std::slice::from_ref(&t1.id), 16).unwrap_err().contains("column"));
    assert!(reorder_tasks(&mut c, TaskStatus::Todo, &["t-no".into()], 16).is_err());
    // An error leaves nothing half-done.
    assert!(reorder_tasks(&mut c, TaskStatus::Todo, &[t2.id.clone(), "t-no".into()], 16).is_err());
    assert_eq!(pos(&c, &t2.id), 3.0);
    assert_eq!(reorder_tasks(&mut c, TaskStatus::Todo, &[], 16).unwrap(), None);
    delete_task(&c, &f.env, &t3.id).unwrap();

    // Relations.
    add_relation(&c, &t1.id, &t2.id, RelationKind::Related).unwrap();
    add_relation(&c, &t2.id, &t1.id, RelationKind::Related).unwrap();
    add_relation(&c, &t1.id, &t2.id, RelationKind::Blocks).unwrap();
    assert_eq!(relations::list(&c, &t2.id).unwrap().len(), 2);
    assert!(add_relation(&c, &t1.id, &t1.id, RelationKind::Blocks).is_err());
    assert!(add_relation(&c, &t1.id, "t-no", RelationKind::Blocks).unwrap_err().contains("no longer exists"));
    remove_relation(&c, &t2.id, &t1.id, RelationKind::Related).unwrap();
    assert_eq!(relations::list(&c, &t1.id).unwrap().len(), 1);

    // Deleting the project cascades.
    let gone = delete_project(&mut c, &p.id).unwrap();
    assert_eq!(gone.len(), 2);
    assert!(tasks::list(&c, None).unwrap().is_empty());
}

#[test]
fn imported_tasks_are_read_only_and_manual_moves_push_state() {
    let f = fx("ops-imported");
    let db = open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    let p = create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None, description: None, root_path: None }, 1).unwrap();
    let r = add_repo(&c, &p.id, &NewRepo::default(), &f.repo_dir, 2).unwrap();
    let t = create_task(&mut c, &f.env, &new_task(&p, &r, "Imported"), 3).unwrap();
    let mut map = StateMap { confirmed_at: Some(1), ..Default::default() };
    map.push.insert(TaskStatus::InProgress, Some("s-prog".into()));
    let link = SourceLink {
        id: "l1".into(),
        project_id: p.id.clone(),
        provider: "linear".into(),
        scope: ScopeRef { kind: "team".into(), id: "tm".into(), name: "Eng".into() },
        default_repo_id: None,
        repo_rules: vec![],
        state_map: map,
        auto_import: false,
        created_at: 1,
        last_synced_at: None,
        last_sync_error: None,
        pending_state_changes: None,
    };
    rows::insert_source_link(&c, &link).unwrap();
    c.execute(
        "UPDATE tasks SET src_provider='linear', src_link_id='l1', src_external_id='e1', src_identifier='ENG-1', src_url='https://linear.app/acme/issue/ENG-1' WHERE id=?1",
        [&t.id],
    )
    .unwrap();
    let err = update_task(&mut c, &f.env, &t.id, &TaskPatch { title: Some("x".into()), ..Default::default() }, 4).unwrap_err();
    assert!(err.contains("read-only"));
    let plan: TaskPatch = serde_json::from_value(json!({"plan": {"kind": "text", "text": "mine"}})).unwrap();
    assert!(update_task(&mut c, &f.env, &t.id, &plan, 5).unwrap().plan_overridden);
    move_task(&mut c, &t.id, TaskStatus::InProgress, 1.0, 6).unwrap();
    let n: i64 = c.query_row("SELECT COUNT(*) FROM sync_outbox WHERE task_id = ?1 AND kind = 'set_state'", [&t.id], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
    // Moving within the same column pushes nothing.
    move_task(&mut c, &t.id, TaskStatus::InProgress, 3.0, 7).unwrap();
    let n: i64 = c.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
    // The project can be deleted even with linked tasks.
    delete_project(&mut c, &p.id).unwrap();
}

#[test]
fn settings_validation() {
    let db = open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    let s = set_settings(&mut c, &Settings { concurrency: 2, editor: Some("cursor".into()), reviewer: "code-reviewer".into(), default_executor: None }).unwrap();
    assert_eq!((s.concurrency, s.editor.as_deref()), (2, Some("cursor")));
    assert!(set_settings(&mut c, &Settings { concurrency: 0, ..s.clone() }).is_err());
    assert!(set_settings(&mut c, &Settings { editor: Some("vim; rm".into()), ..s.clone() }).is_err());
    let s = set_settings(&mut c, &Settings { editor: Some(" ".into()), ..s }).unwrap();
    assert_eq!(s.editor, None);
}
