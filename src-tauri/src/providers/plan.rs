//! Text Nodal generates for imported tasks: `plan.md`, acceptance criteria extracted from
//! the description, and the closing comment posted to the provider. All pure, except
//! `write_plan`.
//!
//! Moved to `nodal_domain::sources::plan_render`; re-exported here so current uses don't
//! break. `plan_path` and `write_plan` moved to `nodal_host::plans` (they touch disk).

/// Only `tests.rs` still calls these (through `use super::*`); rustc's unused-import check
/// doesn't credit that for a glob re-export either, now that wave 3c's `Execution` builds
/// prompts through `nodal_app::board::ops`/`env` instead of this path.
#[allow(unused_imports)]
pub use nodal_domain::sources::plan_render::*;
/// Only `tests.rs` still calls these (through `use super::*`); rustc's unused-import check
/// doesn't credit that for a named (non-glob) re-export.
#[allow(unused_imports)]
pub use nodal_host::plans::{plan_path, write_plan};

#[cfg(test)]
pub mod tests;
