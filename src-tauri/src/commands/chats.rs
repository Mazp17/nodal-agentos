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
use nodal_domain::model::chat::{ChatEnvelope, ChatLive, ClaudeDefaults};
use nodal_domain::model::claude::Transcript;
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
    ));
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

/// Stops its process and forgets the chat. The Claude Code session stays on disk.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn delete_chat(state: State<'_, Arc<App>>, id: String) -> Result<(), CommandError> {
    Ok(state.chats.delete_chat(id).await?)
}

/// Sends a message, starting (or resuming) the chat's process if it isn't running.
/// Returns the chat, named after its first message.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn send_chat_message(state: State<'_, Arc<App>>, id: String, text: String) -> Result<Chat, CommandError> {
    Ok(state.chats.send_chat_message(id, text).await?)
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
) -> Result<(), CommandError> {
    Ok(state.chats.respond_chat_permission(id, request_id, allow, message).await?)
}

/// Stops the running turn; the chat keeps its process for the next message.
#[tauri::command]
#[tracing::instrument(skip_all, level = "info")]
pub async fn interrupt_chat(state: State<'_, Arc<App>>, id: String) -> Result<(), CommandError> {
    Ok(state.chats.interrupt_chat(id).await?)
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

/// The model and effort Claude Code uses when a chat leaves them unset, as configured in its
/// settings for `repo_id` (or only the user's settings without one).
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
