//! Chat Tauri commands (signatures in `src/domain/api.ts`). Thin: they validate ids, run
//! `ops` against the database and drive the chat's process. What the process prints
//! arrives as `nodal://chat` events.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};
use tokio::runtime::Handle;

use nodal_domain::ports::{ChatRuntime, SessionFiles};

use crate::adapters::events::Events;
use crate::chats::process::ChatLive;
use crate::chats::{chat_cwd, ops, spec, ChatPatch, ChatState, NewChat};
use crate::db::queries::chats;
use crate::db::Db;
use crate::domain::Chat;
use crate::events::Kind;
use crate::runs::claude_fs;
use crate::runs::claude_settings::{self, ClaudeDefaults};
use crate::runs::types::Transcript;
use crate::util::{blocking, check_id, now_ms};
use crate::work::WorkState;

/// Builds the one `ChatProcesses` instance (today's `chats::Chats::new`), manages it as the
/// legacy `ChatState` and starts its reaper — exactly like today's setup — then hands back
/// the same runtime as a port, for `nodal_app::Deps`. `sessions`, `claude_dir` and `rt` are
/// unused today: the legacy path builds its own hooks and runtime handle; wave 3b switches
/// this to build `ChatHooksImpl` from them instead.
pub fn runtime(
    h: &AppHandle,
    db: &Db,
    events: &Events,
    _sessions: Arc<dyn SessionFiles>,
    _claude_dir: Option<PathBuf>,
    _rt: &Handle,
) -> Arc<dyn ChatRuntime> {
    let handle = h.clone();
    let emit: crate::chats::process::Emit = Arc::new(move |ev| {
        if let Err(e) = tauri::Emitter::emit(&handle, crate::chats::process::EVENT, ev) {
            eprintln!("chats: {e}");
        }
    });
    let chats = crate::chats::Chats::new(db.clone(), events.clone(), emit);
    chats.start_reaper();
    let runtime: Arc<dyn ChatRuntime> = chats.0.clone();
    h.manage(ChatState(chats));
    runtime
}

/// Nothing to do yet: `runtime` already manages the legacy `ChatState` and starts the
/// reaper. Wave 3b uses this hook once `app.chats` is real.
pub fn setup(_h: &AppHandle, _app: &Arc<nodal_app::App>) {}

/// Stops every process of the project's chats (today's `ChatState.stop_project`).
pub fn stop_project(h: &AppHandle, project_id: &str) {
    if let Some(cs) = h.try_state::<ChatState>() {
        cs.0.stop_project(project_id);
    }
}

async fn db<T, F>(state: &WorkState, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut crate::db::Connection) -> Result<T, String> + Send + 'static,
{
    Ok(crate::db::with_db(&state.0.db, move |c| f(c).map_err(crate::db::DbError::Invalid)).await?)
}

/// A project's chats, or every project's with no `project_id`.
#[tauri::command]
pub async fn list_chats(state: State<'_, WorkState>, project_id: Option<String>) -> Result<Vec<Chat>, String> {
    match project_id {
        Some(pid) => {
            check_id(&pid, "project")?;
            db(&state, move |c| Ok(chats::list(c, &pid)?)).await
        }
        None => db(&state, |c| Ok(chats::list_all(c)?)).await,
    }
}

#[tauri::command]
pub async fn create_chat(state: State<'_, WorkState>, project_id: String, input: NewChat) -> Result<Chat, String> {
    check_id(&project_id, "project")?;
    let chat = db(&state, move |c| ops::create(c, &project_id, &input, now_ms())).await?;
    state.0.events.notify(Kind::Chats, Some(&chat.project_id));
    Ok(chat)
}

/// New settings apply from the next message: an idle process restarts with `--resume`.
#[tauri::command]
pub async fn update_chat(state: State<'_, WorkState>, id: String, patch: ChatPatch) -> Result<Chat, String> {
    check_id(&id, "chat")?;
    let chat = db(&state, move |c| ops::update(c, &id, &patch)).await?;
    state.0.events.notify(Kind::Chats, Some(&chat.project_id));
    Ok(chat)
}

/// Stops its process and forgets the chat. The Claude Code session stays on disk.
#[tauri::command]
pub async fn delete_chat(state: State<'_, WorkState>, chats_state: State<'_, ChatState>, id: String) -> Result<(), String> {
    check_id(&id, "chat")?;
    chats_state.0.stop(&id);
    let project_id = db(&state, move |c| {
        let chat = chats::get(c, &id)?;
        chats::delete(c, &id)?;
        Ok(chat.project_id)
    })
    .await?;
    state.0.events.notify(Kind::Chats, Some(&project_id));
    Ok(())
}

