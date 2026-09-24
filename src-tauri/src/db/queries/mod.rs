//! CRUD de cada entidad sobre SQLite. Funciones síncronas sobre `&Connection` (o
//! `&Transaction`, que deref-ea a `Connection`): los comandos las corren con `with_db`.
//! Las validaciones de negocio viven en `work`; acá solo SQL y errores de integridad.

pub mod projects;
pub mod relations;
pub mod repos;
pub mod runs;
pub mod tasks;

use serde::{Deserialize, Deserializer, Serialize};

use super::DbError;

pub(crate) fn to_json<T: Serialize>(v: &T) -> rusqlite::Result<String> {
    serde_json::to_string(v).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

pub(crate) fn opt_json<T: Serialize>(v: &Option<T>) -> rusqlite::Result<Option<String>> {
    v.as_ref().map(to_json).transpose()
}

/// Error "no existe" listo para mostrar.
pub(crate) fn not_found(what: &str) -> DbError {
    DbError::Invalid(format!("That {what} no longer exists."))
}

/// Para patches: distingue "campo ausente" (`None`) de `null` (`Some(None)`).
/// Uso: `#[serde(default, deserialize_with = "double_option")] x: Option<Option<T>>`.
pub fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// ¿El error es una violación de UNIQUE/FK? Para traducirlo a un mensaje propio.
pub(crate) fn is_constraint(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}

#[cfg(test)]
mod tests;
