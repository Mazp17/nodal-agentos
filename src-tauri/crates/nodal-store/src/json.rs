//! JSON column helpers shared by `rows` and the query modules.

use rusqlite::types::Type;
use rusqlite::Row;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub(crate) fn to_json<T: Serialize>(v: &T) -> rusqlite::Result<String> {
    serde_json::to_string(v).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

pub(crate) fn opt_json<T: Serialize>(v: &Option<T>) -> rusqlite::Result<Option<String>> {
    v.as_ref().map(to_json).transpose()
}

pub(crate) fn parse_json<T: DeserializeOwned>(s: &str) -> rusqlite::Result<T> {
    serde_json::from_str(s)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e)))
}

pub(crate) fn get_json<T: DeserializeOwned>(row: &Row, col: &str) -> rusqlite::Result<T> {
    parse_json(&row.get::<_, String>(col)?)
}

pub(crate) fn get_opt_json<T: DeserializeOwned>(
    row: &Row,
    col: &str,
) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(col)?
        .as_deref()
        .map(parse_json)
        .transpose()
}
