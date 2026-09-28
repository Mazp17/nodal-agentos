use super::*;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/runs/fixtures/workflows")
}

#[test]
fn parses_real_linear_issue_meta() {
    let src = std::fs::read_to_string(fixtures().join("user/linear-issue.js")).unwrap();
    let m = parse_meta(&src).unwrap();
    assert_eq!(m.name.as_deref(), Some("linear-issue"));
    assert!(m.description.unwrap().starts_with("Lleva una issue de Linear"));
    assert!(m.when_to_use.unwrap().contains("Nunca mergea."));
}

#[test]
fn parses_real_demo_board_meta() {
    let src = std::fs::read_to_string(fixtures().join("user/demo-board.js")).unwrap();
    let m = parse_meta(&src).unwrap();
    assert_eq!(m.name.as_deref(), Some("demo-board"));
    assert!(m.description.unwrap().starts_with("Workflow de juguete"));
    // It's on the line after `whenToUse:` and has double quotes inside.
    assert!(m.when_to_use.unwrap().contains(r#"args: "volcanes""#));
}

#[test]
fn nested_name_and_comments_do_not_confuse() {
    let src = "// meta = { name: 'fake' }\nexport const meta = {\n  /* name: 'other' */\n  phases: [{ name: 'phase' }],\n  \"name\": \"real\",\n  description: 'with \\'escape\\' and naïveté',\n}\n";
    let m = parse_meta(src).unwrap();
    assert_eq!(m.name.as_deref(), Some("real"));
    assert_eq!(m.description.as_deref(), Some("with 'escape' and naïveté"));
    assert_eq!(m.when_to_use, None);
}

#[test]
fn manages_source_and_reviews() {
    let src = "export const meta = {\n  name: 'x',\n  managesSource: 'linear',\n  reviews: true,\n  phases: [{ reviews: false }],\n}\n";
    let m = parse_meta(src).unwrap();
    assert_eq!(m.manages_source.as_deref(), Some("linear"));
    assert_eq!(m.reviews, Some(true));
    let m = parse_meta("export const meta = { name: 'y', 'reviews': false, noreviews: true }").unwrap();
    assert_eq!(m.reviews, Some(false));
    assert_eq!(m.manages_source, None);
    // Only the nested one: doesn't count.
    let m = parse_meta("export const meta = { name: 'z', opts: { reviews: true } }").unwrap();
    assert_eq!(m.reviews, None);
    // The real fixtures don't declare it yet.
    let src = std::fs::read_to_string(fixtures().join("user/linear-issue.js")).unwrap();
    assert_eq!(parse_meta(&src).unwrap().reviews, None);
}

#[test]
fn broken_or_missing_meta() {
    assert_eq!(parse_meta("const x = 1"), None);
    assert_eq!(parse_meta("export const meta = { name: 'unclosed'"), None);
    assert_eq!(parse_meta("export const meta = loadMeta()"), None);
    let m = parse_meta("export const meta = { name: someVariable }").unwrap();
    assert_eq!(m.name, None);
}

#[test]
fn catalog_ignores_backups_and_repo_overrides_user() {
    let root = fixtures();
    let list = list_from(Some(&root.join("user")), Some(&root.join("repo")));
    let names: Vec<_> = list.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["demo-board", "linear-issue", "sin-meta"]);
    let li = &list[1];
    assert_eq!(li.source, WorkflowSource::Repo);
    assert_eq!(li.description.as_deref(), Some("Variante del repo con \"comillas\" escapadas"));
    assert_eq!(list[2].description, None);

    let only_user = list_from(Some(&root.join("user")), None);
    assert_eq!(only_user.len(), 2);
    assert!(only_user.iter().all(|w| w.source == WorkflowSource::User));
    assert!(list_from(Some(Path::new("/no/such/dir")), None).is_empty());
}

/// Against this machine's `~/.claude/workflows`: `cargo test -- --ignored`.
#[test]
#[ignore]
fn real_user_catalog() {
    let dir = crate::runs::claude_fs::claude_config_dir().unwrap().join("workflows");
    let list = list_from(Some(&dir), None);
    for w in &list {
        eprintln!("{} · {:?}", w.name, w.description);
    }
    assert!(list.iter().any(|w| w.name == "linear-issue" && w.description.is_some()));
    assert!(list.iter().all(|w| !w.path.contains(".bak")));
}

#[test]
fn workflow_names() {
    assert!(is_valid_workflow_name("linear-issue"));
    assert!(is_valid_workflow_name("plugin:flow_2"));
    assert!(!is_valid_workflow_name("-x"));
    assert!(!is_valid_workflow_name("a b"));
    assert!(!is_valid_workflow_name(""));
}
