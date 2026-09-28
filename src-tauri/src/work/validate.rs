//! Input validation (projects, repos, tasks, executors, settings). Anything that touches
//! disk is blocking: it's called from `blocking`.
//!
//! Moved to `nodal_domain::board::validate`; re-exported here so current uses don't break.
//! `plan_file` moved to `nodal_host::plans`. `root_path` stays here (it touches disk).

use crate::util::paths;

pub use nodal_domain::board::validate::*;
pub use nodal_host::plans::plan_file;

/// Project root folder: absolute, existing, stored canonical; empty → `None`.
pub fn root_path(s: Option<&str>) -> Result<Option<String>, String> {
    let Some(t) = s.map(str::trim).filter(|t| !t.is_empty()) else { return Ok(None) };
    Ok(Some(paths::canonical_dir(t)?.to_string_lossy().into_owned()))
}
