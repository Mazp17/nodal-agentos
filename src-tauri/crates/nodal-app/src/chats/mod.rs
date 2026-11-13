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
mod task;
pub use task::TaskChatStart;
mod transcript;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use nodal_domain::execution::options;
use nodal_domain::model::chat::{ChatLive, ChatSpec, ClaudeDefaults, RewindResult, RunState};
use nodal_domain::model::claude::{SubagentRef, Transcript};
use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::executors::{is_valid_agent_name, ExecutorInfo};
use nodal_domain::model::{Chat, ChatWorkspace, LaunchOptions, Project, Repo};
use nodal_domain::serde_util::double_option;
use nodal_domain::sessions::transcript::{is_valid_message_uuid, is_valid_session_id, TRANSCRIPT_DEFAULT_LIMIT, TRANSCRIPT_MAX_LIMIT};
use nodal_domain::util::{check_id, clip_chars, new_id};
use nodal_store::{board::projects, board::repos, board::tasks, chats, Conn as Connection, StoreError};

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
    #[serde(default)]
    pub agent: Option<String>,
    /// Run in this worktree of `repo_id` (validated against git in `Chats::create_chat`).
    #[serde(default)]
    pub worktree: Option<String>,
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
    pub agent: Option<Option<String>>,
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

pub const AGENT_LOCKED: &str = "A chat keeps the agent it started with: start a new chat to talk to another one.";

/// `None` (or empty) is plain Claude; anything else must be usable as `--agent <name>`.
fn agent(name: Option<&str>) -> Result<Option<String>, String> {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        None => Ok(None),
        Some(n) if is_valid_agent_name(n) => Ok(Some(n.to_string())),
        Some(n) => Err(format!("Invalid agent name: \"{n}\".")),
    }
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
        create_for(conn, project_id, input, None, now)
    }

    /// `create`, linking the chat to the task it works on.
    pub fn create_for(
        conn: &Connection,
        project_id: &str,
        input: &NewChat,
        task_id: Option<&str>,
        now: i64,
    ) -> Result<Chat, String> {
        projects::get(conn, project_id)?;
        let repo_id = input.repo_id.clone().filter(|r| !r.is_empty());
        check_repo(conn, project_id, repo_id.as_deref())?;
        let worktree = input.worktree.clone().filter(|w| !w.is_empty());
        if worktree.is_some() && repo_id.is_none() {
            return Err("A worktree chat needs a repo.".into());
        }
        let chat = Chat {
            id: new_id('c', now),
            project_id: project_id.to_string(),
            repo_id,
            title: title(input.title.as_deref()),
            session_title: None,
            session_id: None,
            agent: agent(input.agent.as_deref())?,
            task_id: task_id.map(str::to_string),
            worktree,
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
        if let Some(a) = &patch.agent {
            let a = agent(a.as_deref())?;
            // Claude Code keeps a session's agent on `--resume` (verified with v2.1: a session
            // started as one agent answers as it whatever `--agent` says), so it can't change.
            if c.session_id.is_some() && a != c.agent {
                return Err(AGENT_LOCKED.into());
            }
            c.agent = a;
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

/// The worktree the chat runs in: its task's, else its own. `None` if it has none or the
/// folder is gone (the chat then runs in the repo as usual).
fn task_worktree(conn: &Connection, chat: &Chat, is_dir: impl Fn(&str) -> bool) -> Result<Option<String>, String> {
    let from_task = match &chat.task_id {
        Some(id) => tasks::get(conn, id)?.worktree.map(|w| w.path),
        None => None,
    };
    Ok(from_task.or_else(|| chat.worktree.clone()).filter(|p| is_dir(p)))
}

/// Where a chat runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChatCwd<'a> {
    /// A task chat: the task's worktree of `repo`.
    Worktree { repo: &'a Repo, path: &'a str },
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
            ChatCwd::Worktree { path, .. } => path,
            ChatCwd::Root(p) => p,
        }
    }

    /// Whether `repo` is already visible from here without `--add-dir`. Both paths are
    /// canonical, and `Path::starts_with` compares whole components (`acme` ≠ `acme-web`).
    pub fn covers(&self, repo: &Repo) -> bool {
        match self {
            ChatCwd::Repo(r) | ChatCwd::Worktree { repo: r, .. } => r.id == repo.id,
            ChatCwd::Root(p) => Path::new(&repo.path).starts_with(p),
        }
    }
}

