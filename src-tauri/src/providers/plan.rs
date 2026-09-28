//! Text Nodal generates for imported tasks: `plan.md`, acceptance criteria extracted from
//! the description, and the closing comment posted to the provider. All pure, except
//! `write_plan`.
//!
//! Moved to `nodal_domain::sources::plan_render`; re-exported here so current uses don't
//! break. `plan_path` and `write_plan` moved to `nodal_host::plans` (they touch disk).

#[cfg(test)]
use super::ExternalItem;

pub use nodal_domain::sources::plan_render::*;
pub use nodal_host::plans::{plan_path, write_plan};

#[cfg(test)]
pub mod tests;
