//! `ChatHooks` implementation: the app-side effects of a chat process turn (was the bodies of
//! `chats::hooks::LegacyHooks::{session_started, turn_ended}`, verbatim, now reached through
//! the `SessionFiles` port for the session title and running on the injected `rt` instead of
//! `tauri::async_runtime`).

use std::path::PathBuf;
use std::sync::Arc;

use tokio::runtime::Handle;

use nodal_domain::model::events::ChangeKind;
use nodal_domain::ports::{ChangeNotifier, ChatHooks, SessionFiles};
use nodal_domain::sessions::transcript::is_valid_session_id;
use nodal_store::{chats, Db};

/// `session_started`/`turn_ended` run on `rt`, guarding `db` directly (`db.guard()`), and
/// notify through `notifier`. The caller (`ChatProcesses`) already validated `session_id`
/// before calling `session_started`.
pub struct ChatHooksImpl {
    db: Db,
    notifier: Arc<dyn ChangeNotifier>,
    sessions: Arc<dyn SessionFiles>,
    claude_dir: Option<PathBuf>,
    rt: Handle,
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
        }
    }
}

impl ChatHooks for ChatHooksImpl {
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
        let (db, notifier, sessions, chat_id) = (
            self.db.clone(),
            self.notifier.clone(),
            self.sessions.clone(),
            chat_id.to_string(),
        );
        self.rt.spawn_blocking(move || {
            let sid = {
                let conn = db.guard();
                chats::get(&conn, &chat_id).ok().and_then(|c| c.session_id)
            };
            let Some(sid) = sid.filter(|s| is_valid_session_id(s)) else {
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

