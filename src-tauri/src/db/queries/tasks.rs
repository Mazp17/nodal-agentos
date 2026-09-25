use rusqlite::{named_params, Connection};

use super::{not_found, opt_json, to_json};
use crate::db::rows::{get_task, insert_task, task_from_row};
use crate::db::DbError;
use crate::domain::{PlanRef, Task, TaskStatus};

/// `None`: all of them. Order: project, status, position.
pub fn list(conn: &Connection, project_id: Option<&str>) -> Result<Vec<Task>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM tasks WHERE ?1 IS NULL OR project_id = ?1 ORDER BY project_id, status, position, number",
    )?;
    let rows = stmt.query_map([project_id], task_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Task, DbError> {
    get_task(conn, id)?.ok_or_else(|| not_found("task"))
}

/// A project's `{KEY}-{number}` task.
pub fn find_by_number(conn: &Connection, project_id: &str, number: i64) -> Result<Option<Task>, DbError> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row("SELECT * FROM tasks WHERE project_id = ?1 AND number = ?2", rusqlite::params![project_id, number], task_from_row)
        .optional()?)
}

pub fn insert(conn: &Connection, t: &Task) -> Result<(), DbError> {
    insert_task(conn, t)
}

/// Saves every Nodal field (doesn't touch `project_id`, `number`, `created_at` or the
/// `src_*` fields, which belong to the provider, except `link`/state, handled by F1-C).
pub fn update(conn: &Connection, t: &Task) -> Result<(), DbError> {
    let (plan_kind, plan_path) = match &t.plan {
        PlanRef::Text => ("text", None),
        PlanRef::File { path } => ("file", Some(path.as_str())),
    };
    let wt = t.worktree.as_ref();
    let n = conn.execute(
        "UPDATE tasks SET repo_id = :repo, title = :title, status = :status, priority = :priority,
                          labels_json = :labels, position = :position, plan_kind = :plan_kind,
                          plan_path = :plan_path, plan_overridden = :plan_overridden,
                          acceptance_json = :acceptance, assignee_json = :assignee, isolation = :isolation,
                          finish = :finish, review = :review, wt_path = :wt_path, wt_branch = :wt_branch,
                          wt_base = :wt_base, updated_at = :updated, closed_at = :closed
         WHERE id = :id",
        named_params! {
            ":id": t.id, ":repo": t.repo_id, ":title": t.title, ":status": t.status, ":priority": t.priority,
            ":labels": to_json(&t.labels)?, ":position": t.position, ":plan_kind": plan_kind,
            ":plan_path": plan_path, ":plan_overridden": t.plan_overridden,
            ":acceptance": to_json(&t.acceptance)?, ":assignee": opt_json(&t.assignee)?,
            ":isolation": t.isolation, ":finish": t.finish, ":review": t.review,
            ":wt_path": wt.map(|w| &w.path), ":wt_branch": wt.map(|w| &w.branch), ":wt_base": wt.map(|w| &w.base),
            ":updated": t.updated_at, ":closed": t.closed_at,
        },
    )?;
    if n == 0 {
        return Err(not_found("task"));
    }
    Ok(())
}

/// Changes the status (and `closed_at` when it moves into or out of Done/Canceled).
pub fn set_status(conn: &Connection, id: &str, status: TaskStatus, now: i64) -> Result<(), DbError> {
    let closed = matches!(status, TaskStatus::Done | TaskStatus::Canceled);
    let n = conn.execute(
        "UPDATE tasks SET status = ?2, updated_at = ?3,
                          closed_at = CASE WHEN ?4 THEN COALESCE(closed_at, ?3) ELSE NULL END
         WHERE id = ?1",
        rusqlite::params![id, status, now, closed],
    )?;
    if n == 0 {
        return Err(not_found("task"));
    }
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> Result<(), DbError> {
    if conn.execute("DELETE FROM tasks WHERE id = ?1", [id])? == 0 {
        return Err(not_found("task"));
    }
    Ok(())
}

/// Position at the end of the project's `status` column.
pub fn next_position(conn: &Connection, project_id: &str, status: TaskStatus) -> Result<f64, DbError> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM tasks WHERE project_id = ?1 AND status = ?2",
        rusqlite::params![project_id, status],
        |r| r.get(0),
    )?)
}

/// Ids of the project's blocked tasks (`None`: all projects).
pub fn blocked_ids(conn: &Connection, project_id: Option<&str>) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare("SELECT id FROM tasks WHERE status = 'blocked' AND (?1 IS NULL OR project_id = ?1)")?;
    let rows = stmt.query_map([project_id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
