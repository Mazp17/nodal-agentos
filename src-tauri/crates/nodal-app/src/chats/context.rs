//! What a chat knows of Nodal: the `nodal` MCP server in chat mode (with `propose_task`),
//! the project's other repos (`--add-dir`, except those under its root folder when the chat
//! runs there) and a system prompt with the project's details.
//!
//! Flags verified with the real CLI (2.1.283): `--mcp-config` takes an inline JSON string and
//! its `nodal` server replaces a user-registered one with the same name; `--allowedTools`
//! takes a comma-joined list that pre-approves MCP tools even with `--permission-prompt-tool
//! stdio`; `--add-dir` is variadic and stops at the next flag (there is no positional prompt
//! in stream-json mode). The system prompt is recorded on the session's first request and
//! reused on `--resume` (`--system-prompt-snapshot` defaults to on).

use std::path::Path;

use serde_json::json;

use nodal_domain::model::{Chat, Project, Repo};
use nodal_domain::util::clip_chars;
use nodal_mcp_proto::stdio::CHAT_FLAG;
use nodal_mcp_proto::tools::PROPOSE_TASK;

use super::ChatCwd;

/// Key of the MCP server: the chat sees its tools as `mcp__nodal__<tool>` (mirror of
/// `PROPOSE_TASK_TOOL` in `api.ts`).
pub const MCP_SERVER: &str = "nodal";
/// Nodal tools the chat uses without asking: they only read the board or show a card.
const PRE_ALLOWED: [&str; 5] = ["list_projects", "list_tasks", "get_task", "get_run", PROPOSE_TASK];
const DESCRIPTION_MAX: usize = 2000;

pub fn tool_name(tool: &str) -> String {
    format!("mcp__{MCP_SERVER}__{tool}")
}

/// Flags that give the chat its Nodal context, each value its own argument. `cwd` is where
/// it runs; `mcp` is `nodal-mcp`, when found.
pub fn args(chat: &Chat, project: &Project, repos: &[Repo], cwd: ChatCwd, mcp: Option<&Path>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(bin) = mcp {
        let config = json!({"mcpServers": {MCP_SERVER: {"command": bin.to_string_lossy(), "args": [CHAT_FLAG]}}});
        out.push("--mcp-config".into());
        out.push(config.to_string());
        out.push("--allowedTools".into());
        out.push(PRE_ALLOWED.map(tool_name).join(","));
    }
    if chat.repo_id.is_none() {
        let others: Vec<String> = repos.iter().filter(|r| !cwd.covers(r)).map(|r| r.path.clone()).collect();
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
pub fn system_prompt(chat: &Chat, project: &Project, repos: &[Repo], cwd: ChatCwd, has_mcp: bool) -> String {
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
    if let ChatCwd::Root(root) = cwd {
        s.push_str(&format!("Project root folder: {root} (this chat runs here)\n"));
    }
    s.push_str("Repos:\n");
    for r in repos {
        let here = if matches!(cwd, ChatCwd::Repo(c) if c.id == r.id) { " (this chat runs here)" } else { "" };
        s.push_str(&format!("- {}: {} (id {}){here}\n", one_line(&r.name), r.path, r.id));
    }
    if repos.is_empty() {
        s.push_str("- none yet\n");
    }
    s.push('\n');
    match cwd {
        ChatCwd::Repo(r) if chat.repo_id.is_some() => s.push_str(&format!(
            "Scope: only the repo {} ({}). Work in it and leave the project's other repos alone unless the user asks.\n\n",
            one_line(&r.name),
            r.path
        )),
        ChatCwd::Repo(_) => s.push_str("Scope: the whole project: its tasks, its runs and all the repos above.\n\n"),
        ChatCwd::Root(root) => s.push_str(&format!(
            "Scope: the whole project: its tasks, its runs, all the repos above and everything else in its root folder \
             {root}, such as docs and notes. This chat runs at the project root. The root folder itself is not a repo: \
             tasks and runs always target one of the repos above.\n\n"
        )),
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
mod tests;
