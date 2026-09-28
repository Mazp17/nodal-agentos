//! Moved to `nodal_host::claude::trust`; re-exported so current uses don't break.

use std::path::PathBuf;

pub use nodal_host::claude::trust::{global_config_file, repo_trust_blocking, RepoTrust};
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_host::claude::trust::{trust_of, TrustSource};

/// Whether Claude Code already trusts the folder (no trust dialog on launch).
#[tauri::command]
pub async fn repo_trust(path: String) -> Result<RepoTrust, String> {
    let path = PathBuf::from(path.trim());
    crate::util::blocking(move || repo_trust_blocking(&path, global_config_file().as_deref())).await
}
