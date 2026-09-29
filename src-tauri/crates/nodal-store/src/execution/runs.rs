use rusqlite::{named_params, OptionalExtension};

use crate::error::not_found;
use crate::json::{opt_json, to_json};
use crate::rows::{get_run, insert_run, run_from_row, run_light_from_row, SqlEnum, RUN_LIGHT_COLUMNS};
use crate::Conn as Connection;
use crate::{DbError, SqliteError};
use nodal_domain::model::{Run, RunLight, RunStatus};

const HISTORY_LIMIT: i64 = 500;
const PENDING_STATUSES: &str = "('queued', 'launching', 'launched')";

fn query(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Run>, DbError> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params, run_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn query_light(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<Vec<RunLight>, DbError> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params, run_light_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Run, DbError> {
    get_run(conn, id)?.ok_or_else(|| not_found("run"))
}

/// Project filter, once `project_id` is known to be `Some`: the task's project, or the
/// repo's if the run has no task. `list_filtered`/`list_filtered_light`/`pending_of` each
/// branch into a query with this clause or one without it, rather than a single
/// `?1 IS NULL OR ...` filter: SQLite can't resolve at prepare time whether a bound
/// parameter is NULL, so that pattern plans as a full scan even when the value is bound.
const IN_PROJECT: &str = "(t.project_id = ?1 OR (r.task_id IS NULL AND rp.project_id = ?1))";
const PROJECT_JOIN: &str = "LEFT JOIN tasks t ON t.id = r.task_id LEFT JOIN repos rp ON rp.id = r.repo_id";

/// Runs of the project and/or task (`None`: unfiltered), most recent first. Up to
/// `HISTORY_LIMIT`.
pub fn list_filtered(conn: &Connection, project_id: Option<&str>, task_id: Option<&str>) -> Result<Vec<Run>, DbError> {
    match (project_id, task_id) {
        (Some(p), Some(t)) => query(
            conn,
            &format!(
                "SELECT r.* FROM runs r {PROJECT_JOIN}
                 WHERE {IN_PROJECT} AND r.task_id = ?2
                 ORDER BY r.queued_at DESC, r.id DESC LIMIT ?3"
            ),
            rusqlite::params![p, t, HISTORY_LIMIT],
        ),
        (Some(p), None) => query(
            conn,
            &format!(
                "SELECT r.* FROM runs r {PROJECT_JOIN}
                 WHERE {IN_PROJECT}
                 ORDER BY r.queued_at DESC, r.id DESC LIMIT ?2"
            ),
            rusqlite::params![p, HISTORY_LIMIT],
        ),
        (None, Some(t)) => query(
            conn,
            "SELECT * FROM runs WHERE task_id = ?1 ORDER BY queued_at DESC, id DESC LIMIT ?2",
            rusqlite::params![t, HISTORY_LIMIT],
        ),
        (None, None) => query(conn, "SELECT * FROM runs ORDER BY queued_at DESC, id DESC LIMIT ?1", rusqlite::params![HISTORY_LIMIT]),
    }
}

/// Like `list_filtered`, without `prompt`/`extraInstructions`: selects `RUN_LIGHT_COLUMNS`
/// instead of `r.*`, so the prompt blob is never read off disk for lists that only ever show
/// `RunLight` (history, board).
pub fn list_filtered_light(conn: &Connection, project_id: Option<&str>, task_id: Option<&str>) -> Result<Vec<RunLight>, DbError> {
    match (project_id, task_id) {
        (Some(p), Some(t)) => query_light(
            conn,
            &format!(
                "SELECT {RUN_LIGHT_COLUMNS} FROM runs r {PROJECT_JOIN}
                 WHERE {IN_PROJECT} AND r.task_id = ?2
                 ORDER BY r.queued_at DESC, r.id DESC LIMIT ?3"
            ),
            rusqlite::params![p, t, HISTORY_LIMIT],
        ),
        (Some(p), None) => query_light(
            conn,
            &format!(
                "SELECT {RUN_LIGHT_COLUMNS} FROM runs r {PROJECT_JOIN}
                 WHERE {IN_PROJECT}
                 ORDER BY r.queued_at DESC, r.id DESC LIMIT ?2"
            ),
            rusqlite::params![p, HISTORY_LIMIT],
        ),
        (None, Some(t)) => query_light(
            conn,
            &format!("SELECT {RUN_LIGHT_COLUMNS} FROM runs r WHERE r.task_id = ?1 ORDER BY r.queued_at DESC, r.id DESC LIMIT ?2"),
            rusqlite::params![t, HISTORY_LIMIT],
        ),
        (None, None) => query_light(
            conn,
            &format!("SELECT {RUN_LIGHT_COLUMNS} FROM runs r ORDER BY r.queued_at DESC, r.id DESC LIMIT ?1"),
            rusqlite::params![HISTORY_LIMIT],
        ),
    }
}

/// The latest run (by `queued_at`) of each of the project's tasks (`None`: all), with no
/// history limit.
pub fn latest_by_task(conn: &Connection, project_id: Option<&str>) -> Result<Vec<Run>, DbError> {
    match project_id {
        Some(p) => query(conn, LATEST_BY_TASK_PROJECT, [p]),
        None => query(conn, LATEST_BY_TASK_ALL, []),
    }
}

/// Like `latest_by_task`, without `prompt`/`extraInstructions` (see `list_filtered_light`).
pub fn latest_by_task_light(conn: &Connection, project_id: Option<&str>) -> Result<Vec<RunLight>, DbError> {
    match project_id {
        Some(p) => query_light(conn, &latest_by_task_sql(true), [p]),
        None => query_light(conn, &latest_by_task_sql(false), []),
    }
}

const LATEST_BY_TASK_PROJECT: &str = "SELECT * FROM (
    SELECT r.*, ROW_NUMBER() OVER (PARTITION BY r.task_id ORDER BY r.queued_at DESC, r.id DESC) AS rn
    FROM runs r JOIN tasks t ON t.id = r.task_id
    WHERE t.project_id = ?1
) WHERE rn = 1 ORDER BY queued_at DESC, id DESC";

const LATEST_BY_TASK_ALL: &str = "SELECT * FROM (
    SELECT r.*, ROW_NUMBER() OVER (PARTITION BY r.task_id ORDER BY r.queued_at DESC, r.id DESC) AS rn
    FROM runs r JOIN tasks t ON t.id = r.task_id
) WHERE rn = 1 ORDER BY queued_at DESC, id DESC";

