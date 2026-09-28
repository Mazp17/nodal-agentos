//! Moved to `nodal_host::git`; re-exported so current uses don't break.

use std::path::Path;

pub use nodal_host::git::ok;
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_host::git::{run, toplevel, GitOutput, GIT_TIMEOUT};

/// `git` version (`git version 2.x`), using the same resolver as the rest of the app.
#[tauri::command]
pub async fn git_version() -> Result<String, String> {
    crate::util::blocking(|| {
        let out = ok(Path::new("/"), &["--version"])?;
        Ok(out.trim().to_string())
    })
    .await
}

#[cfg(test)]
mod tests;
