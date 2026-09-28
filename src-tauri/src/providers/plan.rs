//! Text Nodal generates for imported tasks: `plan.md`, acceptance criteria extracted from
//! the description, and the closing comment posted to the provider. All pure, except
//! `write_plan`.
//!
//! Moved to `nodal_domain::sources::plan_render`; re-exported here so current uses don't
//! break. `plan_path` and `write_plan` stay here (they touch disk).

use std::path::{Path, PathBuf};

#[cfg(test)]
use super::ExternalItem;

pub use nodal_domain::sources::plan_render::*;

/// `<data_dir>/tasks/<id>/plan.md` (same path as the plan of local tasks).
pub fn plan_path(data_dir: &Path, task_id: &str) -> PathBuf {
    data_dir.join(PLANS_DIR).join(task_id).join(PLAN_FILE)
}

/// Writes the plan if it changed. Returns `true` if it wrote.
pub fn write_plan(path: &Path, content: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == content) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

#[cfg(test)]
pub mod tests;
