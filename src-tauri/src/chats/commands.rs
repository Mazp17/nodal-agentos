//! Chat Tauri commands (signatures in `src/domain/api.ts`). Thin: they validate ids, run
//! `ops` against the database and drive the chat's process. What the process prints
//! arrives as `nodal://chat` events.

use tauri::State;

use crate::db::queries::chats;
use crate::domain::Chat;
use crate::events::Kind;
use crate::runs::claude_fs;
use crate::runs::types::Transcript;
use crate::util::{blocking, check_id, now_ms};
use crate::work::WorkState;

use super::process::ChatLive;
use super::{chat_repo, ops, spec, ChatPatch, ChatState, NewChat};

async fn db<T, F>(state: &WorkState, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut rusqlite::Connection) -> Result<T, String> + Send + 'static,
{
    Ok(crate::db::with_db(&state.0.db, move |c| f(c).map_err(crate::db::DbError::Invalid)).await?)
}

#[tauri::command]
pub async fn list_chats(state: State<'_, WorkState>, project_id: String) -> Result<Vec<Chat>, String> {
    check_id(&project_id, "project")?;
    db(&state, move |c| Ok(chats::list(c, &project_id)?)).await
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
    let mcp = super::context::nodal_mcp_bin();
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

/// The chat's history, read from its Claude Code session. `None` before the first message.
#[tauri::command]
pub async fn get_chat_transcript(state: State<'_, WorkState>, id: String, limit: Option<u32>) -> Result<Option<Transcript>, String> {
    check_id(&id, "chat")?;
    let (chat, repos) = db(&state, move |c| {
        let chat = chats::get(c, &id)?;
        let repos = crate::db::queries::repos::list(c, Some(&chat.project_id))?;
        Ok((chat, repos))
    })
    .await?;
    let Some(sid) = chat.session_id.clone().filter(|s| claude_fs::is_valid_session_id(s)) else { return Ok(None) };
    let Some(claude_dir) = state.0.env.claude_dir.clone() else {
        return Err("Couldn't locate the Claude Code folder ($HOME is not set).".into());
    };
    let cwd = chat_repo(&chat, &repos).map(|r| r.path.clone()).unwrap_or_default();
    let limit = limit.unwrap_or(claude_fs::TRANSCRIPT_DEFAULT_LIMIT).clamp(1, claude_fs::TRANSCRIPT_MAX_LIMIT) as usize;
    blocking(move || {
        let Some(path) = claude_fs::find_session_jsonl(&claude_dir.join("projects"), &cwd, &sid) else { return Ok(None) };
        let t = claude_fs::read_session_transcript(&path, &chat.id, chat.title.clone(), chat.launch.model.clone(), limit)?;
        Ok(t.map(with_first_message))
    })
    .await
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
mod tests {
    use super::*;
    use crate::runs::types::TranscriptItem;

    #[test]
    fn first_message_goes_back_into_the_items() {
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"Split the checkout into tasks"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Here is a plan."}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":"Thanks"}}"#,
        ];
        let path = std::env::temp_dir().join(format!("nodal-chat-transcript-{}.jsonl", std::process::id()));
        std::fs::write(&path, lines.join("\n")).unwrap();
        let t = claude_fs::read_session_transcript(&path, "c1", None, None, 10).unwrap().unwrap();
        let _ = std::fs::remove_file(&path);
        let t = with_first_message(t);
        let texts: Vec<_> = t
            .items
            .iter()
            .map(|i| match i {
                TranscriptItem::User { text, .. } => format!("user: {text}"),
                TranscriptItem::Text { text, .. } => format!("claude: {text}"),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(texts, ["user: Split the checkout into tasks", "claude: Here is a plan.", "user: Thanks"]);
        assert_eq!((t.prompt.as_deref(), t.total_items), (None, 3));

        let cut = Transcript { omitted: 2, prompt: Some("first".into()), ..t.clone() };
        assert_eq!(with_first_message(cut).prompt.as_deref(), Some("first"), "not next to a gap");
    }
}
