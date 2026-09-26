//! What a chat knows of Nodal: the `nodal` MCP server in chat mode (with `propose_task`),
//! the project's other repos (`--add-dir`) and a system prompt with the project's details.
//!
//! Flags verified with the real CLI (2.1.283): `--mcp-config` takes an inline JSON string and
//! its `nodal` server replaces a user-registered one with the same name; `--allowedTools`
//! takes a comma-joined list that pre-approves MCP tools even with `--permission-prompt-tool
//! stdio`; `--add-dir` is variadic and stops at the next flag (there is no positional prompt
//! in stream-json mode). The system prompt is recorded on the session's first request and
//! reused on `--resume` (`--system-prompt-snapshot` defaults to on).

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::domain::{Chat, Project, Repo};
use crate::mcp::stdio::CHAT_FLAG;
use crate::mcp::tools::PROPOSE_TASK;
use crate::util::clip_chars;

/// Key of the MCP server: the chat sees its tools as `mcp__nodal__<tool>` (mirror of
/// `PROPOSE_TASK_TOOL` in `api.ts`).
pub const MCP_SERVER: &str = "nodal";
/// Nodal tools the chat uses without asking: they only read the board or show a card.
const PRE_ALLOWED: [&str; 5] = ["list_projects", "list_tasks", "get_task", "get_run", PROPOSE_TASK];
const DESCRIPTION_MAX: usize = 2000;

pub fn tool_name(tool: &str) -> String {
    format!("mcp__{MCP_SERVER}__{tool}")
}

/// `nodal-mcp` next to the app's executable: `Contents/MacOS` in the bundle,
/// `target/<profile>` from source (only if it was built: `cargo build --bin nodal-mcp`).
pub fn nodal_mcp_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("nodal-mcp")).filter(|p| p.is_file())
}

/// Flags that give the chat its Nodal context, each value its own argument. `cwd` is the
/// repo it runs in; `mcp` is `nodal-mcp`, when found.
pub fn args(chat: &Chat, project: &Project, repos: &[Repo], cwd: &Repo, mcp: Option<&Path>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(bin) = mcp {
        let config = json!({"mcpServers": {MCP_SERVER: {"command": bin.to_string_lossy(), "args": [CHAT_FLAG]}}});
        out.push("--mcp-config".into());
        out.push(config.to_string());
        out.push("--allowedTools".into());
        out.push(PRE_ALLOWED.map(tool_name).join(","));
    }
    if chat.repo_id.is_none() {
        let others: Vec<String> = repos.iter().filter(|r| r.id != cwd.id).map(|r| r.path.clone()).collect();
        if !others.is_empty() {
            out.push("--add-dir".into());
            out.extend(others);
        }
    }
    out.push("--append-system-prompt".into());
    out.push(system_prompt(chat, project, repos, cwd, mcp.is_some()));
    out
}

