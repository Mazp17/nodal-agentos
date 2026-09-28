use super::*;
use crate::db::open_in_memory;
use crate::db::rows::{insert_project, insert_repo, insert_source_link};
use crate::domain::*;
use crate::providers::plan::tests::item;
use crate::providers::state_map::{propose, tests::team_states};
use crate::providers::OPEN_KINDS;

pub fn seed(conn: &Connection) -> SourceLink {
    insert_project(
        conn,
        &Project {
            id: "p1".into(),
            name: "Acme Web".into(),
            key: "WEB".into(),
            next_task_number: 1,
            color: "#fff".into(),
            default_executor: None,
            reviewer: None,
            created_at: 1,
            archived_at: None,
            root_path: None,
            description: None,
        },
    )
    .unwrap();
    insert_project(
        conn,
        &Project {
            id: "p2".into(),
            name: "Acme Ops".into(),
            key: "OPS".into(),
            next_task_number: 1,
            color: "#fff".into(),
            default_executor: None,
            reviewer: None,
            created_at: 1,
            archived_at: None,
            root_path: None,
            description: None,
        },
    )
    .unwrap();
    for (id, project) in [("r-web", "p1"), ("r-docs", "p1"), ("r-ops", "p2")] {
        insert_repo(
            conn,
            &Repo {
                id: id.into(),
                project_id: project.into(),
                path: format!("/tmp/acme-{id}"),
                name: id.into(),
                launch: LaunchOptions::default(),
                default_executor: None,
                default_isolation: Isolation::Worktree,
                default_finish: Finish::Pr,
                default_review: true,
                reviewer: None,
                position: 0,
                created_at: 1,
            },
        )
        .unwrap();
    }
    let mut map = propose(&team_states());
    map.confirmed_at = Some(1);
    let link = SourceLink {
        id: "l1".into(),
        project_id: "p1".into(),
        provider: "fake".into(),
        scope: ScopeRef {
            kind: "team".into(),
            id: "team-eng".into(),
            name: "Engineering".into(),
        },
        default_repo_id: None,
        repo_rules: vec![RepoRule::label("Docs", "r-docs")],
        state_map: map,
        auto_import: false,
        created_at: 1,
        last_synced_at: None,
        last_sync_error: None,
        pending_state_changes: None,
    };
    insert_source_link(conn, &link).unwrap();
    link
}

pub fn tmp_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "nodal-import-{}",
        new_id('x', crate::util::now_ms())
    ))
}

#[test]
fn suggest_repo_by_rule_then_default() {
    let db = open_in_memory().unwrap();
    let conn = db.lock().unwrap();
    let mut link = seed(&conn);
    assert_eq!(
        suggest_repo(&link, None, &["docs".into()]).as_deref(),
        Some("r-docs")
    );
    assert_eq!(suggest_repo(&link, None, &["frontend".into()]), None);
    link.default_repo_id = Some("r-web".into());
    assert_eq!(
        suggest_repo(&link, None, &["frontend".into()]).as_deref(),
        Some("r-web")
    );
}

/// An item of the provider's project `proj` (id `proj-{proj}`).
pub fn in_project(mut it: ExternalItem, proj: &str) -> ExternalItem {
    it.scopes.retain(|s| s.kind != "project");
    it.scopes.push(ScopeRef {
        kind: "project".into(),
        id: format!("proj-{proj}"),
        name: proj.into(),
    });
    it
}

#[test]
fn project_rule_beats_label_rule_beats_default() {
    let db = open_in_memory().unwrap();
    let conn = db.lock().unwrap();
    let mut link = seed(&conn);
    link.default_repo_id = Some("r-web".into());
    link.repo_rules
        .push(RepoRule::project("proj-site", "Site", "r-web", 1));
    link.repo_rules
        .push(RepoRule::project("proj-guides", "Guides", "r-docs", 1));
    let labels = vec!["docs".to_string()];
    // A project with a rule beats the label.
    assert_eq!(
        suggest_repo(&link, Some("proj-site"), &labels).as_deref(),
        Some("r-web")
    );
    assert_eq!(
        suggest_repo(&link, Some("proj-guides"), &[]).as_deref(),
        Some("r-docs")
    );
    // Project without a rule: label, otherwise default.
    assert_eq!(
        suggest_repo(&link, Some("proj-other"), &labels).as_deref(),
        Some("r-docs")
    );
    assert_eq!(
        suggest_repo(&link, Some("proj-other"), &[]).as_deref(),
        Some("r-web")
    );
    // A label rule whose value matches a project id is not a project rule.
    link.repo_rules.push(RepoRule::label("proj-x", "r-docs"));
    assert_eq!(
        suggest_repo(&link, Some("proj-x"), &[]).as_deref(),
        Some("r-web")
    );
    assert_eq!(
        suggest_repo_for(&link, &in_project(item(1, None), "site")).as_deref(),
        Some("r-web")
    );
}

