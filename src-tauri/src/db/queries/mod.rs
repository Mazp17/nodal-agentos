//! CRUD for each entity over SQLite. Synchronous functions over `&Connection` (or
//! `&Transaction`, which derefs to `Connection`): commands run them with `with_db`.
//! Business validations live in `work`; here there's only SQL and integrity errors.

pub mod chats;
pub mod hidden_executors;
pub mod projects;
pub mod relations;
pub mod repos;
pub mod runs;
pub mod tasks;

use serde::Serialize;

use super::DbError;

pub(crate) fn to_json<T: Serialize>(v: &T) -> rusqlite::Result<String> {
    serde_json::to_string(v).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

pub(crate) fn opt_json<T: Serialize>(v: &Option<T>) -> rusqlite::Result<Option<String>> {
    v.as_ref().map(to_json).transpose()
}

/// "Doesn't exist" error ready to display.
pub(crate) fn not_found(what: &str) -> DbError {
    DbError::Invalid(format!("That {what} no longer exists."))
}

/// Moved to `nodal_domain::serde_util`; re-exported so current uses don't break.
pub use nodal_domain::serde_util::double_option;

/// Is the error a UNIQUE/FK violation? Used to turn it into our own message.
pub(crate) fn is_constraint(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}

#[cfg(test)]
mod tests;