/// Where the chat runs: the task's `worktree` if it works on one, else its repo, else the
/// project's root folder, else its first repo.
pub fn chat_cwd<'a>(
    chat: &Chat,
    project: &'a Project,
    repos: &'a [Repo],
    worktree: Option<&'a str>,
) -> Result<ChatCwd<'a>, String> {
    if let (Some(path), Some(repo)) = (worktree, repos.iter().find(|r| Some(&r.id) == chat.repo_id.as_ref())) {
        return Ok(ChatCwd::Worktree { repo, path });
    }
    scope_cwd(chat.repo_id.as_deref(), project, repos)
}

/// `chat_cwd` for a Repo pill value (`None`: the whole project), before the chat exists.
pub fn scope_cwd<'a>(repo_id: Option<&str>, project: &'a Project, repos: &'a [Repo]) -> Result<ChatCwd<'a>, String> {
    let found = match (repo_id, project.root_path.as_deref().filter(|p| !p.is_empty())) {
        (Some(id), _) => repos.iter().find(|r| r.id == id).map(ChatCwd::Repo),
        (None, Some(root)) => Some(ChatCwd::Root(root)),
        (None, None) => repos.first().map(ChatCwd::Repo),
    };
    found.ok_or_else(|| "Add a repo to the project before chatting: the chat runs inside one.".to_string())
}

