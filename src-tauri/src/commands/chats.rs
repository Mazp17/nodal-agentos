//! Chats Tauri commands (signatures in `src/domain/api.ts`): thin wrappers over `state.chats`.
//! What a chat's process prints arrives on the `ChatEnvelope` channel `attach_chat_channel`
//! registered for it (P05), or on the `nodal://chat` broadcast for one that never attached.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio::runtime::Handle;

use nodal_app::chats::{ChatPatch, NewChat};
use nodal_app::App;
use nodal_domain::model::chat::{ChatEnvelope, ChatLive, ClaudeDefaults, ImageInput, RewindResult};
use nodal_domain::model::claude::{SubagentRef, Transcript};
use nodal_domain::model::Chat;
use nodal_domain::ports::{ChangeNotifier, ChatHooks, ChatRuntime, SessionFiles};
use nodal_store::Db;

use crate::adapters::chat_sink::TauriChatSink;
use crate::adapters::events::Events;

use super::CommandError;

/// Builds the one `ChatProcesses` instance (`nodal_host::claude::chats::ChatProcesses`), with a
/// `TauriChatSink` (managed separately so `attach_chat_channel`/`detach_chat_channel` can reach
/// it) and a `ChatHooksImpl` for the process's DB side effects (saving the session id,
/// refreshing the title), and starts its reaper.
pub fn runtime(
    h: &AppHandle,
    db: &Db,
    events: &Events,
    sessions: Arc<dyn SessionFiles>,
    claude_dir: Option<PathBuf>,
    rt: &Handle,
) -> Arc<dyn ChatRuntime> {
    let notifier: Arc<dyn ChangeNotifier> = Arc::new(events.clone());
    let hooks: Arc<dyn ChatHooks> = Arc::new(nodal_app::chats::hooks::ChatHooksImpl::new(
        db.clone(),
        notifier,
        sessions,
        claude_dir,
        rt.clone(),
    )
    .with_tasks(Arc::new(nodal_host::adapters::HostGit), Arc::new(nodal_host::adapters::SystemClock)));
    let sink = Arc::new(TauriChatSink::new(h.clone()));
    // Managed separately from the `Arc<dyn ChatSink>` below so `attach_chat_channel` /
    // `detach_chat_channel` can reach the concrete type (registering a channel isn't part of
    // the `ChatSink` port: it's IPC/Tauri-only glue, which nodal-domain can't know about).
    h.manage(sink.clone());
    let runtime: Arc<dyn ChatRuntime> = nodal_host::claude::chats::ChatProcesses::new(sink, hooks, rt.clone());
    runtime.start_reaper();
    runtime
}

/// Registers (or replaces) `id`'s channel: from now on its events arrive there instead of the
/// `nodal://chat` broadcast. The frontend calls this before a chat's first message, and again
/// after a full reload, and keeps it for the chat's lifetime.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn attach_chat_channel(
    sink: State<'_, Arc<TauriChatSink>>,
    id: String,
    on_event: Channel<ChatEnvelope>,
) -> Result<(), CommandError> {
    sink.attach(id, on_event);
    Ok(())
}

/// Stops delivering `id`'s events (the chat was deleted).
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn detach_chat_channel(sink: State<'_, Arc<TauriChatSink>>, id: String) -> Result<(), CommandError> {
    sink.detach(&id);
    Ok(())
}

/// Nothing to do yet: `runtime` already starts the reaper. Kept for the frozen setup order
/// (`execution::setup` → `mcp::setup` → `chats::setup`).
pub fn setup(_h: &AppHandle, _app: &Arc<App>) {}

/// A project's chats, or every project's with no `project_id`.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_chats(state: State<'_, Arc<App>>, project_id: Option<String>) -> Result<Vec<Chat>, CommandError> {
    Ok(state.chats.list_chats(project_id).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn create_chat(state: State<'_, Arc<App>>, project_id: String, input: NewChat) -> Result<Chat, CommandError> {
    Ok(state.chats.create_chat(project_id, input).await?)
}

/// New settings apply from the next message: an idle process restarts with `--resume`.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn update_chat(state: State<'_, Arc<App>>, id: String, patch: ChatPatch) -> Result<Chat, CommandError> {
    Ok(state.chats.update_chat(id, patch).await?)
}

/// Pins the chat to the top of its project's list, or unpins it.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn set_chat_pinned(state: State<'_, Arc<App>>, id: String, pinned: bool) -> Result<Chat, CommandError> {
    Ok(state.chats.set_chat_pinned(id, pinned).await?)
}

/// Stops its process and forgets the chat. The Claude Code session stays on disk.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn delete_chat(state: State<'_, Arc<App>>, id: String) -> Result<(), CommandError> {
    Ok(state.chats.delete_chat(id).await?)
}

/// Hides the chat from the sidebar, stopping its process first.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn archive_chat(state: State<'_, Arc<App>>, id: String) -> Result<Chat, CommandError> {
    Ok(state.chats.set_chat_archived(id, true).await?)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn unarchive_chat(state: State<'_, Arc<App>>, id: String) -> Result<Chat, CommandError> {
    Ok(state.chats.set_chat_archived(id, false).await?)
}

