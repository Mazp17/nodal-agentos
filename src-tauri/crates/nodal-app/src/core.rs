//! The pieces every context is built from: paths and ports (`Env`), what a context needs to
//! do its work (`Deps`, `Core`) and what the DB-free sources hub needs (`HubDeps`).

use std::path::PathBuf;
use std::sync::Arc;

use tokio::runtime::Handle;

use nodal_domain::error::HostError;
use nodal_domain::model::providers::ProviderError;
use nodal_domain::ports::{
    ChangeNotifier, ChatRuntime, ClaudeConfig, Clock, Git, LocalFs, PlanFiles,
    SessionFiles,
};
use nodal_store::{Db, StoreError};

use crate::sources;

/// Paths and blocking ports the app depends on (was `work::Env`); same field names and
/// methods, plus the ports the old code reached directly.
#[derive(Clone)]
pub struct Env {
    /// `app_data_dir`: plans in `tasks/<id>/plan.md`, patches of stopped runs.
    pub data_dir: PathBuf,
    /// `~/.nodal/worktrees`.
    pub worktrees_root: PathBuf,
    /// `~/.claude` (or `$CLAUDE_CONFIG_DIR`): agents, workflows and plugins.
    pub claude_dir: Option<PathBuf>,
    pub plans: Arc<dyn PlanFiles>,
    pub fs: Arc<dyn LocalFs>,
    pub git: Arc<dyn Git>,
    pub claude_config: Arc<dyn ClaudeConfig>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .field("data_dir", &self.data_dir)
            .field("worktrees_root", &self.worktrees_root)
            .field("claude_dir", &self.claude_dir)
            .finish_non_exhaustive()
    }
}

impl Env {
    pub fn plan_dir(&self, task_id: &str) -> PathBuf {
        self.data_dir.join("tasks").join(task_id)
    }
    pub fn text_plan_path(&self, task_id: &str) -> PathBuf {
        self.plan_dir(task_id).join("plan.md")
    }
}

/// What `App::new` needs: one database, the ports and the already-built sources hub.
pub struct Deps {
    pub db: Db,
    pub env: Env,
    pub rt: Handle,
    pub clock: Arc<dyn Clock>,
    pub sessions: Arc<dyn SessionFiles>,
    pub notifier: Arc<dyn ChangeNotifier>,
    pub chats: Arc<dyn ChatRuntime>,
    pub sources: Arc<sources::SourcesHub>,
}

/// What `SourcesHub::new` needs. Works without a database (today's `ProvidersState` +
/// `Secrets` + `LinearState`), so provider, key and live-session commands keep working when
/// the app opens without one.
pub struct HubDeps {
    pub db: Option<Db>,
    pub data_dir: PathBuf,
    pub rt: Handle,
    pub clock: Arc<dyn Clock>,
    pub secrets: sources::keys::Secrets,
    pub providers: sources::registry::ProviderRegistry,
    pub notifier: Arc<dyn ChangeNotifier>,
    pub plans: Arc<dyn PlanFiles>,
}

/// The dependencies every context facade shares, minus what only the sources hub needs
/// (that one keeps its own state, since it may run without a database). Every context is
/// still a skeleton, so its fields aren't read yet outside waves 3a-3c.
#[allow(dead_code)]
pub(crate) struct Core {
    pub db: Db,
    pub env: Env,
    pub rt: Handle,
    pub clock: Arc<dyn Clock>,
    pub sessions: Arc<dyn SessionFiles>,
    pub notifier: Arc<dyn ChangeNotifier>,
    pub chats: Arc<dyn ChatRuntime>,
}

impl Env {
    /// The repo's linked (non-main) worktree at `path`, as git lists it (compared
    /// canonicalized). Blocking. Trust boundary: never an arbitrary path or the main checkout.
    pub(crate) fn linked_worktree(&self, repo_path: &str, path: &str) -> Result<nodal_domain::model::RepoWorktree, String> {
        let not_ours = || "Not a linked worktree of this repo.".to_string();
        let want = self.fs.canonicalize(std::path::Path::new(path)).map_err(|_| not_ours())?;
        self.git
            .list_worktrees(std::path::Path::new(repo_path))
            .map_err(|e| e.0)?
            .into_iter()
            .find(|w| !w.is_main && self.fs.canonicalize(std::path::Path::new(&w.path)).is_ok_and(|p| p == want))
            .ok_or_else(not_ours)
    }
}

/// Runs `f` on a blocking thread without stalling the runtime (was `util::blocking`, minus
/// the tauri dependency: `tokio::task::spawn_blocking` works the same from inside an async fn
/// already running on the injected `rt`).
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("Internal error: {e}"))?
}

/// The shell's `CommandError` wraps this with `.to_string()`; ports and the store already
/// have display-ready messages, so the transparent variants keep those verbatim.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Host(#[from] HostError),
    /// `ProviderError` only implements `Display` (it isn't `std::error::Error`), so this
    /// can't use thiserror's `#[from]` (it needs `source()`, below).
    #[error("{0}")]
    Provider(ProviderError),
}

impl From<String> for AppError {
    fn from(e: String) -> Self {
        AppError::Message(e)
    }
}

impl From<&str> for AppError {
    fn from(e: &str) -> Self {
        AppError::Message(e.to_string())
    }
}

impl From<ProviderError> for AppError {
    fn from(e: ProviderError) -> Self {
        AppError::Provider(e)
    }
}

#[cfg(test)]
mod tests;
