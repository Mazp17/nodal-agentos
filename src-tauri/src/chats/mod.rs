//! Per-project chats: `claude -p` sessions scoped to a project (or narrowed to one repo)
//! and driven from the UI, with permission prompts answered in the app.
//!
//! - `ops`: synchronous CRUD and validation over the database;
//! - `context`: the Nodal context a chat starts with (MCP, other repos, system prompt);
//! - `process`: one `claude` process per active chat and its `nodal://chat` events;
//! - `commands`: the Tauri commands.

pub mod commands;
pub mod context;
pub mod process;

use std::path::{Path, PathBuf};

use crate::db::Connection;
use serde::Deserialize;

use crate::db::queries::{chats, double_option, projects, repos};
use crate::domain::{Chat, LaunchOptions, Project, Repo};
use crate::runs::options;
use crate::util::{clip_chars, new_id};

pub use process::{Chats, Spec};

/// Managed state: the processes of every chat.
#[derive(Clone)]
pub struct ChatState(pub Chats);

const TITLE_MAX: usize = 80;
/// A message goes to stdin as one JSON line: a generous cap keeps a paste from being huge.
pub const MESSAGE_MAX: usize = 100_000;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewChat {
    #[serde(default)]
    pub repo_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(flatten)]
    pub launch: LaunchOptions,
}

/// Missing field = leave as is, `null` = clear.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatPatch {
    #[serde(default, deserialize_with = "double_option")]
    pub title: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub repo_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub model: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub effort: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub permission_mode: Option<Option<String>>,
}

fn title(t: Option<&str>) -> Option<String> {
    let one_line = t?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!one_line.is_empty()).then(|| clip_chars(&one_line, TITLE_MAX))
}

fn check_repo(conn: &Connection, project_id: &str, repo_id: Option<&str>) -> Result<(), String> {
    if let Some(id) = repo_id {
        if repos::get(conn, id)?.project_id != project_id {
            return Err("That repo belongs to another project.".into());
        }
    }
    Ok(())
}

pub mod ops {
    use super::*;

    pub fn create(conn: &Connection, project_id: &str, input: &NewChat, now: i64) -> Result<Chat, String> {
        projects::get(conn, project_id)?;
        let repo_id = input.repo_id.clone().filter(|r| !r.is_empty());
        check_repo(conn, project_id, repo_id.as_deref())?;
        let chat = Chat {
            id: new_id('c', now),
            project_id: project_id.to_string(),
            repo_id,
            title: title(input.title.as_deref()),
            session_title: None,
            session_id: None,
            launch: options::normalize(&input.launch).map_err(|e| e.join("\n"))?,
            created_at: now,
            updated_at: now,
        };
        chats::insert(conn, &chat)?;
        Ok(chat)
    }

    pub fn update(conn: &Connection, id: &str, patch: &ChatPatch) -> Result<Chat, String> {
        let mut c = chats::get(conn, id)?;
        if let Some(t) = &patch.title {
            c.title = title(t.as_deref());
        }
        if let Some(r) = &patch.repo_id {
            let r = r.clone().filter(|r| !r.is_empty());
            check_repo(conn, &c.project_id, r.as_deref())?;
            c.repo_id = r;
        }
        let mut launch = c.launch.clone();
        for (field, value) in [
            (&mut launch.model, &patch.model),
            (&mut launch.effort, &patch.effort),
            (&mut launch.permission_mode, &patch.permission_mode),
        ] {
            if let Some(v) = value {
                *field = v.clone();
            }
        }
        c.launch = options::normalize(&launch).map_err(|e| e.join("\n"))?;
        chats::update(conn, &c)?;
        Ok(c)
    }

    /// Checks the message, marks the chat as used now (naming it after its first message)
    /// and returns what launching it needs.
    pub fn prepare_send(conn: &Connection, id: &str, text: &str, now: i64) -> Result<(Chat, Project, Vec<Repo>), String> {
        if text.trim().is_empty() {
            return Err("The message is empty.".into());
        }
        if text.chars().count() > MESSAGE_MAX {
            return Err(format!("The message is too long (over {MESSAGE_MAX} characters)."));
        }
        let mut c = chats::get(conn, id)?;
        if c.title.is_none() {
            c.title = title(Some(text));
        }
        c.updated_at = now;
        chats::update(conn, &c)?;
        let project = projects::get(conn, &c.project_id)?;
        let repos = repos::list(conn, Some(&c.project_id))?;
        Ok((c, project, repos))
    }
}

/// Where a chat runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChatCwd<'a> {
    /// The chat's repo or, in a project without a root folder, its first repo.
    Repo(&'a Repo),
    /// The project's root folder, for a repo-less chat: it sees the repos and everything
    /// else under it. Never a run target.
    Root(&'a str),
}

impl<'a> ChatCwd<'a> {
    pub fn path(&self) -> &'a str {
        match self {
            ChatCwd::Repo(r) => &r.path,
            ChatCwd::Root(p) => p,
        }
    }

    /// Whether `repo` is already visible from here without `--add-dir`. Both paths are
    /// canonical, and `Path::starts_with` compares whole components (`acme` ≠ `acme-web`).
    pub fn covers(&self, repo: &Repo) -> bool {
        match self {
            ChatCwd::Repo(r) => r.id == repo.id,
            ChatCwd::Root(p) => Path::new(&repo.path).starts_with(p),
        }
    }
}

/// Where the chat runs: its repo, else the project's root folder, else its first repo.
pub fn chat_cwd<'a>(chat: &Chat, project: &'a Project, repos: &'a [Repo]) -> Result<ChatCwd<'a>, String> {
    let found = match (&chat.repo_id, project.root_path.as_deref().filter(|p| !p.is_empty())) {
        (Some(id), _) => repos.iter().find(|r| &r.id == id).map(ChatCwd::Repo),
        (None, Some(root)) => Some(ChatCwd::Root(root)),
        (None, None) => repos.first().map(ChatCwd::Repo),
    };
    found.ok_or_else(|| "Add a repo to the project before chatting: the chat runs inside one.".to_string())
}

/// How the chat's process is launched: in its repo or the project's root folder, with its
/// own model, effort and permission mode, and the Nodal context (`mcp` is `nodal-mcp`, when
/// found).
pub fn spec(chat: &Chat, project: &Project, repos: &[Repo], mcp: Option<&Path>) -> Result<Spec, String> {
    let cwd = chat_cwd(chat, project, repos)?;
    let mut args = options::to_args(&chat.launch)?;
    args.extend(context::args(chat, project, repos, cwd, mcp));
    Ok(Spec { cwd: PathBuf::from(cwd.path()), args })
}

#[cfg(test)]
mod tests;
