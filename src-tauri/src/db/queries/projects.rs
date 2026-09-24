use rusqlite::{named_params, Connection};

use super::{is_constraint, not_found, opt_json};
use crate::db::rows::{get_project, insert_project, project_from_row};
use crate::db::DbError;
use crate::domain::Project;

pub fn list(conn: &Connection, include_archived: bool) -> Result<Vec<Project>, DbError> {
    let sql = if include_archived {
        "SELECT * FROM projects ORDER BY created_at, id"
    } else {
        "SELECT * FROM projects WHERE archived_at IS NULL ORDER BY created_at, id"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], project_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Project, DbError> {
    get_project(conn, id)?.ok_or_else(|| not_found("project"))
}

fn key_error(key: &str) -> DbError {
    DbError::Invalid(format!("The key \"{key}\" is already used by another project."))
}

pub fn key_taken(conn: &Connection, key: &str, except_id: Option<&str>) -> Result<bool, DbError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM projects WHERE key = ?1 AND id <> ?2",
        rusqlite::params![key, except_id.unwrap_or("")],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

pub fn insert(conn: &Connection, p: &Project) -> Result<(), DbError> {
    if key_taken(conn, &p.key, None)? {
        return Err(key_error(&p.key));
    }
    insert_project(conn, p)
}

/// Guarda todos los campos editables (no toca `next_task_number`).
pub fn update(conn: &Connection, p: &Project) -> Result<(), DbError> {
    if key_taken(conn, &p.key, Some(&p.id))? {
        return Err(key_error(&p.key));
    }
    let n = conn
        .execute(
            "UPDATE projects SET name = :name, key = :key, color = :color, default_executor_json = :exec,
                                 reviewer = :reviewer, archived_at = :archived
             WHERE id = :id",
            named_params! {
                ":id": p.id, ":name": p.name, ":key": p.key, ":color": p.color,
                ":exec": opt_json(&p.default_executor)?, ":reviewer": p.reviewer, ":archived": p.archived_at,
            },
        )
        .map_err(|e| if is_constraint(&e) { key_error(&p.key) } else { e.into() })?;
    if n == 0 {
        return Err(not_found("project"));
    }
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> Result<(), DbError> {
    if conn.execute("DELETE FROM projects WHERE id = ?1", [id])? == 0 {
        return Err(not_found("project"));
    }
    Ok(())
}

/// Toma el próximo número de tarea del proyecto y avanza el contador.
pub fn take_task_number(conn: &Connection, project_id: &str) -> Result<i64, DbError> {
    let n: Option<i64> = conn
        .query_row(
            "UPDATE projects SET next_task_number = next_task_number + 1 WHERE id = ?1
             RETURNING next_task_number - 1",
            [project_id],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })?;
    n.ok_or_else(|| not_found("project"))
}

pub fn count(conn: &Connection) -> Result<i64, DbError> {
    Ok(conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))?)
}
