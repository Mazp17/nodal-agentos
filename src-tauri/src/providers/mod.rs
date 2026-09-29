//! Task providers moved to `nodal_app::sources` in wave 3b. `plan` and `store` stay as
//! bridges: `work::pump` reaches `plan::{closing_comment, ClosingInfo}` and
//! `commands::execution` reaches `store::moved_ids` directly.

pub mod plan;
pub mod store;
