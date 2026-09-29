use super::*;

#[test]
fn patch_distinguishes_missing_from_null() {
    let p: SourceLinkPatch = serde_json::from_str(r#"{"autoImport": true}"#).unwrap();
    assert_eq!(p.default_repo_id, None);
    assert_eq!(p.auto_import, Some(true));
    let p: SourceLinkPatch = serde_json::from_str(r#"{"defaultRepoId": null}"#).unwrap();
    assert_eq!(p.default_repo_id, Some(None));
    let p: SourceLinkPatch = serde_json::from_str(r#"{"defaultRepoId": "r1"}"#).unwrap();
    assert_eq!(p.default_repo_id, Some(Some("r1".into())));
}

#[test]
fn prepare_rules_assigns_ids_and_keeps_created_at() {
    let old = vec![RepoRule::project("proj-a", "A", "r1", 5), RepoRule::label("docs", "r2")];
    // Unchanged: keeps id and created_at even if the client sends another.
    let mut same = old.clone();
    same[0].created_at = 999;
    let out = prepare_rules(&old, same, 50).unwrap();
    assert_eq!((out[0].id.as_str(), out[0].created_at), ("rule-proj-a", 5));
    // New rule (no id) and rule with the same id but another project: new id and created_at.
    let incoming: Vec<RepoRule> = serde_json::from_value(serde_json::json!([
        {"id": "rule-proj-a", "kind": "project", "value": "proj-b", "name": "B", "repoId": "r1"},
        {"kind": "project", "value": " proj-c ", "repoId": "r2"},
        {"label": "legacy", "repoId": "r2"}
    ]))
    .unwrap();
    let out = prepare_rules(&old, incoming, 50).unwrap();
    assert!(
        out.iter().all(|r| r.id.starts_with('r') && r.id != "rule-proj-a" && r.created_at == 50),
        "{out:?}"
    );
    assert_eq!((out[1].value.as_str(), out[1].name.as_str()), ("proj-c", "proj-c"));
    assert_eq!((out[2].kind, out[2].value.as_str()), (RuleKind::Label, "legacy"));
    // Validations.
    let dup = vec![RepoRule::project("p", "P", "r1", 1), RepoRule::project("p", "P", "r2", 1)];
    assert!(prepare_rules(&[], dup, 1).unwrap_err().contains("already has a rule"));
    let empty = vec![RepoRule::label(" ", "r1")];
    assert!(prepare_rules(&[], empty, 1).unwrap_err().contains("need a label"));
}
