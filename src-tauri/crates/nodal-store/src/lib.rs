//! SQLite persistence (rusqlite) for the board, execution and sources domains, plus the
//! legacy-format importer. See `db.rs`/`conn.rs` for the connection and transaction types.
#![deny(unsafe_code)] // conn.rs has the only #[allow(unsafe_code)]

mod conn;
mod db;
mod error;
mod json;
mod schema;

pub use conn::{Conn, Tx};
pub use db::{with_db, Db, DbGuard, DB_FILE};
pub use error::{SqliteError, StoreError};
/// Migration alias, removed in W4.
pub type DbError = StoreError;

pub mod board;
pub mod chats;
pub mod execution;
pub mod legacy;
pub mod rows;
pub mod sources;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

#[cfg(test)]
mod queries_tests;
#[cfg(test)]
mod tests;