/// `latest_by_task_light`'s query text, with `RUN_LIGHT_COLUMNS` in place of `r.*`.
fn latest_by_task_sql(filtered: bool) -> String {
    let filter = if filtered { "WHERE t.project_id = ?1" } else { "" };
    format!(
        "SELECT * FROM (
             SELECT {RUN_LIGHT_COLUMNS}, ROW_NUMBER() OVER (PARTITION BY r.task_id ORDER BY r.queued_at DESC, r.id DESC) AS rn
             FROM runs r JOIN tasks t ON t.id = r.task_id
             {filter}
         ) WHERE rn = 1 ORDER BY queued_at DESC, id DESC"
    )
}

/// Like `pending`, for the project (`None`: all).
pub fn pending_of(conn: &Connection, project_id: Option<&str>) -> Result<Vec<Run>, DbError> {
    let Some(p) = project_id else { return pending(conn) };
    query(
        conn,
        &format!(
            "SELECT r.* FROM runs r {PROJECT_JOIN}
             WHERE {IN_PROJECT} AND r.status IN {PENDING_STATUSES}
             ORDER BY r.queue_position, r.queued_at, r.id"
        ),
        [p],
    )
}

/// Global queue in dispatch order.
pub fn queue(conn: &Connection) -> Result<Vec<Run>, DbError> {
    query(conn, "SELECT * FROM runs WHERE status = 'queued' ORDER BY queue_position, queued_at, id", [])
}

/// Everything the queue tracks: queued, launching or launched (until it finishes).
pub fn pending(conn: &Connection) -> Result<Vec<Run>, DbError> {
    query(
        conn,
        &format!("SELECT * FROM runs WHERE status IN {PENDING_STATUSES} ORDER BY queue_position, queued_at, id"),
        [],
    )
}

pub fn pending_for_task(conn: &Connection, task_id: &str) -> Result<Vec<Run>, DbError> {
    query(
        conn,
        &format!("SELECT * FROM runs WHERE task_id = ?1 AND status IN {PENDING_STATUSES} ORDER BY queued_at"),
        [task_id],
    )
}

/// Pending runs of a project's tasks.
pub fn pending_in_project(conn: &Connection, project_id: &str) -> Result<i64, DbError> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT COUNT(*) FROM runs r JOIN tasks t ON t.id = r.task_id
         WHERE t.project_id = ?1 AND r.status IN {PENDING_STATUSES}"
    ))?;
    Ok(stmt.query_row([project_id], |r| r.get(0))?)
}

/// The task's last finished run (the previous step in the chain), including those stopped
/// by the user (canceled after launching).
pub fn last_finished(conn: &Connection, task_id: &str) -> Result<Option<Run>, DbError> {
    let mut stmt = conn.prepare_cached(
        "SELECT * FROM runs WHERE task_id = ?1
           AND (status = 'finished' OR (status = 'canceled' AND launched_at IS NOT NULL))
         ORDER BY COALESCE(finished_at, queued_at) DESC, queued_at DESC LIMIT 1",
    )?;
    Ok(stmt.query_row([task_id], run_from_row).optional()?)
}

/// The task's last work run, in any status.
pub fn last_work(conn: &Connection, task_id: &str) -> Result<Option<Run>, DbError> {
    let mut stmt = conn.prepare_cached("SELECT * FROM runs WHERE task_id = ?1 AND kind = 'work' ORDER BY queued_at DESC, id DESC LIMIT 1")?;
    Ok(stmt.query_row([task_id], run_from_row).optional()?)
}