#[test]
fn legacy_rules_json_still_reads() {
    let db = open_in_memory().unwrap();
    let conn = db.lock().unwrap();
    let link = seed(&conn);
    conn.execute(
        r#"UPDATE source_links SET repo_rules_json = '[{"label":"Docs","repoId":"r-docs"},{"label":"api","repoId":"r-web"}]' WHERE id = ?1"#,
        [&link.id],
    )
    .unwrap();
    let a = store::get_link(&conn, &link.id).unwrap();
    let b = store::get_link(&conn, &link.id).unwrap();
    assert_eq!(a.repo_rules, b.repo_rules, "stable ids across reads");
    let r = &a.repo_rules[1];
    assert_eq!(
        (
            r.kind,
            r.value.as_str(),
            r.name.as_str(),
            r.repo_id.as_str()
        ),
        (RuleKind::Label, "api", "api", "r-web")
    );
    assert_eq!((r.id.as_str(), r.created_at), ("rule-1", link.created_at));
    assert_eq!(
        suggest_repo(&a, None, &["DOCS".into()]).as_deref(),
        Some("r-docs")
    );
    // Saved and read back: new format, same data.
    store::save_link(&conn, &a).unwrap();
    let json: String = conn
        .query_row(
            "SELECT repo_rules_json FROM source_links WHERE id = ?1",
            [&link.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        json.contains(r#""kind":"label""#) && json.contains(r#""value":"api""#),
        "{json}"
    );
    assert_eq!(
        store::get_link(&conn, &link.id).unwrap().repo_rules,
        a.repo_rules
    );

    // An unknown kind (future version) doesn't break reading or routing.
    conn.execute(
        r#"UPDATE source_links SET repo_rules_json = '[{"kind":"milestone","value":"m1","repoId":"r-docs"}]' WHERE id = ?1"#,
        [&link.id],
    )
    .unwrap();
    let l = store::get_link(&conn, &link.id).unwrap();
    assert_eq!(l.repo_rules[0].kind, RuleKind::Unknown);
    assert_eq!(suggest_repo(&l, Some("m1"), &["m1".into()]), None);
}


#[test]
fn backfill_plan_splits_new_here_elsewhere_and_unlinked() {
    let db = open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    let mut link = seed(&conn);
    let rule = RepoRule::project("proj-guides", "Guides", "r-docs", 5);
    link.repo_rules.push(rule.clone());
    let dir = tmp_dir();
    let items: Vec<_> = (1..=5)
        .map(|n| in_project(item(n, None), "guides"))
        .collect();
    // 1 already in the rule's repo, 2 in another repo, 3 unlinked, 4 and 5 new.
    let r = import_items(
        &mut conn,
        &dir,
        &link,
        vec![
            (items[0].clone(), "r-docs".into()),
            (items[1].clone(), "r-web".into()),
            (items[2].clone(), "r-web".into()),
        ],
        2,
    )
    .unwrap();
    store::unlink_task(&conn, &r.imported[2].id, 3).unwrap();
    let mut dup = items.clone();
    dup.push(items[4].clone());
    let plan = plan_backfill(&conn, &link, &rule, &dup).unwrap();
    assert_eq!(plan.to_import, vec!["uuid-4", "uuid-5"]);
    assert_eq!(plan.here, vec![r.imported[0].id.clone()]);
    assert_eq!(plan.unlinked, 1);
    assert_eq!(plan.elsewhere.len(), 1);
    assert!(
        plan.elsewhere[0]
            .reason
            .contains("ENG-2 is already imported in r-web"),
        "{:?}",
        plan.elsewhere
    );
    assert_eq!(
        plan.preview(),
        RulePreview {
            count: 2,
            already_imported: 1,
            in_other_repos: 1
        }
    );
    // Imported into its rule's repo: stays tied to the rule and has the project.
    let t = &r.imported[0];
    let src = t.source.as_ref().unwrap();
    assert_eq!(src.rule_id.as_deref(), Some("rule-proj-guides"));
    assert_eq!(
        src.project.as_ref().map(|p| p.id.as_str()),
        Some("proj-guides")
    );
    assert_eq!(
        r.imported[1].source.as_ref().unwrap().rule_id,
        None,
        "another repo: didn't arrive through the rule"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn rule_queries_backfill_vs_auto_import() {
    let db = open_in_memory().unwrap();
    let conn = db.lock().unwrap();
    let link = seed(&conn);
    let rule = RepoRule::project("proj-guides", "Guides", "r-docs", 77);
    let b = rule_query(&link, &rule, true);
    assert_eq!(
        (
            b.project_id.as_deref(),
            b.closed_within_days,
            b.created_after
        ),
        (Some("proj-guides"), Some(14), None)
    );
    assert_eq!(b.state_kinds, OPEN_KINDS.to_vec());
    let a = rule_query(&link, &rule, false);
    assert_eq!((a.closed_within_days, a.created_after), (None, Some(77)));
    assert_eq!(a.scope, link.scope);
}

#[test]
fn imports_with_plan_criteria_and_status() {
    let db = open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    let link = seed(&conn);
    let dir = tmp_dir();
    let mut it = item(
        142,
        Some("Text.\n\n## Acceptance\n- New logo\n- Green tests\n"),
    );
    it.labels = vec!["frontend".into()];
    it.priority = Priority::High;
    it.state = team_states()
        .into_iter()
        .find(|s| s.id == "s-review")
        .unwrap();

    let r = import_items(
        &mut conn,
        &dir,
        &link,
        vec![(it.clone(), "r-web".into())],
        10,
    )
    .unwrap();
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    let t = &r.imported[0];
    assert_eq!(t.number, 1);
    assert_eq!(t.status, TaskStatus::InReview);
    assert_eq!(t.priority, Priority::High);
    assert_eq!(t.labels, vec!["frontend"]);
    assert_eq!(t.acceptance, vec!["New logo", "Green tests"]);
    let src = t.source.as_ref().unwrap();
    assert_eq!(src.identifier, "ENG-142");
    assert_eq!(src.link_id.as_deref(), Some("l1"));
    assert_eq!(src.external_state.as_ref().unwrap().id, "s-review");
    let plan = std::fs::read_to_string(plan_path(&dir, &t.id)).unwrap();
    assert!(plan.starts_with("# ENG-142 · Issue 142\n"));
    assert_eq!(
        crate::db::rows::get_task(&conn, &t.id).unwrap().as_ref(),
        Some(t)
    );

    // Second time: already imported. Repo from another project: rejected.
    let mut other = item(7, None);
    other.external_id = "uuid-7".into();
    let r = import_items(
        &mut conn,
        &dir,
        &link,
        vec![(it, "r-web".into()), (other, "r-ops".into())],
        11,
    )
    .unwrap();
    assert!(r.imported.is_empty());
    assert!(r.skipped[0].reason.contains("already imported"));
    assert!(r.skipped[1].reason.contains("another project"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn listing_marks_imported_and_suggests_repo() {
    let db = open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    let link = seed(&conn);
    let dir = tmp_dir();
    let a = item(1, None);
    let mut b = item(2, None);
    b.labels = vec!["DOCS".into()];
    let r = import_items(&mut conn, &dir, &link, vec![(a.clone(), "r-web".into())], 1).unwrap();
    let rows = importable_rows(&conn, &link, vec![a, b]).unwrap();
    assert_eq!(rows[0].task_id.as_deref(), Some(r.imported[0].id.as_str()));
    assert_eq!(rows[1].task_id, None);
    assert_eq!(rows[1].suggested_repo_id.as_deref(), Some("r-docs"));
    std::fs::remove_dir_all(dir).ok();
}