/// Links the chat to a task (`None` unlinks it); refused when its folder would change.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn link_chat_task(state: State<'_, Arc<App>>, id: String, task_id: Option<String>) -> Result<Chat, CommandError> {
    Ok(state.chats.link_chat_task(id, task_id).await?)
}

/// Sends a message, starting (or resuming) the chat's process if it isn't running.
/// Returns the chat, named after its first message.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn send_chat_message(state: State<'_, Arc<App>>, id: String, text: String, images: Vec<ImageInput>) -> Result<Chat, CommandError> {
    Ok(state.chats.send_chat_message(id, text, images).await?)
}

/// Allows or denies a pending permission request. `message` is what Claude reads on a denial.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn respond_chat_permission(
    state: State<'_, Arc<App>>,
    id: String,
    request_id: String,
    allow: bool,
    message: Option<String>,
    answers: Option<std::collections::BTreeMap<String, String>>,
) -> Result<(), CommandError> {
    Ok(state.chats.respond_chat_permission(id, request_id, allow, message, answers).await?)
}

/// Stops the running turn; the chat keeps its process for the next message.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn interrupt_chat(state: State<'_, Arc<App>>, id: String) -> Result<(), CommandError> {
    Ok(state.chats.interrupt_chat(id).await?)
}

/// Stops one of the chat's background shells; the chat keeps running.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn stop_chat_shell(state: State<'_, Arc<App>>, id: String, task_id: String) -> Result<(), CommandError> {
    Ok(state.chats.stop_chat_shell(id, task_id).await?)
}

/// Rewinds the chat to just before its user message `message_uuid`: files and conversation
/// (`dry_run` only reports the files it would change). Fails while a turn runs.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn rewind_chat(
    state: State<'_, Arc<App>>,
    id: String,
    message_uuid: String,
    dry_run: bool,
) -> Result<RewindResult, CommandError> {
    Ok(state.chats.rewind_chat(id, message_uuid, dry_run).await?)
}

/// Stops the chat's process now instead of waiting for it to go idle.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn stop_chat(state: State<'_, Arc<App>>, id: String) -> Result<(), CommandError> {
    Ok(state.chats.stop_chat(id).await?)
}

/// Whether the chat's process runs and what it is waiting for, for a UI that (re)opens it.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_chat_live(state: State<'_, Arc<App>>, id: String) -> Result<ChatLive, CommandError> {
    Ok(state.chats.get_chat_live(id).await?)
}

/// Agents a chat can pick in the folder it runs in for `repo_id` (`null`: the whole project).
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_chat_agents(
    state: State<'_, Arc<App>>,
    project_id: String,
    repo_id: Option<String>,
) -> Result<Vec<nodal_domain::model::executors::ExecutorInfo>, CommandError> {
    Ok(state.chats.list_chat_agents(project_id, repo_id).await?)
}

/// The model and effort Claude Code uses when a chat leaves them unset, as configured in its
/// settings for `repo_id` (or only the user's settings without one).
/// The built-in intro of the chat system prompt (Settings shows it and restores it).
#[tauri::command]
pub fn default_chat_system_prompt() -> &'static str {
    nodal_app::chats::context::DEFAULT_CHAT_INTRO
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_claude_defaults(state: State<'_, Arc<App>>, repo_id: Option<String>) -> Result<ClaudeDefaults, CommandError> {
    Ok(state.chats.get_claude_defaults(repo_id).await?)
}

/// The chat's history, read from its Claude Code session. `None` before the first message.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_chat_transcript(
    state: State<'_, Arc<App>>,
    id: String,
    limit: Option<u32>,
) -> Result<Option<Transcript>, CommandError> {
    Ok(state.chats.get_chat_transcript(id, limit).await?)
}

/// Subagents the chat's session ran, oldest first.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn list_chat_subagents(state: State<'_, Arc<App>>, chat_id: String) -> Result<Vec<SubagentRef>, CommandError> {
    Ok(state.chats.list_chat_subagents(chat_id).await?)
}

/// One subagent's own conversation. `None` if there's no such transcript.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn get_chat_subagent_transcript(
    state: State<'_, Arc<App>>,
    chat_id: String,
    agent_id: String,
    limit: Option<u32>,
) -> Result<Option<Transcript>, CommandError> {
    Ok(state.chats.get_chat_subagent_transcript(chat_id, agent_id, limit).await?)
}

/// Where the chat is working now (it may have moved into a worktree) and its uncommitted changes.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn chat_workspace(
    state: State<'_, Arc<App>>,
    chat_id: String,
) -> Result<nodal_domain::model::ChatWorkspace, CommandError> {
    Ok(state.chats.chat_workspace(chat_id).await?)
}

/// Opens the task's chat (the existing one, or a new one in the task's worktree) and moves the
/// task to In Progress. Doesn't send: the UI sends `prompt` over its attached channel.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn start_task_chat(
    state: State<'_, Arc<App>>,
    task_id: String,
) -> Result<nodal_app::chats::TaskChatStart, CommandError> {
    Ok(state.chats.prepare_task_chat(task_id).await?)
}
