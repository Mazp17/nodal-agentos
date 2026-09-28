//! Store error types. `SqliteError`'s `Display` matches `rusqlite::Error`'s byte for byte, so
//! every `.map_err(|e| e.to_string())` site that used to see a raw `rusqlite::Error` keeps
//! seeing the same text.

use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Sqlite(#[from] SqliteError),
    /// Validation or state error, with a message ready to display (in English).
    #[error("{0}")]
    Invalid(String),
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(SqliteError(e))
    }
}

/// Tauri commands reject with a string.
impl From<StoreError> for String {
    fn from(e: StoreError) -> Self {
        e.to_string()
    }
}

/// Opaque `rusqlite::Error`; `Display` mirrors it so `.map_err(|e| e.to_string())` sites stay
/// byte-identical.
#[derive(Debug)]
pub struct SqliteError(pub(crate) rusqlite::Error);

impl fmt::Display for SqliteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for SqliteError {}

impl From<rusqlite::Error> for SqliteError {
    fn from(e: rusqlite::Error) -> Self {
        SqliteError(e)
    }
}

/// "Doesn't exist" error ready to display.
pub(crate) fn not_found(what: &str) -> StoreError {
    StoreError::Invalid(format!("That {what} no longer exists."))
}

/// Is the error a UNIQUE/FK violation? Used to turn it into our own message.
pub(crate) fn is_constraint(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}
