//! `ChatHooks` implementation: the app-side effects of a chat process turn (was the bodies
//! of `Chats::save_session`/`refresh_title`). Filled in wave 3b.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::runtime::Handle;

use nodal_domain::ports::{ChangeNotifier, SessionFiles};
use nodal_store::Db;

/// Filled in wave 3b: `session_started`/`turn_ended` run on `rt`, guarding `db` directly
/// (`db.guard()`), and notify through `notifier`.
#[allow(dead_code)]
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
