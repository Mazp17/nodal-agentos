//! System context: legacy data import. Host diagnostics (`claude_version`, `git_version`,
//! `resolve_git_root`, `scan_git_repos`, `repo_trust`, `attach_run`, `open_terminal_at`) and
//! the PTY commands need no database, so they call `nodal_host` straight from the shell
//! (`SRC/commands/{host,pty}.rs`) instead of going through this facade.

use std::path::PathBuf;
use std::sync::Arc;

use std::path::Path;

use nodal_domain::diff::FileDiff;
use nodal_domain::model::{DirEntry, Repo, RepoFile, RepoWorktree};
use nodal_domain::ports::Git;
use nodal_domain::util::check_id;
use nodal_store::{rows, with_db};

use crate::core::{blocking, AppError, Core};

/// Legacy data import behind one facade method; `core` gives it `env.data_dir`, `clock` and
/// the shared `db`.
pub struct System {
    core: Arc<Core>,
}

impl System {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    async fn repo(&self, repo_id: String) -> Result<Repo, AppError> {
        check_id(&repo_id, "repo")?;
        let repo = with_db(&self.core.db, move |c| rows::get_repo(c, &repo_id)).await?;
        Ok(repo.ok_or("Repo not found.")?)
    }

    /// Branch checked out in the repo's folder; `None` when detached or unknown.
    pub async fn repo_branch(&self, repo_id: String) -> Result<Option<String>, AppError> {
        let repo = self.repo(repo_id).await?;
        let git = self.core.env.git.clone();
        Ok(blocking(move || Ok(git.current_branch(Path::new(&repo.path)))).await?)
    }

    /// The repo's checkouts (main first) for the files panel's picker.
    pub async fn list_repo_worktrees(&self, repo_id: String) -> Result<Vec<RepoWorktree>, AppError> {
        let repo = self.repo(repo_id).await?;
        let git = self.core.env.git.clone();
        Ok(blocking(move || git.list_worktrees(Path::new(&repo.path)).map_err(|e| e.0)).await?)
    }

    /// Runs `f` on a blocking thread with the checkout root: the repo's folder, or `worktree`
    /// when given. Trust boundary: `worktree` must be one of the repo's own worktrees
    /// (compared canonicalized), never an arbitrary path.
    async fn with_root<T: Send + 'static>(
        &self,
        repo_id: String,
        worktree: Option<String>,
        f: impl FnOnce(&dyn Git, &Path) -> Result<T, String> + Send + 'static,
    ) -> Result<T, AppError> {
        let repo = self.repo(repo_id).await?;
        let env = self.core.env.clone();
        Ok(blocking(move || {
            let root = match worktree {
                None => PathBuf::from(&repo.path),
                Some(wt) => {
                    let not_ours = || "Not a worktree of this repo.".to_string();
                    let want = env.fs.canonicalize(Path::new(&wt)).map_err(|_| not_ours())?;
                    let found = env.git.list_worktrees(Path::new(&repo.path)).map_err(|e| e.0)?.into_iter().find(|w| {
                        env.fs.canonicalize(Path::new(&w.path)).is_ok_and(|p| p == want)
                    });
                    PathBuf::from(found.ok_or_else(not_ours)?.path)
                }
            };
            f(&*env.git, &root)
        })
        .await?)
    }

    /// Direct children of `dir` (relative to the checkout root) for the UI's file picker.
    pub async fn list_repo_dir(&self, repo_id: String, dir: String, worktree: Option<String>) -> Result<Vec<DirEntry>, AppError> {
        self.with_root(repo_id, worktree, move |git, root| git.list_dir(root, &dir).map_err(|e| e.0)).await
    }

    /// Uncommitted changes of the checkout vs HEAD (untracked files included), per file with
    /// its hunks, for the files panel's "Changes" view.
    pub async fn repo_changes(&self, repo_id: String, worktree: Option<String>) -> Result<Vec<FileDiff>, AppError> {
        self.with_root(repo_id, worktree, |git, root| {
            let (patch, _) = git.diff(root, None).map_err(|e| e.0)?;
            Ok(nodal_domain::diff::parse(&patch))
        })
        .await
    }

    /// A repo file's text for the viewer (`path` relative to the checkout root).
    pub async fn read_repo_file(&self, repo_id: String, path: String, worktree: Option<String>) -> Result<RepoFile, AppError> {
        self.with_root(repo_id, worktree, move |git, root| git.read_file(root, &path).map_err(|e| e.0)).await
    }

    /// Saves an edited repo file (see `Git::write_file`).
    pub async fn write_repo_file(
        &self,
        repo_id: String,
        path: String,
        content: String,
        version: String,
        force: bool,
        worktree: Option<String>,
    ) -> Result<RepoFile, AppError> {
        self.with_root(repo_id, worktree, move |git, root| {
            git.write_file(root, &path, &content, &version, force).map_err(|e| e.0)
        })
        .await
    }

    /// Discards one file's uncommitted changes (see `Git::revert_file`).
    pub async fn revert_repo_file(&self, repo_id: String, path: String, worktree: Option<String>) -> Result<(), AppError> {
        self.with_root(repo_id, worktree, move |git, root| git.revert_file(root, &path).map_err(|e| e.0)).await
    }

    /// "Import data from a previous version…": copies `folder` into a dated backup under the
    /// data dir, then imports it inside one transaction (`nodal_store::legacy`). Same as
    /// today's `import_legacy_data` command, minus the Tauri app handle/`State<Db>` plumbing:
    /// `data_dir` comes from `core.env`, `now` from `core.clock`, the connection from
    /// `core.db`.
    pub async fn import_legacy(&self, folder: String) -> Result<nodal_store::legacy::LegacyImportReport, AppError> {
        let data_dir = self.core.env.data_dir.clone();
        let now = self.core.clock.now_ms();
        let src = PathBuf::from(folder.trim());
        let dd = data_dir.clone();
        // The copy doesn't take the database lock.
        let (backup_dir, skipped) = tokio::task::spawn_blocking(move || nodal_store::legacy::backup(&src, &dd, now))
            .await
            .map_err(|e| format!("Backup task failed: {e}"))??;
        let db = self.core.db.clone();
        let mut report = nodal_store::with_db(&db, move |conn| {
            nodal_store::legacy::import_backup(conn, &backup_dir, &data_dir, now).map_err(nodal_store::DbError::Invalid)
        })
        .await?;
        report.skipped.splice(0..0, skipped);
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
