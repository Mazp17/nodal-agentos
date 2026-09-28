//! SQLite persistence (`<app_data_dir>/nodal.db`).
//! A single connection behind a `Mutex`; async access goes through `with_db`, which
//! runs the closure on a blocking thread so it doesn't stall the runtime.

use std::path::Path;
use std::sync::{Arc, LockResult, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rusqlite::Connection;

use crate::conn::{Conn, Tx};
use crate::error::StoreError;
use crate::schema;

pub const DB_FILE: &str = "nodal.db";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

fn configure(conn: &mut Connection) -> Result<(), StoreError> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // On `:memory:` it returns "memory" and is ignored; `pragma_update` fails if the pragma
    // returns a row, so it's read with `query_row`.
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    schema::migrate(Conn::wrap_mut(conn))
}

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

impl Db {
    /// Opens (or creates) the database at `path`, with WAL, FKs on and migrations applied.
    pub fn open(path: &Path) -> Result<Db, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| StoreError::Invalid(format!("Could not create {}: {e}", dir.display())))?;
        }
        let mut conn = Connection::open(path)?;
        configure(&mut conn)?;
        Ok(Db(Arc::new(Mutex::new(conn))))
    }

    /// In-memory database with the schema applied (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn open_in_memory() -> Result<Db, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        configure(&mut conn)?;
        Ok(Db(Arc::new(Mutex::new(conn))))
    }

    /// Std semantics: propagates poisoning.
    pub fn lock(&self) -> LockResult<DbGuard<'_>> {
        self.0.lock().map(DbGuard).map_err(|e| PoisonError::new(DbGuard(e.into_inner())))
    }

    /// Poison-recovering (a panic while holding the lock doesn't leave the connection
    /// inconsistent: see `with_db`).
    pub fn guard(&self) -> DbGuard<'_> {
        DbGuard(self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Forward-compat (P10): not used by moved code, which keeps using `with_db`.
    pub async fn read<T, E>(&self, f: impl FnOnce(&Conn) -> Result<T, E> + Send + 'static) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
    {
        let db = self.0.clone();
        match tokio::task::spawn_blocking(move || {
            let conn = db.lock().unwrap_or_else(|p| p.into_inner());
            f(Conn::wrap(&conn))
        })
        .await
        {
            Ok(r) => r,
            Err(e) => Err(StoreError::Invalid(format!("Database task failed: {e}")).into()),
        }
    }

    /// Forward-compat (P10): not used by moved code. `DEFERRED` transaction, committed on `Ok`.
    pub async fn write<T, E>(&self, f: impl FnOnce(&Tx<'_>) -> Result<T, E> + Send + 'static) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
    {
        let db = self.0.clone();
        match tokio::task::spawn_blocking(move || {
            let mut conn = db.lock().unwrap_or_else(|p| p.into_inner());
            let tx = Conn::wrap_mut(&mut conn).transaction().map_err(StoreError::from)?;
            let result = f(&tx)?;
            tx.commit().map_err(StoreError::from)?;
            Ok(result)
        })
        .await
        {
            Ok(r) => r,
            Err(e) => Err(StoreError::Invalid(format!("Database task failed: {e}")).into()),
        }
    }
}

pub struct DbGuard<'a>(MutexGuard<'a, Connection>);

impl std::ops::Deref for DbGuard<'_> {
    type Target = Conn;
    fn deref(&self) -> &Conn {
        Conn::wrap(&self.0)
    }
}

impl std::ops::DerefMut for DbGuard<'_> {
    fn deref_mut(&mut self) -> &mut Conn {
        Conn::wrap_mut(&mut self.0)
    }
}

/// Runs `f` with the connection on a blocking thread. Identical to today's `db::with_db`: a
/// panic while holding the lock doesn't leave the connection inconsistent (SQLite rolls back
/// the open transaction when it's dropped, so it keeps being used).
pub async fn with_db<T, F>(db: &Db, f: F) -> Result<T, StoreError>
where
    T: Send + 'static,
    F: FnOnce(&mut Conn) -> Result<T, StoreError> + Send + 'static,
{
    let db = db.0.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = db.lock().unwrap_or_else(|p| p.into_inner());
        f(Conn::wrap_mut(&mut conn))
    })
    .await
    .map_err(|e| StoreError::Invalid(format!("Database task failed: {e}")))?
}
