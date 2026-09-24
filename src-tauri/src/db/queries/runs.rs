use rusqlite::{named_params, Connection, OptionalExtension};

use super::{not_found, opt_json, to_json};
use crate::db::rows::{get_run, insert_run, run_from_row};
use crate::db::DbError;
use crate::domain::{Run, RunStatus};

const HISTORY_LIMIT: i64 = 500;

fn query(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Run>, DbError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, run_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Run, DbError> {
    get_run(conn, id)?.ok_or_else(|| not_found("run"))
}

/// Runs de la tarea (o los últimos de todas con `None`), más recientes primero.
pub fn list(conn: &Connection, task_id: Option<&str>) -> Result<Vec<Run>, DbError> {
    query(
        conn,
        "SELECT * FROM runs WHERE ?1 IS NULL OR task_id = ?1 ORDER BY queued_at DESC, id DESC LIMIT ?2",
        rusqlite::params![task_id, HISTORY_LIMIT],
    )
}

/// Cola global en orden de salida.
pub fn queue(conn: &Connection) -> Result<Vec<Run>, DbError> {
    query(conn, "SELECT * FROM runs WHERE status = 'queued' ORDER BY queue_position, queued_at, id", [])
}

/// Todo lo que la cola sigue: en cola, lanzándose o lanzado (hasta que termine).
pub fn pending(conn: &Connection) -> Result<Vec<Run>, DbError> {
    query(
        conn,
        "SELECT * FROM runs WHERE status IN ('queued', 'launching', 'launched') ORDER BY queue_position, queued_at, id",
        [],
    )
}

pub fn pending_for_task(conn: &Connection, task_id: &str) -> Result<Vec<Run>, DbError> {
    query(
        conn,
        "SELECT * FROM runs WHERE task_id = ?1 AND status IN ('queued', 'launching', 'launched') ORDER BY queued_at",
        [task_id],
    )
}

/// Runs pendientes de las tareas de un proyecto.
pub fn pending_in_project(conn: &Connection, project_id: &str) -> Result<i64, DbError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM runs r JOIN tasks t ON t.id = r.task_id
         WHERE t.project_id = ?1 AND r.status IN ('queued', 'launching', 'launched')",
        [project_id],
        |r| r.get(0),
    )?)
}

/// Último run terminado de la tarea (el paso previo de la cadena), incluidos los detenidos
/// por el usuario (cancelados después de lanzarse).
pub fn last_finished(conn: &Connection, task_id: &str) -> Result<Option<Run>, DbError> {
    Ok(conn
        .query_row(
            "SELECT * FROM runs WHERE task_id = ?1
               AND (status = 'finished' OR (status = 'canceled' AND launched_at IS NOT NULL))
             ORDER BY COALESCE(finished_at, queued_at) DESC, queued_at DESC LIMIT 1",
            [task_id],
            run_from_row,
        )
        .optional()?)
}

/// Último run de trabajo de la tarea, en cualquier estado.
pub fn last_work(conn: &Connection, task_id: &str) -> Result<Option<Run>, DbError> {
    Ok(conn
        .query_row(
            "SELECT * FROM runs WHERE task_id = ?1 AND kind = 'work' ORDER BY queued_at DESC, id DESC LIMIT 1",
            [task_id],
            run_from_row,
        )
        .optional()?)
}

pub fn next_queue_position(conn: &Connection) -> Result<f64, DbError> {
    Ok(conn.query_row("SELECT COALESCE(MAX(queue_position), 0) + 1 FROM runs", [], |r| r.get(0))?)
}

pub fn insert(conn: &Connection, r: &Run) -> Result<(), DbError> {
    insert_run(conn, r)
}

/// Guarda el estado mutable del run (todo menos identidad, tarea, prompt y opciones).
pub fn update(conn: &Connection, r: &Run) -> Result<(), DbError> {
    let n = conn.execute(
        "UPDATE runs SET cwd = :cwd, status = :status, queue_position = :qpos, verdict_json = :verdict,
                         claude_run_id = :claude_id, session_id = :session, launched_at = :launched,
                         finished_at = :finished, outcome = :outcome, summary = :summary, pr_url = :pr,
                         branch = :branch, error = :error, legacy_label = :legacy, prompt = :prompt,
                         options_json = :options
         WHERE id = :id",
        named_params! {
            ":id": r.id, ":cwd": r.cwd, ":status": r.status, ":qpos": r.queue_position,
            ":verdict": opt_json(&r.verdict)?, ":claude_id": r.claude_run_id, ":session": r.session_id,
            ":launched": r.launched_at, ":finished": r.finished_at, ":outcome": r.outcome,
            ":summary": r.summary, ":pr": r.pr_url, ":branch": r.branch, ":error": r.error,
            ":legacy": r.legacy_label, ":prompt": r.prompt, ":options": to_json(&r.options)?,
        },
    )?;
    if n == 0 {
        return Err(not_found("run"));
    }
    Ok(())
}

/// Pasa de `from` a `to` solo si sigue en `from` (compare-and-set). Devuelve si cambió.
pub fn transition(conn: &Connection, id: &str, from: RunStatus, to: RunStatus) -> Result<bool, DbError> {
    Ok(conn.execute("UPDATE runs SET status = ?3 WHERE id = ?1 AND status = ?2", rusqlite::params![id, from, to])? > 0)
}

/// Al arrancar: un `launching` que quedó de una sesión anterior no se sabe si llegó a
/// lanzarse. Pasa a `failed` con explicación.
pub fn fail_interrupted_launches(conn: &Connection, now: i64) -> Result<usize, DbError> {
    Ok(conn.execute(
        "UPDATE runs SET status = 'failed', finished_at = ?1,
                         error = 'The app closed while the run was launching; check the runs list before retrying.'
         WHERE status = 'launching'",
        [now],
    )?)
}

/// Nuevo orden de la cola: `ids` tiene que ser exactamente el conjunto de runs en cola.
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
    // Las posiciones nuevas arrancan después de todas las existentes: el orden relativo
    // con los runs ya lanzados no importa, pero así nunca hay empates.
    let base: f64 = tx.query_row("SELECT COALESCE(MAX(queue_position), 0) FROM runs", [], |r| r.get(0))?;
    for (i, id) in ids.iter().enumerate() {
        tx.execute("UPDATE runs SET queue_position = ?2 WHERE id = ?1", rusqlite::params![id, base + 1.0 + i as f64])?;
    }
    tx.commit()?;
    Ok(())
}

/// `(claude_run_id, session_id)` de un run lanzado.
pub type LaunchedRef = (Option<String>, Option<String>);

/// Refs de todos los runs lanzados (para marcar "lanzado por la app" en la actividad).
pub fn launched_refs(conn: &Connection) -> Result<Vec<LaunchedRef>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT claude_run_id, session_id FROM runs WHERE claude_run_id IS NOT NULL OR session_id IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
