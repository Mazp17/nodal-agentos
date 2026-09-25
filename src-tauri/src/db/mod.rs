//! SQLite persistence (`<app_data_dir>/nodal.db`).
//! A single connection behind a `Mutex`; async access goes through `with_db`, which
//! runs the closure on a blocking thread so it doesn't stall the runtime.

pub mod queries;
pub mod rows;
pub mod schema;

use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;

pub const DB_FILE: &str = "nodal.db";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub type Db = Arc<Mutex<Connection>>;

#[derive(Debug)]
pub enum DbError {
    Sqlite(rusqlite::Error),
    /// Validation or state error, with a message ready to display (in English).
    Invalid(String),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Sqlite(e) => write!(f, "Database error: {e}"),
            DbError::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sqlite(e)
    }
}

/// Tauri commands reject with a string.
impl From<DbError> for String {
    fn from(e: DbError) -> Self {
        e.to_string()
    }
}

fn configure(conn: &mut Connection) -> Result<(), DbError> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // On `:memory:` it returns "memory" and is ignored; `pragma_update` fails if the pragma
    // returns a row, so it's read with `query_row`.
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    schema::migrate(conn)
}

/// Opens (or creates) the database at `path`, with WAL, FKs on and migrations applied.
pub fn open(path: &Path) -> Result<Db, DbError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| DbError::Invalid(format!("Could not create {}: {e}", dir.display())))?;
    }
    let mut conn = Connection::open(path)?;
    configure(&mut conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// In-memory database with the schema applied (tests).
#[cfg(test)]
pub fn open_in_memory() -> Result<Db, DbError> {
    let mut conn = Connection::open_in_memory()?;
    configure(&mut conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// Runs `f` with the connection on a blocking thread.
pub async fn with_db<T, F>(db: &Db, f: F) -> Result<T, DbError>
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> Result<T, DbError> + Send + 'static,
{
    let db = db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // A panic while holding the lock doesn't leave the connection inconsistent: SQLite
        // rolls back the open transaction when it's dropped, so it keeps being used.
        let mut conn = db.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut conn)
    })
    .await
    .map_err(|e| DbError::Invalid(format!("Database task failed: {e}")))?
}

#[cfg(test)]
mod tests;