/// Sends a message, starting (or resuming) the chat's process if it isn't running.
/// Returns the chat, named after its first message.
#[tauri::command]
pub async fn send_chat_message(
    state: State<'_, WorkState>,
    chats_state: State<'_, ChatState>,
    id: String,
    text: String,
) -> Result<Chat, String> {
    check_id(&id, "chat")?;
    let body = text.clone();
    let (chat, project, repos) = db(&state, move |c| ops::prepare_send(c, &id, &body, now_ms())).await?;
    let mcp = crate::chats::context::nodal_mcp_bin();
    let spec = spec(&chat, &project, &repos, mcp.as_deref())?;
    chats_state.0.send(&chat.id, &chat.project_id, chat.session_id.as_deref(), spec, &text)?;
    state.0.events.notify(Kind::Chats, Some(&chat.project_id));
    Ok(chat)
}

/// Allows or denies a pending permission request. `message` is what Claude reads on a denial.
#[tauri::command]
pub fn respond_chat_permission(
    chats_state: State<'_, ChatState>,
    id: String,
    request_id: String,
    allow: bool,
    message: Option<String>,
) -> Result<(), String> {
    check_id(&id, "chat")?;
    chats_state.0.respond(&id, &request_id, allow, message.as_deref())
}

/// Stops the running turn; the chat keeps its process for the next message.
#[tauri::command]
pub fn interrupt_chat(chats_state: State<'_, ChatState>, id: String) -> Result<(), String> {
    check_id(&id, "chat")?;
    chats_state.0.interrupt(&id)
}

/// Stops the chat's process now instead of waiting for it to go idle.
#[tauri::command]
pub fn stop_chat(chats_state: State<'_, ChatState>, id: String) -> Result<(), String> {
    check_id(&id, "chat")?;
    chats_state.0.stop(&id);
    Ok(())
}

/// Whether the chat's process runs and what it is waiting for, for a UI that (re)opens it.
#[tauri::command]
pub fn get_chat_live(chats_state: State<'_, ChatState>, id: String) -> Result<ChatLive, String> {
    check_id(&id, "chat")?;
    Ok(chats_state.0.live(&id))
}

/// The model and effort Claude Code uses when a chat leaves them unset, as configured in its
/// settings for `repo_id` (or only the user's settings without one).
#[tauri::command]
pub async fn get_claude_defaults(state: State<'_, WorkState>, repo_id: Option<String>) -> Result<ClaudeDefaults, String> {
    let repo_path = match repo_id {
        Some(id) => {
            check_id(&id, "repo")?;
            Some(db(&state, move |c| Ok(crate::db::queries::repos::get(c, &id)?.path)).await?)
        }
        None => None,
    };
    let claude = state.0.env.claude_dir.clone();
    blocking(move || Ok(claude_settings::read(claude.as_deref(), repo_path.as_deref().map(std::path::Path::new)))).await
}

/// The chat's history, read from its Claude Code session. `None` before the first message.
#[tauri::command]
pub async fn get_chat_transcript(state: State<'_, WorkState>, id: String, limit: Option<u32>) -> Result<Option<Transcript>, String> {
    check_id(&id, "chat")?;
    let (chat, project, repos) = db(&state, move |c| {
        let chat = chats::get(c, &id)?;
        let project = crate::db::queries::projects::get(c, &chat.project_id)?;
        let repos = crate::db::queries::repos::list(c, Some(&chat.project_id))?;
        Ok((chat, project, repos))
    })
    .await?;
    let Some(sid) = chat.session_id.clone().filter(|s| claude_fs::is_valid_session_id(s)) else { return Ok(None) };
    let Some(claude_dir) = state.0.env.claude_dir.clone() else {
        return Err("Couldn't locate the Claude Code folder ($HOME is not set).".into());
    };
    let cwd = chat_cwd(&chat, &project, &repos).map(|c| c.path().to_string()).unwrap_or_default();
    let limit = limit.unwrap_or(claude_fs::TRANSCRIPT_DEFAULT_LIMIT).clamp(1, claude_fs::TRANSCRIPT_MAX_LIMIT) as usize;
    let (chat_id, project_id, known_title) = (chat.id.clone(), chat.project_id.clone(), chat.session_title.clone());
    let (transcript, title) = blocking(move || {
        let Some(path) = claude_fs::find_session_jsonl(&claude_dir.join("projects"), &cwd, &sid) else { return Ok((None, None)) };
        let t = claude_fs::read_session_transcript(&path, &chat.id, chat.title.clone(), chat.launch.model.clone(), limit)?;
        Ok((t.map(with_first_message), claude_fs::session_title(&path)))
    })
    .await?;
    // Chats from before `session_title`, or renamed with `/rename` outside Nodal.
    if let Some(title) = title.filter(|t| known_title.as_ref() != Some(t)) {
        if db(&state, move |c| Ok(chats::set_session_title(c, &chat_id, &title)?)).await? {
            state.0.events.notify(Kind::Chats, Some(&project_id));
        }
    }
    Ok(transcript)
}

/// The transcript keeps the messages before the first answer apart as `prompt`; in a chat
/// it is the first user message, so it goes back into the items (when none are omitted).
fn with_first_message(mut t: Transcript) -> Transcript {
    if t.omitted == 0 {
        if let Some(p) = t.prompt.take() {
            t.items.insert(0, claude_fs::user_item(&p));
            t.total_items += 1;
        }
    }
    t
}

#[cfg(test)]
mod tests;
