use super::*;
use crate::util::paths::tests::TempDir;

#[test]
fn frontmatter_real_shapes() {
    // Like `~/.claude/agents/frontend-developer.md`: double-quoted description with \n.
    let fe = "---\nname: frontend-developer\ndescription: \"Use when building apps. Specifically:\\n\\n<example>\\nContext: x\\n</example>\"\ntools: Read, Write, Edit, Bash, Glob, Grep\n---\n\nYou are a senior frontend developer.\n";
    let f = parse_frontmatter(fe).unwrap();
    assert_eq!(f.name.as_deref(), Some("frontend-developer"));
    assert_eq!(f.tools.as_deref().unwrap(), ["Read", "Write", "Edit", "Bash", "Glob", "Grep"]);
    assert_eq!(short_description(&f.description.unwrap()), "Use when building apps. Specifically:");
    // Like `code-reviewer.md`: a literal `\\n` inside double quotes.
    let cr = "---\nname: code-reviewer\ndescription: \"Reviews code.\\\\n\\\\n<example>\\\\nx\"\ntools: Read, Grep\n---\n";
    let f = parse_frontmatter(cr).unwrap();
    assert_eq!(short_description(&f.description.unwrap()), "Reviews code.");
    // YAML list, folded block, single quotes.
    let other = "---\nname: 'db-helper'\ndescription: >\n  Helps with\n  databases\ntools:\n  - Read\n  - Bash\nmodel: sonnet\n---\n";
    let f = parse_frontmatter(other).unwrap();
    assert_eq!(f.name.as_deref(), Some("db-helper"));
    assert_eq!(f.description.as_deref(), Some("Helps with databases"));
    assert_eq!(f.tools.unwrap(), ["Read", "Bash"]);
    assert_eq!(parse_frontmatter("no frontmatter"), None);
    let f = parse_frontmatter("---\nname: x\ntools: [Read, \"Write\"]\n---").unwrap();
    assert_eq!(f.tools.unwrap(), ["Read", "Write"]);
    assert_eq!(parse_frontmatter("---\nname: y\n---\n").unwrap().tools, None);
}

#[test]
fn agent_names() {
    assert!(is_valid_agent_name("code-reviewer"));
    assert!(is_valid_agent_name("pr-review-toolkit:code-reviewer"));
    assert!(!is_valid_agent_name("-x"));
    assert!(!is_valid_agent_name("a b"));
    assert!(!is_valid_agent_name(""));
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

#[test]
fn catalog_merges_user_repo_plugins_and_workflows() {
    let t = TempDir::new("executors");
    let claude = t.0.join("claude");
    let repo = t.0.join("repo");
    write(&claude.join("agents/code-reviewer.md"), "---\nname: code-reviewer\ndescription: user version\ntools: Read\n---\n");
    write(&claude.join("agents/frontend-developer.md"), "---\nname: frontend-developer\ndescription: fe\n---\n");
    write(&claude.join("agents/.hidden.md"), "---\nname: hidden\n---\n");
    write(&claude.join("agents/notes.txt"), "x");
    write(&repo.join(".claude/agents/code-reviewer.md"), "---\nname: code-reviewer\ndescription: repo version\n---\n");
    // Enabled plugin with agents, another installed but disabled, one from another project.
    let inst = t.0.join("cache/toolkit/1.0");
    write(&inst.join("agents/security.md"), "---\nname: security\ndescription: sec\n---\n");
    let off = t.0.join("cache/off/1.0");
    write(&off.join("agents/nope.md"), "---\nname: nope\n---\n");
    let other = t.0.join("cache/proj/1.0");
    write(&other.join("agents/elsewhere.md"), "---\nname: elsewhere\n---\n");
    write(
        &claude.join("settings.json"),
        r#"{"enabledPlugins": {"toolkit@market": true, "off@market": false, "proj@market": true}}"#,
    );
    let installed = serde_json::json!({
        "version": 2,
        "plugins": {
            "toolkit@market": [{"scope": "user", "installPath": inst}],
            "off@market": [{"scope": "user", "installPath": off}],
            "proj@market": [{"scope": "project", "projectPath": "/somewhere/else", "installPath": other}]
        }
    });
    write(&claude.join("plugins/installed_plugins.json"), &installed.to_string());
    write(&claude.join("workflows/plan-task.js"), "export const meta = { name: 'plan-task', reviews: true }\n");
    write(&claude.join("workflows/linear-issue.js"), "export const meta = { name: 'linear-issue', managesSource: 'linear', reviews: true }\n");

    let list = catalog(Some(&claude), Some(&repo));
    let names: Vec<String> = list
        .iter()
        .map(|e| match &e.executor {
            Executor::Claude => "claude".to_string(),
            Executor::Agent { name, source } => format!("agent:{name}:{}", source.as_str()),
            Executor::Workflow { name } => format!("wf:{name}"),
        })
        .collect();
    assert_eq!(
        names,
        [
            "claude",
            "agent:code-reviewer:repo",
            "agent:frontend-developer:user",
            "agent:toolkit:security:plugin",
            "wf:linear-issue",
            "wf:plan-task"
        ]
    );
    assert_eq!(list[1].description.as_deref(), Some("repo version"));
    let li = &list[4];
    assert_eq!(li.manages_source.as_deref(), Some("linear"));
    assert!(li.reviews);
    assert_eq!(li.source, Some(AgentSource::User));
    assert!(find_workflow(Some(&claude), Some(&repo), "plan-task").unwrap().reviews);
    assert!(find_workflow(Some(&claude), None, "nope").is_none());
    // No repo: only the user's agents and user-scoped plugins.
    let no_repo = catalog(Some(&claude), None);
    assert!(no_repo.iter().any(|e| e.description.as_deref() == Some("user version")));
    assert!(!no_repo.iter().any(|e| matches!(&e.executor, Executor::Agent { name, .. } if name.contains("elsewhere"))));
}

/// Against this machine's `~/.claude`: `cargo test -- --ignored`.
#[test]
#[ignore]
fn real_catalog() {
    let dir = crate::runs::claude_fs::claude_config_dir().unwrap();
    for e in catalog(Some(&dir), None) {
        eprintln!("{:?} · {:?} · {:?}", e.executor, e.tools, e.description.map(|d| clip_chars(&d, 60)));
    }
}
