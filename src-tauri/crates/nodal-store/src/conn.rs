//! `Conn`/`Tx` are `#[repr(transparent)]` wrappers over `rusqlite::Connection`/`Transaction`.
//! They carry no extra state, so a shared/exclusive reference to the wrapped `rusqlite` type
//! can be reinterpreted as a reference to the wrapper at zero cost: that's the only unsafe
//! code in the crate (`Db`/`DbGuard`, in `db.rs`, use it to hand out `&(mut) Conn` without
//! ever moving or copying the connection).
//!
//! `Conn` has no `Deref` to `rusqlite::Connection`. Instead it re-exposes, as delegator
//! methods, the handful of `rusqlite` methods the moved SQL bodies call, so those bodies
//! compile unchanged against `Conn` the same way they did against `rusqlite::Connection`. The
//! delegators are `pub(crate)` in production builds and `pub` under `test-support` (or
//! `cfg(test)`), for raw SQL in tests outside this crate.

use std::time::Duration;

use crate::error::SqliteError;

/// A SQLite connection. See the module doc for why there's no `Deref` to `rusqlite`.
#[repr(transparent)]
pub struct Conn(rusqlite::Connection);

impl Conn {
    /// SAFETY: `Conn` is `#[repr(transparent)]` over `rusqlite::Connection`, so the two share
    /// layout and a shared reference to one may be reinterpreted as a shared reference to the
    /// other.
    #[allow(unsafe_code)]
    pub(crate) fn wrap(c: &rusqlite::Connection) -> &Conn {
        unsafe { &*(c as *const rusqlite::Connection as *const Conn) }
    }

    /// SAFETY: see `wrap`; the same holds for exclusive references.
    #[allow(unsafe_code)]
    pub(crate) fn wrap_mut(c: &mut rusqlite::Connection) -> &mut Conn {
        unsafe { &mut *(c as *mut rusqlite::Connection as *mut Conn) }
    }

    pub fn transaction(&mut self) -> Result<Tx<'_>, SqliteError> {
        Ok(Tx(self.0.transaction()?))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn prepare(&self, sql: &str) -> rusqlite::Result<rusqlite::Statement<'_>> {
        self.0.prepare(sql)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn prepare(&self, sql: &str) -> rusqlite::Result<rusqlite::Statement<'_>> {
        self.0.prepare(sql)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        self.0.execute(sql, params)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn execute<P: rusqlite::Params>(
        &self,
        sql: &str,
        params: P,
    ) -> rusqlite::Result<usize> {
        self.0.execute(sql, params)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> rusqlite::Result<T>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        self.0.query_row(sql, params, f)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> rusqlite::Result<T>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        self.0.query_row(sql, params, f)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn execute_batch(&self, sql: &str) -> rusqlite::Result<()> {
        self.0.execute_batch(sql)
    }
    /// Not called by moved SQL bodies yet (`db.rs`/`schema.rs` work on the raw connection);
    /// kept for API completeness and future store code.
    #[allow(dead_code)]
    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn execute_batch(&self, sql: &str) -> rusqlite::Result<()> {
        self.0.execute_batch(sql)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pragma_update<V: rusqlite::ToSql>(
        &self,
        schema_name: Option<&str>,
        pragma_name: &str,
        pragma_value: V,
    ) -> rusqlite::Result<()> {
        self.0.pragma_update(schema_name, pragma_name, pragma_value)
    }
    /// Not called by moved SQL bodies yet (`db.rs`'s `configure` works on the raw connection);
    /// kept for API completeness and future store code.
    #[allow(dead_code)]
    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn pragma_update<V: rusqlite::ToSql>(
        &self,
        schema_name: Option<&str>,
        pragma_name: &str,
        pragma_value: V,
    ) -> rusqlite::Result<()> {
        self.0.pragma_update(schema_name, pragma_name, pragma_value)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn busy_timeout(&self, timeout: Duration) -> rusqlite::Result<()> {
        self.0.busy_timeout(timeout)
    }
    /// Not called by moved SQL bodies yet (`db.rs`'s `configure` works on the raw connection);
    /// kept for API completeness and future store code.
    #[allow(dead_code)]
    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn busy_timeout(&self, timeout: Duration) -> rusqlite::Result<()> {
        self.0.busy_timeout(timeout)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn last_insert_rowid(&self) -> i64 {
        self.0.last_insert_rowid()
    }
    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn last_insert_rowid(&self) -> i64 {
        self.0.last_insert_rowid()
    }
}

/// A transaction, `#[repr(transparent)]` over `rusqlite::Transaction` and `Deref` to `Conn`
/// (same cast as `Conn::wrap`: `rusqlite::Transaction` itself derefs to `rusqlite::Connection`,
/// and, like it, only holds a shared reference to the connection — every `Conn` delegator
/// takes `&self`).
#[repr(transparent)]
pub struct Tx<'a>(rusqlite::Transaction<'a>);

impl Tx<'_> {
    pub fn commit(self) -> Result<(), SqliteError> {
        Ok(self.0.commit()?)
    }
}

impl std::ops::Deref for Tx<'_> {
    type Target = Conn;
    fn deref(&self) -> &Conn {
        Conn::wrap(&self.0)
    }
}
