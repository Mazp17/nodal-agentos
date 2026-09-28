//! Types the `runs` module exposes to the frontend. Mirrored in `src/features/runs/types.ts`.
//! They're ours, not Claude Code's: parsing of the internal format lives in `claude_fs.rs`.
//!
//! Moved to `nodal_domain::model::claude`; re-exported here so current uses don't break.

pub use crate::domain::LaunchOptions;
pub use nodal_domain::model::claude::*;