pub fn next_queue_position(conn: &Connection) -> Result<f64, DbError> {
    let mut stmt = conn.prepare_cached("SELECT COALESCE(MAX(queue_position), 0) + 1 FROM runs")?;
    Ok(stmt.query_row([], |r| r.get(0))?)
}

pub fn insert(conn: &Connection, r: &Run) -> Result<(), DbError> {
    insert_run(conn, r)
}

/// Saves the run's mutable state (everything except identity, task, prompt and options).
pub fn update(conn: &Connection, r: &Run) -> Result<(), DbError> {
    let mut stmt = conn.prepare_cached(
        "UPDATE runs SET cwd = :cwd, status = :status, queue_position = :qpos, verdict_json = :verdict,
                         claude_run_id = :claude_id, session_id = :session, launched_at = :launched,
                         finished_at = :finished, outcome = :outcome, summary = :summary, pr_url = :pr,
                         branch = :branch, error = :error, legacy_label = :legacy, prompt = :prompt,
                         options_json = :options, tokens = :tokens
         WHERE id = :id",
    )?;
    let n = stmt.execute(named_params! {
        ":id": r.id, ":cwd": r.cwd, ":status": SqlEnum(r.status), ":qpos": r.queue_position,
        ":verdict": opt_json(&r.verdict)?, ":claude_id": r.claude_run_id, ":session": r.session_id,
        ":launched": r.launched_at, ":finished": r.finished_at, ":outcome": r.outcome.map(SqlEnum),
        ":summary": r.summary, ":pr": r.pr_url, ":branch": r.branch, ":error": r.error,
        ":legacy": r.legacy_label, ":prompt": r.prompt, ":options": to_json(&r.options)?, ":tokens": r.tokens,
    })?;
    if n == 0 {
        return Err(not_found("run"));
    }
    Ok(())
}

/// Moves from `from` to `to` only if still in `from` (compare-and-set). Returns whether it changed.
pub fn transition(conn: &Connection, id: &str, from: RunStatus, to: RunStatus) -> Result<bool, DbError> {
    let mut stmt = conn.prepare_cached("UPDATE runs SET status = ?3 WHERE id = ?1 AND status = ?2")?;
    Ok(stmt.execute(rusqlite::params![id, SqlEnum(from), SqlEnum(to)])? > 0)
}

/// `launching` runs. Only the queue pass (holding its turn) sets them so and clears them in
/// the same pass: outside of it, one left `launching` is orphaned.
pub fn launching(conn: &Connection) -> Result<Vec<Run>, DbError> {
    query(conn, "SELECT * FROM runs WHERE status = 'launching' ORDER BY queued_at, id", [])
}

/// New queue order: `ids` must be exactly the set of queued runs.
pub fn reorder_queue(conn: &mut Connection, ids: &[String]) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    let current: Vec<String> = {
        let mut stmt = tx.prepare("SELECT id FROM runs WHERE status = 'queued'")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut want: Vec<&String> = ids.iter().collect();
    want.sort();
    want.dedup();
    let mut have: Vec<&String> = current.iter().collect();
    have.sort();
    if want.len() != ids.len() || want != have {
        return Err(DbError::Invalid("The queue changed; reload it and try again.".into()));
    }
    // New positions start after all existing ones: the relative order against already
    // launched runs doesn't matter, but this way there are never ties.
    let base: f64 = tx.query_row("SELECT COALESCE(MAX(queue_position), 0) FROM runs", [], |r| r.get(0))?;
    for (i, id) in ids.iter().enumerate() {
        tx.execute("UPDATE runs SET queue_position = ?2 WHERE id = ?1", rusqlite::params![id, base + 1.0 + i as f64])?;
    }
    tx.commit()?;
    Ok(())
}

/// `(claude_run_id, session_id)` of a launched run.
pub type LaunchedRef = (Option<String>, Option<String>);

/// Refs of every launched run (to mark "launched by the app" in the activity).
pub fn launched_refs(conn: &Connection) -> Result<Vec<LaunchedRef>, DbError> {
    let mut stmt = conn.prepare_cached("SELECT claude_run_id, session_id FROM runs WHERE claude_run_id IS NOT NULL OR session_id IS NOT NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Sets `session_id` on a run that doesn't have one yet (the queue pass fills it in once
/// `claude agents` reports it). No-op if the run already has one.
pub fn set_session_id_if_null(conn: &Connection, id: &str, session_id: Option<&str>) -> Result<usize, SqliteError> {
    let mut stmt = conn.prepare_cached("UPDATE runs SET session_id = ?2 WHERE id = ?1 AND session_id IS NULL")?;
    Ok(stmt.execute(rusqlite::params![id, session_id])?)
}

/// Sets `finished_at` on a run being cancelled while queued.
pub fn set_finished_at(conn: &Connection, id: &str, now: i64) -> Result<usize, SqliteError> {
    let mut stmt = conn.prepare_cached("UPDATE runs SET finished_at = ?2 WHERE id = ?1")?;
    Ok(stmt.execute(rusqlite::params![id, now])?)
}
