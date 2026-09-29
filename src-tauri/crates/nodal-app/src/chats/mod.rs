//! Chats context: per-project `claude -p` sessions scoped to a project (or narrowed to one
//! repo) and driven from the UI, with permission prompts answered in the app.
//!
//! - `ops`: synchronous CRUD and validation over the database (was `chats::ops`);
//! - `context`: the Nodal context a chat starts with (MCP, other repos, system prompt);
//! - `hooks`: the `ChatHooks` port implementation (DB side effects of a chat process turn);
//! - `transcript`: turns a chat's raw transcript into what the UI expects.
//!
//! The process itself (one `claude -p` per active chat) lives in
//! `nodal_host::claude::chats::ChatProcesses`, reached through `core.chats` (a `ChatRuntime`).

pub mod context;
pub mod hooks;
mod transcript;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use nodal_domain::execution::options;
use nodal_domain::model::chat::{ChatLive, ChatSpec, ClaudeDefaults};
use nodal_domain::model::claude::Transcript;
use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::{Chat, LaunchOptions, Project, Repo};
use nodal_domain::serde_util::double_option;
use nodal_domain::sessions::transcript::{is_valid_session_id, TRANSCRIPT_DEFAULT_LIMIT, TRANSCRIPT_MAX_LIMIT};
use nodal_domain::util::{check_id, clip_chars, new_id};
use nodal_store::{board::projects, board::repos, chats, Conn as Connection, StoreError};

use crate::core::{blocking, AppError, Core};

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
pub fn spec(chat: &Chat, project: &Project, repos: &[Repo], mcp: Option<&Path>) -> Result<ChatSpec, String> {
    let cwd = chat_cwd(chat, project, repos)?;
    let mut args = options::to_args(&chat.launch)?;
    args.extend(context::args(chat, project, repos, cwd, mcp));
    Ok(ChatSpec { cwd: PathBuf::from(cwd.path()), args })
}

/// Chats context: 11 Tauri commands plus `stop_project`, a cross-context hook called when the
/// project is deleted (`App::delete_project`, `flows.rs`).
pub struct Chats {
    core: Arc<Core>,
}

impl Chats {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    /// Runs `f` with the connection; `ops` errors already come ready to display.
    async fn db<T, F>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, String> + Send + 'static,
    {
        Ok(nodal_store::with_db(&self.core.db, move |c| f(c).map_err(StoreError::Invalid)).await?)
    }

    /// A project's chats, or every project's with no `project_id`.
    pub async fn list_chats(&self, project_id: Option<String>) -> Result<Vec<Chat>, AppError> {
        match project_id {
            Some(pid) => {
                check_id(&pid, "project")?;
                self.db(move |c| Ok(chats::list(c, &pid)?)).await
            }
            None => self.db(|c| Ok(chats::list_all(c)?)).await,
        }
    }

    pub async fn create_chat(&self, project_id: String, input: NewChat) -> Result<Chat, AppError> {
        check_id(&project_id, "project")?;
        let now = self.core.clock.now_ms();
        let chat = self.db(move |c| ops::create(c, &project_id, &input, now)).await?;
        self.core.notifier.notify(ChangeKind::Chats, Some(&chat.project_id));
        Ok(chat)
    }

    /// New settings apply from the next message: an idle process restarts with `--resume`.
    pub async fn update_chat(&self, id: String, patch: ChatPatch) -> Result<Chat, AppError> {
        check_id(&id, "chat")?;
        let chat = self.db(move |c| ops::update(c, &id, &patch)).await?;
        self.core.notifier.notify(ChangeKind::Chats, Some(&chat.project_id));
        Ok(chat)
    }

    /// Stops its process and forgets the chat. The Claude Code session stays on disk.
    pub async fn delete_chat(&self, id: String) -> Result<(), AppError> {
        check_id(&id, "chat")?;
        self.core.chats.stop(&id);
        let project_id = self
            .db(move |c| {
                let chat = chats::get(c, &id)?;
                chats::delete(c, &id)?;
                Ok(chat.project_id)
            })
            .await?;
        self.core.notifier.notify(ChangeKind::Chats, Some(&project_id));
        Ok(())
    }

    /// Sends a message, starting (or resuming) the chat's process if it isn't running.
    /// Returns the chat, named after its first message.
    pub async fn send_chat_message(&self, id: String, text: String) -> Result<Chat, AppError> {
        check_id(&id, "chat")?;
        let now = self.core.clock.now_ms();
        let body = text.clone();
        let (chat, project, repos) = self.db(move |c| ops::prepare_send(c, &id, &body, now)).await?;
        let mcp = self.core.env.fs.nodal_mcp_bin();
        let spec = spec(&chat, &project, &repos, mcp.as_deref())?;
        self.core.chats.send(&chat.id, &chat.project_id, chat.session_id.as_deref(), spec, &text)?;
        self.core.notifier.notify(ChangeKind::Chats, Some(&chat.project_id));
        Ok(chat)
    }