/// The project, its repos and the chat's scope, plus how to turn the conversation into tasks.
pub fn system_prompt(chat: &Chat, project: &Project, repos: &[Repo], cwd: &Repo, has_mcp: bool) -> String {
    let mut s = String::from(
        "You are chatting with the user inside Nodal, a macOS app that keeps a board of tasks per project \
         and runs each task with a Claude Code agent. This chat belongs to one Nodal project. It is mostly \
         for thinking work through with the user and turning it into well-scoped tasks; you can also read \
         and edit code, and the user approves each action in the app.\n\n",
    );
    s.push_str(&format!("Project: {} (key {}, id {})\n", one_line(&project.name), project.key, project.id));
    if let Some(d) = project.description.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        s.push_str(&format!("Description: {}\n", clip_chars(d, DESCRIPTION_MAX)));
    }
    s.push_str("Repos:\n");
    for r in repos {
        let here = if r.id == cwd.id { " (this chat runs here)" } else { "" };
        s.push_str(&format!("- {}: {} (id {}){here}\n", one_line(&r.name), r.path, r.id));
    }
    s.push('\n');
    if chat.repo_id.is_some() {
        s.push_str(&format!(
            "Scope: only the repo {} ({}). Work in it and leave the project's other repos alone unless the user asks.\n\n",
            one_line(&cwd.name),
            cwd.path
        ));
    } else {
        s.push_str("Scope: the whole project: its tasks, its runs and all the repos above.\n\n");
    }
    if has_mcp {
        s.push_str(&format!(
            "The `{MCP_SERVER}` MCP tools read and change this project's board; pass project \"{}\" where a tool takes one.\n\
             - list_tasks, get_task, get_run: read tasks and the results of their runs.\n\
             - {PROPOSE_TASK}: when the conversation lands on work to do, propose it as a task. The user sees a task \
             card in this chat and creates it with one click; the tool itself creates nothing. One task per step \
             that can be run and reviewed on its own, and one repo per task. Check list_tasks first so you don't \
             propose a task that already exists.\n\
             - create_task, update_task: change the board directly; use them only when the user asks you to.\n",
            project.key
        ));
    } else {
        s.push_str(
            "Nodal's tools are not available in this chat. To suggest a task, write it out: title, plan and acceptance criteria.\n",
        );
    }
    s.push_str(
        "A task's executor starts from zero: it gets only the title, the plan and the acceptance criteria, not \
         this conversation. Write the plan with the context, concrete steps (files, functions, commands) and what \
         is out of scope, and give three to six acceptance criteria that can be checked by reading the diff or \
         running a command.",
    );
    s
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LaunchOptions;
    use crate::work::testutil::{project_of, repo_of};

    fn chat(repo_id: Option<&str>) -> Chat {
        Chat {
            id: "c1".into(),
            project_id: "p1".into(),
            repo_id: repo_id.map(Into::into),
            title: None,
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
        let Some(i) = args.iter().position(|a| a == flag) else { return vec![] };
        args[i + 1..].iter().take_while(|a| !a.starts_with("--")).map(String::as_str).collect()
    }

    #[test]
    fn a_project_chat_gets_nodal_mcp_every_other_repo_and_the_project() {
        let (project, repos) = fixture();
        let bin = Path::new("/Applications/Nodal.app/Contents/MacOS/nodal-mcp");
        let args = args(&chat(None), &project, &repos, &repos[0], Some(bin));

        let config: serde_json::Value = serde_json::from_str(value_of(&args, "--mcp-config")[0]).unwrap();
        assert_eq!(config, json!({"mcpServers": {"nodal": {"command": "/Applications/Nodal.app/Contents/MacOS/nodal-mcp", "args": ["--chat"]}}}));
        assert_eq!(
            value_of(&args, "--allowedTools"),
            ["mcp__nodal__list_projects,mcp__nodal__list_tasks,mcp__nodal__get_task,mcp__nodal__get_run,mcp__nodal__propose_task"]
        );
        assert_eq!(value_of(&args, "--add-dir"), ["/Users/me/Code/acme-web"], "the cwd isn't added again");

        let prompt = value_of(&args, "--append-system-prompt")[0];
        assert!(prompt.starts_with("You are"), "a value starting with - would read as a flag");
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
        let args = args(&chat(Some("r2")), &project, &repos, &repos[1], None);
        assert_eq!(args.len(), 2, "{args:?}");
        assert_eq!(args[0], "--append-system-prompt");
        let prompt = &args[1];
        assert!(prompt.contains("Scope: only the repo acme-web (/Users/me/Code/acme-web)"), "{prompt}");
        assert!(prompt.contains("- acme-web: /Users/me/Code/acme-web (id r2) (this chat runs here)"));
        assert!(prompt.contains("tools are not available") && !prompt.contains(PROPOSE_TASK), "{prompt}");
    }

    #[test]
    fn a_long_description_is_clipped() {
        let (mut project, repos) = fixture();
        project.description = Some("x".repeat(DESCRIPTION_MAX + 50));
        let prompt = system_prompt(&chat(None), &project, &repos, &repos[0], true);
        let line = prompt.lines().find(|l| l.starts_with("Description: ")).unwrap();
        assert!(line.chars().count() <= "Description: ".len() + DESCRIPTION_MAX, "{}", line.len());
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
        let args = args(&chat(None), &project, std::slice::from_ref(&repo), &repo, Some(&bin));
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
            stdin.write_all(format!("{msg}\n").as_bytes()).await.unwrap();
            let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
            while let Some(line) = lines.next_line().await.unwrap() {
                let ev: serde_json::Value = serde_json::from_str(&line).unwrap();
                if ev["type"] == "system" && ev["subtype"] == "init" {
                    return ev["tools"].clone();
                }
            }
            panic!("no init event");
        });
        assert!(tools.as_array().unwrap().iter().any(|t| t == &json!(tool_name(PROPOSE_TASK))), "{tools}");
    }
}
