//! SQLite persistence (rusqlite) for the board, execution and sources domains, plus the
//! legacy-format importer. See `db.rs`/`conn.rs` for the connection and transaction types.

mod chats;
mod conn;
mod db;
mod error;
mod json;
mod rows;
mod schema;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub mod board;
pub mod execution;
pub mod legacy;
pub mod sources;

#[cfg(test)]
mod queries_tests;
#[cfg(test)]
mod tests;
