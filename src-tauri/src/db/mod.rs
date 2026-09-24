//! Persistencia en SQLite (`<app_data_dir>/nodal.db`).
//! Una sola conexión detrás de un `Mutex`; los accesos async pasan por `with_db`, que
//! corre el closure en un hilo bloqueante para no frenar el runtime.

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
    /// Error de validación o de estado, con un mensaje listo para mostrar (en inglés).
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

/// Los comandos de Tauri rechazan con un string.
impl From<DbError> for String {
    fn from(e: DbError) -> Self {
        e.to_string()
    }
}

fn configure(conn: &mut Connection) -> Result<(), DbError> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // En `:memory:` devuelve "memory" y se ignora; `pragma_update` falla si el pragma
    // devuelve fila, así que se lee con `query_row`.
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    schema::migrate(conn)
}

/// Abre (o crea) la base en `path`, con WAL, FKs activas y migraciones aplicadas.
pub fn open(path: &Path) -> Result<Db, DbError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| DbError::Invalid(format!("Could not create {}: {e}", dir.display())))?;
    }
    let mut conn = Connection::open(path)?;
    configure(&mut conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// Base en memoria con el esquema aplicado (tests).
#[cfg(test)]
pub fn open_in_memory() -> Result<Db, DbError> {
    let mut conn = Connection::open_in_memory()?;
    configure(&mut conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// Corre `f` con la conexión en un hilo bloqueante.
pub async fn with_db<T, F>(db: &Db, f: F) -> Result<T, DbError>
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> Result<T, DbError> + Send + 'static,
{
    let db = db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Un panic con el lock tomado no deja la conexión inconsistente: SQLite revierte
        // la transacción abierta al soltarla, así que se sigue usando.
        let mut conn = db.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut conn)
    })
    .await
    .map_err(|e| DbError::Invalid(format!("Database task failed: {e}")))?
}

#[cfg(test)]
mod tests;
