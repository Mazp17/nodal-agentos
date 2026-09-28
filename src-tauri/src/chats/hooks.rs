//! DB-side effects of a chat process. Moved out of `process.rs` so the process machinery
//! (now `nodal_host::claude::chats::ChatProcesses`) doesn't need a database: bodies are
//! today's `Chats::save_session`/`refresh_title`, verbatim, wired through the
//! `nodal_domain::ports::ChatHooks` port.

use std::path::PathBuf;

use nodal_domain::ports::ChatHooks;

use crate::db::{queries::chats, Db};
use crate::events::{Events, Kind};
use crate::runs::claude_fs;

pub(crate) struct LegacyHooks {
    pub(crate) db: Db,
    pub(crate) events: Events,
    /// Where session files live, to read each session's title (`None` in tests).
    pub(crate) claude_dir: Option<PathBuf>,
}

impl ChatHooks for LegacyHooks {
    fn session_started(&self, chat_id: &str, session_id: String, project_id: String) {
        let (db, events, chat_id) = (self.db.clone(), self.events.clone(), chat_id.to_string());
        tauri::async_runtime::spawn_blocking(move || {
            let conn = db.lock().unwrap_or_else(|p| p.into_inner());
            match chats::set_session(&conn, &chat_id, &session_id) {
                Ok(true) => events.notify(Kind::Chats, Some(&project_id)),
                Ok(false) => {}
                Err(e) => eprintln!("chats: {e}"),
            }
        });
    }

    /// After a turn, stores the name Claude Code gave the session (it writes it to the session
    /// file, not to the stream).
    /// `cwd` is the running process's `Spec.cwd` (a repo or the project root), not one
    /// recomputed from the project, which may have changed since the process started.
    fn turn_ended(&self, chat_id: &str, cwd: PathBuf, project_id: String) {
        let Some(projects) = self.claude_dir.as_ref().map(|d| d.join("projects")) else { return };
        let (db, events, chat_id) = (self.db.clone(), self.events.clone(), chat_id.to_string());
        tauri::async_runtime::spawn_blocking(move || {
            let sid = {
                let conn = db.lock().unwrap_or_else(|p| p.into_inner());
                chats::get(&conn, &chat_id).ok().and_then(|c| c.session_id)
            };
            let Some(sid) = sid.filter(|s| claude_fs::is_valid_session_id(s)) else { return };
            let Some(path) = claude_fs::find_session_jsonl(&projects, &cwd.to_string_lossy(), &sid) else { return };
            let Some(title) = claude_fs::session_title(&path) else { return };
            let conn = db.lock().unwrap_or_else(|p| p.into_inner());
            match chats::set_session_title(&conn, &chat_id, &title) {
                Ok(true) => events.notify(Kind::Chats, Some(&project_id)),
                Ok(false) => {}
                Err(e) => eprintln!("chats: {e}"),
            }
        });
    }
}
