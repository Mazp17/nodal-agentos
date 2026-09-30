//! `ChatHooks` implementation: the app-side effects of a chat process turn (was the bodies of
//! `chats::hooks::LegacyHooks::{session_started, turn_ended}`, verbatim, now reached through
//! the `SessionFiles` port for the session title and running on the injected `rt` instead of
//! `tauri::async_runtime`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;

use nodal_domain::model::events::ChangeKind;
use nodal_domain::ports::{ChangeNotifier, ChatHooks, SessionFiles};
use nodal_domain::sessions::transcript::is_valid_session_id;
use nodal_store::{chats, Db};

/// P11: once a chat already has a title, `session_title` (which streams the whole `.jsonl`,
/// `fs::transcript`'s doc says "read after every turn") is re-run at most this often per chat,
/// not on every turn — Claude Code rarely changes a title after its first couple of turns, and
/// a rapid back-and-forth would otherwise re-scan an ever-growing file every time. A chat with
/// no title yet always refreshes right away (below), so a new chat is still named promptly.
const TITLE_REFRESH_INTERVAL: Duration = Duration::from_secs(20);

/// `session_started`/`turn_ended` run on `rt`, guarding `db` directly (`db.guard()`), and
/// notify through `notifier`. The caller (`ChatProcesses`) already validated `session_id`
/// before calling `session_started`.
pub struct ChatHooksImpl {
    db: Db,
    notifier: Arc<dyn ChangeNotifier>,
    sessions: Arc<dyn SessionFiles>,
    claude_dir: Option<PathBuf>,
    rt: Handle,
    title_refreshed_at: Arc<Mutex<HashMap<String, Instant>>>,
}

impl ChatHooksImpl {
    pub fn new(
        db: Db,
        notifier: Arc<dyn ChangeNotifier>,
        sessions: Arc<dyn SessionFiles>,
        claude_dir: Option<PathBuf>,
        rt: Handle,
    ) -> Self {
        Self {
            db,
            notifier,
            sessions,
            claude_dir,
            rt,
            title_refreshed_at: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// `true` at most once per `TITLE_REFRESH_INTERVAL` per chat id; only consulted (and only
/// consumes a slot) for a chat that already has a title — see `turn_ended`.
fn due_for_title_refresh(gate: &Mutex<HashMap<String, Instant>>, chat_id: &str) -> bool {
    let mut m = gate.lock().unwrap_or_else(|p| p.into_inner());
    let now = Instant::now();
    let due = m.get(chat_id).is_none_or(|last| now.duration_since(*last) >= TITLE_REFRESH_INTERVAL);
    if due {
        m.insert(chat_id.to_string(), now);
    }
    due
}

impl ChatHooks for ChatHooksImpl {
    /// Precondition: `session_id` is already validated by the caller (`ChatProcesses`); this
    /// stores it as-is.
    fn session_started(&self, chat_id: &str, session_id: String, project_id: String) {
        let (db, notifier, chat_id) = (self.db.clone(), self.notifier.clone(), chat_id.to_string());
        self.rt.spawn_blocking(move || {
            let conn = db.guard();
            match chats::set_session(&conn, &chat_id, &session_id) {
                Ok(true) => notifier.notify(ChangeKind::Chats, Some(&project_id)),
                Ok(false) => {}
                Err(e) => eprintln!("chats: {e}"),
            }
        });
    }

    /// After a turn, stores the name Claude Code gave the session (it writes it to the session
    /// file, not to the stream).
    /// `cwd` is the running process's `ChatSpec.cwd` (a repo or the project root), not one
    /// recomputed from the project, which may have changed since the process started.
    fn turn_ended(&self, chat_id: &str, cwd: PathBuf, project_id: String) {
        let Some(projects) = self.claude_dir.as_ref().map(|d| d.join("projects")) else {
            return;
        };
        let (db, notifier, sessions, gate, chat_id) = (
            self.db.clone(),
            self.notifier.clone(),
            self.sessions.clone(),
            self.title_refreshed_at.clone(),
            chat_id.to_string(),
        );
        self.rt.spawn_blocking(move || {
            let chat = {
                let conn = db.guard();
                chats::get(&conn, &chat_id).ok()
            };
            let Some(chat) = chat else {
                return;
            };
            // P11: a chat that already has a title only re-scans the transcript when due — a
            // brand new one (no title yet) always tries, so it still gets named promptly.
            if chat.session_title.is_some() && !due_for_title_refresh(&gate, &chat_id) {
                return;
            }
            let Some(sid) = chat.session_id.filter(|s| is_valid_session_id(s)) else {
                return;
            };
            let Some(path) = sessions.find_session_jsonl(&projects, &cwd.to_string_lossy(), &sid)
            else {
                return;
            };
            let Some(title) = sessions.session_title(&path) else {
                return;
            };
            let conn = db.guard();
            match chats::set_session_title(&conn, &chat_id, &title) {
                Ok(true) => notifier.notify(ChangeKind::Chats, Some(&project_id)),
                Ok(false) => {}
                Err(e) => eprintln!("chats: {e}"),
            }
        });
    }
}

#[cfg(test)]
mod tests;
