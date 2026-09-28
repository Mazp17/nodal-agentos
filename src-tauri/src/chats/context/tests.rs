use super::*;
use crate::domain::LaunchOptions;
use crate::work::testutil::{project_of, repo_of};

fn chat(repo_id: Option<&str>) -> Chat {
    Chat {
        id: "c1".into(),
        project_id: "p1".into(),
        repo_id: repo_id.map(Into::into),
        title: None,
        session_title: None,
        session_id: None,
        launch: LaunchOptions::default(),
        created_at: 1,
        updated_at: 1,
    }
}

fn fixture() -> (Project, Vec<Repo>) {
    let mut project = project_of("p1", "PAY");
    project.name = "Acme\n payments".into();
    project.description = Some("  Checkout and invoices.  ".into());
    let mut web = repo_of("r2", "p1", "/Users/me/Code/acme-web");
    web.name = "acme-web".into();
    let mut api = repo_of("r1", "p1", "/Users/me/Code/acme-api");
    api.name = "acme-api".into();
    (project, vec![api, web])
}

fn value_of<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
    let Some(i) = args.iter().position(|a| a == flag) else {
        return vec![];
    };
    args[i + 1..]
        .iter()
        .take_while(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect()
}

#[test]
fn a_project_chat_gets_nodal_mcp_every_other_repo_and_the_project() {
    let (project, repos) = fixture();
    let bin = Path::new("/Applications/Nodal.app/Contents/MacOS/nodal-mcp");
    let args = args(
        &chat(None),
        &project,
        &repos,
        ChatCwd::Repo(&repos[0]),
        Some(bin),
    );

    let config: serde_json::Value =
        serde_json::from_str(value_of(&args, "--mcp-config")[0]).unwrap();
    assert_eq!(
        config,
        json!({"mcpServers": {"nodal": {"command": "/Applications/Nodal.app/Contents/MacOS/nodal-mcp", "args": ["--chat"]}}})
    );
    assert_eq!(
        value_of(&args, "--allowedTools"),
        ["mcp__nodal__list_projects,mcp__nodal__list_tasks,mcp__nodal__get_task,mcp__nodal__get_run,mcp__nodal__propose_task"]
    );
    assert_eq!(
        value_of(&args, "--add-dir"),
        ["/Users/me/Code/acme-web"],
        "the cwd isn't added again"
    );

    let prompt = value_of(&args, "--append-system-prompt")[0];
    assert!(
        prompt.starts_with("You are"),
        "a value starting with - would read as a flag"
    );
    for part in [
        "Project: Acme payments (key PAY, id p1)",
        "Description: Checkout and invoices.\n",
        "- acme-api: /Users/me/Code/acme-api (id r1) (this chat runs here)",
        "- acme-web: /Users/me/Code/acme-web (id r2)\n",
        "Scope: the whole project",
        "pass project \"PAY\"",
        "propose_task: when",
    ] {
        assert!(prompt.contains(part), "missing {part:?} in:\n{prompt}");
    }
    assert_eq!(args.last().map(String::as_str), Some(prompt));
}

#[test]
fn a_repo_chat_is_narrowed_and_works_without_nodal_mcp() {
    let (project, repos) = fixture();
    let args = args(
        &chat(Some("r2")),
        &project,
        &repos,
        ChatCwd::Repo(&repos[1]),
        None,
    );
    assert_eq!(args.len(), 2, "{args:?}");
    assert_eq!(args[0], "--append-system-prompt");
    let prompt = &args[1];
    assert!(
        prompt.contains("Scope: only the repo acme-web (/Users/me/Code/acme-web)"),
        "{prompt}"
    );
    assert!(prompt.contains("- acme-web: /Users/me/Code/acme-web (id r2) (this chat runs here)"));
    assert!(
        prompt.contains("tools are not available") && !prompt.contains(PROPOSE_TASK),
        "{prompt}"
    );
}

