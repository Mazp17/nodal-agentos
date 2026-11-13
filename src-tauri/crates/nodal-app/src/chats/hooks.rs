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
use nodal_domain::model::TaskStatus;
use nodal_domain::ports::{ChangeNotifier, ChatHooks, Clock, Git, SessionFiles};
use nodal_domain::sessions::transcript::is_valid_session_id;
use nodal_store::board::{repos, tasks};
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
    /// Lets a finished turn of a task chat move its task to Review (see `with_tasks`).
    tasks: Option<(Arc<dyn Git>, Arc<dyn Clock>)>,
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
            tasks: None,
        }
    }

    /// Enables the task side of `turn_ended`: a task chat that leaves changes in its worktree
    /// moves the task from In Progress to Review.
    pub fn with_tasks(mut self, git: Arc<dyn Git>, clock: Arc<dyn Clock>) -> Self {
        self.tasks = Some((git, clock));
        self
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

/// A task chat's turn ended: if the task is In Progress and its worktree has uncommitted
/// changes or commits ahead of the base, the task goes to Review (queuing the status for the
/// linked task manager). `true` if it moved.
fn review_if_changed(db: &Db, git: &dyn Git, now: i64, chat_id: &str) -> Result<bool, String> {
    let (task, repo_path) = {
        let conn = db.guard();
        let Some(task_id) = chats::get(&conn, chat_id).map_err(|e| e.to_string())?.task_id else { return Ok(false) };
        let task = tasks::get(&conn, &task_id).map_err(|e| e.to_string())?;
        let repo = repos::get(&conn, &task.repo_id).map_err(|e| e.to_string())?;
        (task, repo.path)
    };
    let Some(wt) = task.worktree.as_ref().filter(|_| task.status == TaskStatus::InProgress) else { return Ok(false) };
    let st = git.worktree_status(std::path::Path::new(&repo_path), Some(wt)).map_err(|e| e.to_string())?;
    if !(st.exists && (st.dirty || st.ahead > 0)) {
        return Ok(false);
    }
    let mut conn = db.guard();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    // Re-read: the user may have moved the task while git was working.
    let task = tasks::get(&tx, &task.id).map_err(|e| e.to_string())?;
    let moved = task.status == TaskStatus::InProgress
        && crate::board::ops::apply_task_transition(&tx, &task, Some(TaskStatus::InReview), None, None, now)?.is_some();
    tx.commit().map_err(|e| e.to_string())?;
    Ok(moved)
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
        if let Some((git, clock)) = self.tasks.clone() {
            let (db, notifier, chat_id, project_id) =
                (self.db.clone(), self.notifier.clone(), chat_id.to_string(), project_id.clone());
            self.rt.spawn_blocking(move || {
                match review_if_changed(&db, git.as_ref(), clock.now_ms(), &chat_id) {
                    Ok(true) => notifier.notify(ChangeKind::Tasks, Some(&project_id)),
                    Ok(false) => {}
                    Err(e) => eprintln!("chats: {e}"),
                }
            });
        }
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
