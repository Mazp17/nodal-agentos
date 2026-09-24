use rusqlite::Connection;

use super::is_constraint;
use crate::db::rows::{insert_relation, relations_of};
use crate::db::DbError;
use crate::domain::{RelationKind, TaskRelation};

pub fn list(conn: &Connection, task_id: &str) -> Result<Vec<TaskRelation>, DbError> {
    relations_of(conn, task_id)
}

/// Idempotente: agregar una relación que ya existe no falla.
pub fn add(conn: &Connection, r: &TaskRelation) -> Result<(), DbError> {
    if r.task_id == r.other_id {
        return Err(DbError::Invalid("A task can't be related to itself.".into()));
    }
    match insert_relation(conn, r) {
        Ok(()) => Ok(()),
        Err(DbError::Sqlite(e)) if is_constraint(&e) => {
            // PK repetida (ya existe) o FK (alguna tarea no existe).
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE id IN (?1, ?2)",
                rusqlite::params![r.task_id, r.other_id],
                |row| row.get(0),
            )?;
            if exists == 2 {
                Ok(())
            } else {
                Err(DbError::Invalid("That task no longer exists.".into()))
            }
        }
        Err(e) => Err(e),
    }
}

pub fn remove(conn: &Connection, r: &TaskRelation) -> Result<(), DbError> {
    let (a, b) = match r.kind {
        RelationKind::Related if r.task_id > r.other_id => (&r.other_id, &r.task_id),
        _ => (&r.task_id, &r.other_id),
    };
    conn.execute(
        "DELETE FROM task_relations WHERE task_id = ?1 AND other_id = ?2 AND kind = ?3",
        rusqlite::params![a, b, r.kind],
    )?;
    Ok(())
}
