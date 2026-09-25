use rusqlite::{named_params, Connection, OptionalExtension};

use super::{not_found, opt_json};
use crate::db::rows::{get_repo, insert_repo, repo_from_row};
use crate::db::DbError;
use crate::domain::Repo;

/// `None`: all of them. Order: project, position.
pub fn list(conn: &Connection, project_id: Option<&str>) -> Result<Vec<Repo>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM repos WHERE ?1 IS NULL OR project_id = ?1 ORDER BY project_id, position, created_at, id",
    )?;
    let rows = stmt.query_map([project_id], repo_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Repo, DbError> {
    get_repo(conn, id)?.ok_or_else(|| not_found("repo"))
}

pub fn find_by_path(conn: &Connection, path: &str) -> Result<Option<Repo>, DbError> {
    Ok(conn.query_row("SELECT * FROM repos WHERE path = ?1", [path], repo_from_row).optional()?)
}

/// Rejects a path that already belongs to some project (naming that project).
pub fn insert(conn: &Connection, r: &Repo) -> Result<(), DbError> {
    if let Some(existing) = find_by_path(conn, &r.path)? {
        let project: String = conn
            .query_row("SELECT name FROM projects WHERE id = ?1", [&existing.project_id], |row| row.get(0))
            .optional()?
            .unwrap_or_else(|| "another project".into());
        return Err(DbError::Invalid(format!("{} is already added to {project}.", r.path)));
    }
    insert_repo(conn, r)
}

/// Saves everything except `project_id`, `path` and `created_at`.
pub fn update(conn: &Connection, r: &Repo) -> Result<(), DbError> {
    let n = conn.execute(
        "UPDATE repos SET name = :name, model = :model, effort = :effort, permission_mode = :perm,
                          default_executor_json = :exec, default_isolation = :iso, default_finish = :finish,
                          default_review = :review, reviewer = :reviewer, position = :pos
         WHERE id = :id",
        named_params! {
            ":id": r.id, ":name": r.name, ":model": r.launch.model, ":effort": r.launch.effort,
            ":perm": r.launch.permission_mode, ":exec": opt_json(&r.default_executor)?,
            ":iso": r.default_isolation, ":finish": r.default_finish, ":review": r.default_review,
            ":reviewer": r.reviewer, ":pos": r.position,
        },
    )?;
    if n == 0 {
        return Err(not_found("repo"));
    }
    Ok(())
}

pub fn task_count(conn: &Connection, id: &str) -> Result<i64, DbError> {
    Ok(conn.query_row("SELECT COUNT(*) FROM tasks WHERE repo_id = ?1", [id], |r| r.get(0))?)
}

pub fn delete(conn: &Connection, id: &str) -> Result<(), DbError> {
    let n = task_count(conn, id)?;
    if n > 0 {
        return Err(DbError::Invalid(format!(
            "The repo has {n} task{}: move or delete them first.",
            if n == 1 { "" } else { "s" }
        )));
    }
    if conn.execute("DELETE FROM repos WHERE id = ?1", [id])? == 0 {
        return Err(not_found("repo"));
    }
    Ok(())
}

pub fn next_position(conn: &Connection, project_id: &str) -> Result<i64, DbError> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM repos WHERE project_id = ?1",
        [project_id],
        |r| r.get(0),
    )?)
}