#[test]
fn a_long_description_is_clipped() {
    let (mut project, repos) = fixture();
    project.description = Some("x".repeat(DESCRIPTION_MAX + 50));
    let prompt = system_prompt(
        &chat(None),
        &project,
        &repos,
        ChatCwd::Repo(&repos[0]),
        true,
    );
    let line = prompt
        .lines()
        .find(|l| l.starts_with("Description: "))
        .unwrap();
    assert!(
        line.chars().count() <= "Description: ".len() + DESCRIPTION_MAX,
        "{}",
        line.len()
    );
}

#[test]
fn a_root_chat_runs_at_the_root_and_adds_only_repos_outside_it() {
    let (mut project, mut repos) = fixture();
    project.root_path = Some("/Users/me/Code/acme".into());
    repos[0].path = "/Users/me/Code/acme/api".into();
    repos[1].path = "/Users/me/Code/acme-web".into();
    let args = args(
        &chat(None),
        &project,
        &repos,
        ChatCwd::Root("/Users/me/Code/acme"),
        None,
    );
    assert_eq!(
        value_of(&args, "--add-dir"),
        ["/Users/me/Code/acme-web"],
        "a sibling with a shared prefix is outside"
    );

    let prompt = value_of(&args, "--append-system-prompt")[0];
    for part in [
        "Project root folder: /Users/me/Code/acme (this chat runs here)\n",
        "- acme-api: /Users/me/Code/acme/api (id r1)\n",
        "- acme-web: /Users/me/Code/acme-web (id r2)\n",
        "This chat runs at the project root.",
        "tasks and runs always target one of the repos above",
    ] {
        assert!(prompt.contains(part), "missing {part:?} in:\n{prompt}");
    }
    assert_eq!(
        prompt.matches("(this chat runs here)").count(),
        1,
        "{prompt}"
    );
}

#[test]
fn a_root_chat_with_every_repo_under_it_adds_none() {
    let (mut project, mut repos) = fixture();
    project.root_path = Some("/Users/me/Code/acme".into());
    repos[0].path = "/Users/me/Code/acme/api".into();
    repos[1].path = "/Users/me/Code/acme/web/app".into();
    let root = ChatCwd::Root("/Users/me/Code/acme");
    assert!(value_of(
        &args(&chat(None), &project, &repos, root, None),
        "--add-dir"
    )
    .is_empty());

    let prompt = system_prompt(&chat(None), &project, &[], root, false);
    assert!(prompt.contains("Repos:\n- none yet\n"), "{prompt}");
}

/// Needs `cargo build --bin nodal-mcp` first: the test binary has no `nodal-mcp` beside it.
#[test]
#[ignore]
fn real_chat_sees_propose_task() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let bin = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/nodal-mcp");
    assert!(bin.is_file(), "build nodal-mcp first");
    let dir = std::env::temp_dir().join(format!("nodal-chat-context-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut project = project_of("p1", "PAY");
    project.name = "Acme".into();
    let repo = repo_of("r1", "p1", &dir.to_string_lossy());
    let args = args(
        &chat(None),
        &project,
        std::slice::from_ref(&repo),
        ChatCwd::Repo(&repo),
        Some(&bin),
    );
    let tools = tauri::async_runtime::block_on(async {
        let mut cmd = crate::runs::claude_bin::claude_command().expect("claude");
        cmd.args(crate::runs::stream_json::CHAT_ARGS)
            .args(["--model", "haiku"])
            .args(&args)
            .current_dir(&dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let msg = json!({"type": "user", "message": {"role": "user", "content": "Reply OK"}});
        stdin
            .write_all(format!("{msg}\n").as_bytes())
            .await
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            let ev: serde_json::Value = serde_json::from_str(&line).unwrap();
            if ev["type"] == "system" && ev["subtype"] == "init" {
                return ev["tools"].clone();
            }
        }
        panic!("no init event");
    });
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == &json!(tool_name(PROPOSE_TASK))),
        "{tools}"
    );
}
