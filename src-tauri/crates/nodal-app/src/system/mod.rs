//! System context: legacy data import. Host diagnostics (`claude_version`, `git_version`,
//! `resolve_git_root`, `scan_git_repos`, `repo_trust`, `attach_run`, `open_terminal_at`) and
//! the PTY commands need no database, so they call `nodal_host` straight from the shell
//! (`SRC/commands/{host,pty}.rs`) instead of going through this facade.

use std::path::PathBuf;
use std::sync::Arc;

use crate::core::{AppError, Core};

/// Legacy data import behind one facade method; `core` gives it `env.data_dir`, `clock` and
/// the shared `db`.
pub struct System {
    core: Arc<Core>,
}

impl System {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    /// "Import data from a previous version…": copies `folder` into a dated backup under the
    /// data dir, then imports it inside one transaction (`nodal_store::legacy`). Same as
    /// today's `import_legacy_data` command, minus the `AppHandle`/`State<Db>` plumbing:
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
