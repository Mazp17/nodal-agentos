//! Command inputs (mirror of the DTOs in `src/domain/api.ts`).
//! In patches, a missing field = leave as is and `null` = clear (`Option<Option<T>>`).
//!
//! Moved to `nodal_domain::board::dto`; re-exported here so current uses don't break.

pub use nodal_domain::board::dto::*;