/// How the chat's process is launched: in its repo or the project's root folder, with its
/// own model, effort and permission mode, and the Nodal context (`mcp` is `nodal-mcp`, when
/// found).
pub fn spec(
    chat: &Chat,
    project: &Project,
    repos: &[Repo],
    worktree: Option<&str>,
    mcp: Option<&Path>,
) -> Result<ChatSpec, String> {
    let cwd = chat_cwd(chat, project, repos, worktree)?;
    let mut args = options::to_args(&chat.launch)?;
    if let Some(a) = &chat.agent {
        args.extend(["--agent".to_string(), a.clone()]);
    }
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
        let mut input = input;
        if let Some(wt) = input.worktree.take().filter(|w| !w.is_empty()) {
            let repo_id = input.repo_id.clone().filter(|r| !r.is_empty()).ok_or("A worktree chat needs a repo.")?;
            check_id(&repo_id, "repo")?;
            let repo = self.db(move |c| Ok(repos::get(c, &repo_id)?)).await?;
            let env = self.core.env.clone();
            input.worktree = Some(blocking(move || env.linked_worktree(&repo.path, &wt)).await?.path);
        }
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
        let fs = self.core.env.fs.clone();
        let (chat, project, repos, wt) = self
            .db(move |c| {
                let (chat, project, repos) = ops::prepare_send(c, &id, &body, now)?;
                let wt = task_worktree(c, &chat, |p| fs.is_dir(Path::new(p)))?;
                Ok((chat, project, repos, wt))
            })
            .await?;
        let mcp = self.core.env.fs.nodal_mcp_bin();
        let spec = spec(&chat, &project, &repos, wt.as_deref(), mcp.as_deref())?;
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
        answers: Option<std::collections::BTreeMap<String, String>>,
    ) -> Result<(), AppError> {
        check_id(&id, "chat")?;
        Ok(self.core.chats.respond(&id, &request_id, allow, message.as_deref(), answers.as_ref())?)
    }

    /// Stops the running turn; the chat keeps its process for the next message.
    pub async fn interrupt_chat(&self, id: String) -> Result<(), AppError> {
        check_id(&id, "chat")?;
        Ok(self.core.chats.interrupt(&id)?)
    }

    /// Rewinds the chat to just before its user message `message_uuid`: restores the files
    /// Claude edited since (a `dry_run` only reports them) and, unless it is a `dry_run`,
    /// forgets the conversation from that message on. Files that can't be restored come back
    /// as `files_error`; the conversation is rewound anyway.
    pub async fn rewind_chat(&self, id: String, message_uuid: String, dry_run: bool) -> Result<RewindResult, AppError> {
        check_id(&id, "chat")?;
        if !is_valid_message_uuid(&message_uuid) {
            return Err("Invalid message id.".to_string().into());
        }
        if self.core.chats.live(&id).state == RunState::Busy {
            return Err("Wait for the current turn to finish (or stop it) before rewinding.".to_string().into());
        }
        let Some((chat, path)) = self.chat_session_file(id.clone()).await? else {
            return Err("This chat has no history to rewind.".to_string().into());
        };
        // Resolved first: a message that isn't on the active branch must not touch any file.
        let resume_at = if dry_run {
            None
        } else {
            let (sessions, uuid) = (self.core.sessions.clone(), message_uuid.clone());
            Some(blocking(move || sessions.rewind_point(&path, &uuid).map_err(|e| e.to_string())).await.map_err(AppError::from)?)
        };
        let fs = self.core.env.fs.clone();
        let (project, repos, wt) = {
            let chat = chat.clone();
            self.db(move |c| {
                let project = projects::get(c, &chat.project_id)?;
                let repos = repos::list(c, Some(&chat.project_id))?;
                let wt = task_worktree(c, &chat, |p| fs.is_dir(Path::new(p)))?;
                Ok((project, repos, wt))
            })
            .await?
        };
        let mcp = self.core.env.fs.nodal_mcp_bin();
        let spec = spec(&chat, &project, &repos, wt.as_deref(), mcp.as_deref())?;
        let result = self
            .core
            .chats
            .rewind_files(&id, &chat.project_id, chat.session_id.as_deref(), spec, &message_uuid, dry_run)
            .await?;
        if let Some(resume_at) = resume_at {
            self.core.chats.rewind_conversation(&id, resume_at.as_deref())?;
            if resume_at.is_none() {
                // Nothing precedes the first message: the next one starts a new session.
                let chat_id = id.clone();
                self.db(move |c| Ok(chats::clear_session(c, &chat_id)?)).await?;
            }
            self.core.notifier.notify(ChangeKind::Chats, Some(&chat.project_id));
        }
        Ok(result)
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

    /// Agents a chat can talk to in the folder it runs in for this Repo pill value (`None`: the
    /// whole project): the user's, the folder's `.claude/agents` and enabled plugins'.
    pub async fn list_chat_agents(&self, project_id: String, repo_id: Option<String>) -> Result<Vec<ExecutorInfo>, AppError> {
        check_id(&project_id, "project")?;
        if let Some(id) = &repo_id {
            check_id(id, "repo")?;
        }
        let dir = self
            .db(move |c| {
                let project = projects::get(c, &project_id)?;
                let repos = repos::list(c, Some(&project_id))?;
                Ok(scope_cwd(repo_id.as_deref(), &project, &repos)?.path().to_string())
            })
            .await?;
        let claude_dir = self.core.env.claude_dir.clone();
        let claude_config = self.core.env.claude_config.clone();
        let agents = blocking(move || Ok(claude_config.list_agents(claude_dir.as_deref(), Some(Path::new(&dir)))))
            .await
            .map_err(AppError::from)?;
        Ok(agents.into_iter().map(ExecutorInfo::from).collect())
    }

    /// The chat and its Claude Code session file. `None` before the first message or without the file.
    async fn chat_session_file(&self, id: String) -> Result<Option<(Chat, PathBuf)>, AppError> {
        check_id(&id, "chat")?;
        let fs = self.core.env.fs.clone();
        let (chat, project, repos, wt) = self
            .db(move |c| {
                let chat = chats::get(c, &id)?;
                let project = projects::get(c, &chat.project_id)?;
                let repos = repos::list(c, Some(&chat.project_id))?;
                let wt = task_worktree(c, &chat, |p| fs.is_dir(Path::new(p)))?;
                Ok((chat, project, repos, wt))
            })
            .await?;
        let Some(sid) = chat.session_id.clone().filter(|s| is_valid_session_id(s)) else { return Ok(None) };
        let Some(claude_dir) = self.core.env.claude_dir.clone() else {
            return Err("Couldn't locate the Claude Code folder ($HOME is not set).".into());
        };
        let cwd = chat_cwd(&chat, &project, &repos, wt.as_deref()).map(|c| c.path().to_string()).unwrap_or_default();
        let sessions = self.core.sessions.clone();
        let path = blocking(move || Ok(sessions.find_session_jsonl(&claude_dir.join("projects"), &cwd, &sid)))
            .await
            .map_err(AppError::from)?;
        Ok(path.map(|p| (chat, p)))
    }

    /// The chat's history, read from its Claude Code session. `None` before the first message.
    pub async fn get_chat_transcript(&self, id: String, limit: Option<u32>) -> Result<Option<Transcript>, AppError> {
        let Some((chat, path)) = self.chat_session_file(id).await? else { return Ok(None) };
        let limit = limit.unwrap_or(TRANSCRIPT_DEFAULT_LIMIT).clamp(1, TRANSCRIPT_MAX_LIMIT) as usize;
        let (chat_id, project_id, known_title) = (chat.id.clone(), chat.project_id.clone(), chat.session_title.clone());
        let sessions = self.core.sessions.clone();
        let (transcript, title) = blocking(move || {
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

    /// Subagents (`Agent`/`Task` calls) the chat's session ran, oldest first.
    pub async fn list_chat_subagents(&self, chat_id: String) -> Result<Vec<SubagentRef>, AppError> {
        let Some((_, path)) = self.chat_session_file(chat_id).await? else { return Ok(vec![]) };
        let sessions = self.core.sessions.clone();
        blocking(move || Ok(sessions.list_subagents(&path))).await.map_err(AppError::from)
    }

    /// One subagent's own conversation. `None` if the chat has no session or the agent no file.
    pub async fn get_chat_subagent_transcript(
        &self,
        chat_id: String,
        agent_id: String,
        limit: Option<u32>,
    ) -> Result<Option<Transcript>, AppError> {
        let Some((_, path)) = self.chat_session_file(chat_id).await? else { return Ok(None) };
        let limit = limit.unwrap_or(TRANSCRIPT_DEFAULT_LIMIT).clamp(1, TRANSCRIPT_MAX_LIMIT) as usize;
        let sessions = self.core.sessions.clone();
        blocking(move || sessions.read_subagent_transcript(&path, &agent_id, limit).map_err(|e| e.to_string()))
            .await
            .map_err(AppError::from)
    }

    /// Where the chat works right now and its uncommitted changes there. Claude Code can move
    /// its session into a worktree or switch branches, so the session file's last `cwd` wins
    /// over the launch folder. Read-only and cheap: polled per visible chat.
    pub async fn chat_workspace(&self, id: String) -> Result<ChatWorkspace, AppError> {
        check_id(&id, "chat")?;
        let fs = self.core.env.fs.clone();
        let (chat, project, repos, wt) = self
            .db(move |c| {
                let chat = chats::get(c, &id)?;
                let project = projects::get(c, &chat.project_id)?;
                let repos = repos::list(c, Some(&chat.project_id))?;
                let wt = task_worktree(c, &chat, |p| fs.is_dir(Path::new(p)))?;
                Ok((chat, project, repos, wt))
            })
            .await?;
        let launch = chat_cwd(&chat, &project, &repos, wt.as_deref()).ok().map(|c| c.path().to_string());
        let sid = chat.session_id.clone().filter(|s| is_valid_session_id(s));
        let projects_dir = self.core.env.claude_dir.as_ref().map(|d| d.join("projects"));
        let (sessions, env) = (self.core.sessions.clone(), self.core.env.clone());
        Ok(blocking(move || {
            let located = match (&sid, &projects_dir) {
                (Some(sid), Some(dir)) => sessions
                    .find_session_jsonl(dir, launch.as_deref().unwrap_or(""), sid)
                    .map(|p| sessions.session_location(&p)),
                _ => None,
            };
            let (session_cwd, session_branch) = located.unwrap_or_default();
            let Some(cwd) = pick_cwd(session_cwd, launch, |p| env.fs.is_dir(Path::new(p))) else {
                return Ok(ChatWorkspace::default());
            };
            let dir = Path::new(&cwd);
            let branch = env
                .git
                .current_branch(dir)
                .or_else(|| session_branch.filter(|b| b != "HEAD"));
            let stat = env.git.changes(dir);
            Ok(ChatWorkspace {
                is_worktree: env.git.is_worktree(dir),
                branch,
                files: stat.files,
                additions: stat.additions,
                deletions: stat.deletions,
                cwd: Some(cwd),
            })
        })
        .await?)
    }

    /// Stops every process of the project's chats (called when the project is deleted, through
    /// `App::delete_project`). Not a Tauri command.
    pub fn stop_project(&self, project_id: &str) {
        self.core.chats.stop_project(project_id);
    }
}

/// The session's last known cwd if it still exists, else the launch folder if it does.
fn pick_cwd(session: Option<String>, launch: Option<String>, is_dir: impl Fn(&str) -> bool) -> Option<String> {
    session.filter(|p| is_dir(p)).or(launch.filter(|p| is_dir(p)))
}

#[cfg(test)]
mod tests;