    /// Allows or denies a pending permission request. `message` is what Claude reads on a denial.
    pub async fn respond_chat_permission(
        &self,
        id: String,
        request_id: String,
        allow: bool,
        message: Option<String>,
    ) -> Result<(), AppError> {
        check_id(&id, "chat")?;
        Ok(self.core.chats.respond(&id, &request_id, allow, message.as_deref())?)
    }

    /// Stops the running turn; the chat keeps its process for the next message.
    pub async fn interrupt_chat(&self, id: String) -> Result<(), AppError> {
        check_id(&id, "chat")?;
        Ok(self.core.chats.interrupt(&id)?)
    }

    /// Stops the chat's process now instead of waiting for it to go idle.
    pub async fn stop_chat(&self, id: String) -> Result<(), AppError> {
        check_id(&id, "chat")?;
        self.core.chats.stop(&id);
        Ok(())
    }

    /// Whether the chat's process runs and what it is waiting for, for a UI that (re)opens it.
    pub async fn get_chat_live(&self, id: String) -> Result<ChatLive, AppError> {
        check_id(&id, "chat")?;
        Ok(self.core.chats.live(&id))
    }

    /// The model and effort Claude Code uses when a chat leaves them unset, as configured in its
    /// settings for `repo_id` (or only the user's settings without one).
    pub async fn get_claude_defaults(&self, repo_id: Option<String>) -> Result<ClaudeDefaults, AppError> {
        let repo_path = match repo_id {
            Some(id) => {
                check_id(&id, "repo")?;
                Some(self.db(move |c| Ok(repos::get(c, &id)?.path)).await?)
            }
            None => None,
        };
        let claude_dir = self.core.env.claude_dir.clone();
        let claude_config = self.core.env.claude_config.clone();
        blocking(move || Ok(claude_config.claude_defaults(claude_dir.as_deref(), repo_path.as_deref().map(Path::new))))
            .await
            .map_err(AppError::from)
    }

    /// The chat's history, read from its Claude Code session. `None` before the first message.
    pub async fn get_chat_transcript(&self, id: String, limit: Option<u32>) -> Result<Option<Transcript>, AppError> {
        check_id(&id, "chat")?;
        let (chat, project, repos) = self
            .db(move |c| {
                let chat = chats::get(c, &id)?;
                let project = projects::get(c, &chat.project_id)?;
                let repos = repos::list(c, Some(&chat.project_id))?;
                Ok((chat, project, repos))
            })
            .await?;
        let Some(sid) = chat.session_id.clone().filter(|s| is_valid_session_id(s)) else { return Ok(None) };
        let Some(claude_dir) = self.core.env.claude_dir.clone() else {
            return Err("Couldn't locate the Claude Code folder ($HOME is not set).".into());
        };
        let cwd = chat_cwd(&chat, &project, &repos).map(|c| c.path().to_string()).unwrap_or_default();
        let limit = limit.unwrap_or(TRANSCRIPT_DEFAULT_LIMIT).clamp(1, TRANSCRIPT_MAX_LIMIT) as usize;
        let (chat_id, project_id, known_title) = (chat.id.clone(), chat.project_id.clone(), chat.session_title.clone());
        let sessions = self.core.sessions.clone();
        let (transcript, title) = blocking(move || {
            let Some(path) = sessions.find_session_jsonl(&claude_dir.join("projects"), &cwd, &sid) else {
                return Ok((None, None));
            };
            let t = sessions
                .read_session_transcript(&path, &chat.id, chat.title.clone(), chat.launch.model.clone(), limit)
                .map_err(|e| e.to_string())?;
            Ok((t.map(transcript::with_first_message), sessions.session_title(&path)))
        })
        .await
        .map_err(AppError::from)?;
        // Chats from before `session_title`, or renamed with `/rename` outside Nodal.
        if let Some(title) = title.filter(|t| known_title.as_ref() != Some(t)) {
            if self.db(move |c| Ok(chats::set_session_title(c, &chat_id, &title)?)).await? {
                self.core.notifier.notify(ChangeKind::Chats, Some(&project_id));
            }
        }
        Ok(transcript)
    }

    /// Stops every process of the project's chats (called when the project is deleted, through
    /// `App::delete_project`). Not a Tauri command.
    pub fn stop_project(&self, project_id: &str) {
        self.core.chats.stop_project(project_id);
    }
}

#[cfg(test)]
mod tests;
