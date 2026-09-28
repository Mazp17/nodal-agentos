//! Bridge to `nodal-store`. Kept only so existing imports keep working; removed in W4.

pub mod queries;

pub use nodal_store::{rows, with_db, Conn as Connection, Db, DbGuard, StoreError as DbError, DB_FILE};

/// Opens (or creates) the database at `path`, with WAL, FKs on and migrations applied.
pub fn open(path: &std::path::Path) -> Result<Db, DbError> {
    Db::open(path)
}

/// In-memory database with the schema applied (tests).
#[cfg(test)]
pub fn open_in_memory() -> Result<Db, DbError> {
    Db::open_in_memory()
}
