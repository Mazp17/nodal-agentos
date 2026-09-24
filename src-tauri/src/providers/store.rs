//! SQL de proveedores: source links, tareas vinculadas, outbox y consulta de runs activos.
//! Vive acá (y no en `db/queries`) para no pisar el CRUD de F1-B; usa `db::rows` para las
//! conversiones fila ↔ struct.

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::rows::{self, outbox_from_row, source_link_from_row, task_from_row};
use crate::db::DbError;
use crate::domain::{
    ExternalState, OutboxItem, OutboxPayload, PlanRef, SourceLink, StateChanges, Task, TaskSource, TaskStatus,
};

use super::ExternalItem;

fn json<T: serde::Serialize>(v: &T) -> Result<String, DbError> {
    serde_json::to_string(v).map_err(|e| DbError::Invalid(format!("Could not serialize: {e}")))
}

// ---------- Source links ----------

pub fn get_link(conn: &Connection, id: &str) -> Result<SourceLink, DbError> {
    rows::get_source_link(conn, id)?.ok_or_else(|| DbError::Invalid("Source not found.".into()))
}

pub fn list_links(conn: &Connection, project_id: Option<&str>) -> Result<Vec<SourceLink>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM source_links WHERE ?1 IS NULL OR project_id = ?1 ORDER BY project_id, created_at, id",
    )?;
    let out = stmt.query_map([project_id], source_link_from_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(out)
}

pub fn insert_link(conn: &Connection, link: &SourceLink) -> Result<(), DbError> {
    rows::insert_source_link(conn, link).map_err(|e| match e {
        DbError::Sqlite(rusqlite::Error::SqliteFailure(f, _)) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            DbError::Invalid("This project is already linked to that scope.".into())
        }
        other => other,
    })
}

/// Guarda los campos editables (repo por defecto, reglas, mapeo, auto-import).
pub fn save_link(conn: &Connection, link: &SourceLink) -> Result<(), DbError> {
    conn.execute(
        "UPDATE source_links SET default_repo_id = ?2, repo_rules_json = ?3, state_map_json = ?4, auto_import = ?5
         WHERE id = ?1",
        params![link.id, link.default_repo_id, json(&link.repo_rules)?, json(&link.state_map)?, link.auto_import],
    )?;
    Ok(())
}

/// Guarda el mapeo y limpia `pending_state_changes` (el usuario ya revisó los estados).
pub fn save_state_map(conn: &Connection, link_id: &str, map: &crate::domain::StateMap) -> Result<(), DbError> {
    let n = conn.execute(
        "UPDATE source_links SET state_map_json = ?2, pending_state_changes = NULL WHERE id = ?1",
        params![link_id, json(map)?],
    )?;
    if n == 0 {
        return Err(DbError::Invalid("Source not found.".into()));
    }
    Ok(())
}

/// Resultado de la pasada del sync sobre un link (`error: None` = fue bien).
pub fn set_link_sync(conn: &Connection, link_id: &str, now: i64, error: Option<&str>) -> Result<(), DbError> {
    conn.execute(
        "UPDATE source_links SET last_synced_at = ?2, last_sync_error = ?3 WHERE id = ?1",
        params![link_id, now, error],
    )?;
    Ok(())
}

/// Altas/bajas de estados contra `known_states`; vacías → `NULL`.
pub fn set_pending_changes(conn: &Connection, link_id: &str, changes: &StateChanges) -> Result<(), DbError> {
    let v = if changes.is_empty() { None } else { Some(json(changes)?) };
    conn.execute("UPDATE source_links SET pending_state_changes = ?2 WHERE id = ?1", params![link_id, v])?;
    Ok(())
}

/// Proyecto al que pertenece un repo.
pub fn repo_project(conn: &Connection, repo_id: &str) -> Result<Option<String>, DbError> {
    Ok(conn.query_row("SELECT project_id FROM repos WHERE id = ?1", [repo_id], |r| r.get(0)).optional()?)
}

pub fn project_exists(conn: &Connection, id: &str) -> Result<bool, DbError> {
    Ok(conn.query_row("SELECT 1 FROM projects WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
}

/// Rechaza un repo que no sea del proyecto.
pub fn check_repo_in_project(conn: &Connection, repo_id: &str, project_id: &str) -> Result<(), DbError> {
    match repo_project(conn, repo_id)? {
        Some(p) if p == project_id => Ok(()),
        Some(_) => Err(DbError::Invalid("That repo belongs to another project.".into())),
        None => Err(DbError::Invalid("Repo not found.".into())),
    }
}

// ---------- Tareas vinculadas ----------

pub fn task_by_external(conn: &Connection, provider: &str, external_id: &str) -> Result<Option<Task>, DbError> {
    Ok(conn
        .query_row(
            "SELECT * FROM tasks WHERE src_provider = ?1 AND src_external_id = ?2",
            [provider, external_id],
            task_from_row,
        )
        .optional()?)
}

/// Las tareas cerradas hace más que esto dejan de refrescarse en el pull (volumen de API).
pub const PULL_CLOSED_WINDOW_MS: i64 = 7 * 24 * 3_600_000;

/// Tareas importadas por un link (o por cualquier link con `None`) que el pull refresca:
/// abiertas, o cerradas hace menos de `PULL_CLOSED_WINDOW_MS`.
pub fn linked_tasks(conn: &Connection, link_id: Option<&str>, now: i64) -> Result<Vec<Task>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM tasks WHERE src_link_id IS NOT NULL AND (?1 IS NULL OR src_link_id = ?1)
           AND (closed_at IS NULL OR closed_at > ?2)
         ORDER BY id",
    )?;
    let out = stmt
        .query_map(params![link_id, now - PULL_CLOSED_WINDOW_MS], task_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(out)
}

/// "Lápida" de un ítem desvinculado: el auto-import no lo vuelve a traer. Va en `settings`
/// (clave con prefijo; `load_settings` ignora las claves que no conoce) para no tocar el
/// esquema. TODO(F1 integración): tabla propia si `settings` pasa a listarse entero.
const UNLINKED_PREFIX: &str = "providers.unlinked:";

fn unlinked_key(provider: &str, external_id: &str) -> String {
    format!("{UNLINKED_PREFIX}{provider}:{external_id}")
}

pub fn is_unlinked(conn: &Connection, provider: &str, external_id: &str) -> Result<bool, DbError> {
    Ok(conn
        .query_row("SELECT 1 FROM settings WHERE key = ?1", [unlinked_key(provider, external_id)], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Un run que todavía no terminó (en cola, lanzándose o corriendo).
pub fn has_active_run(conn: &Connection, task_id: &str) -> Result<bool, DbError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM runs WHERE task_id = ?1 AND status IN ('queued', 'launching', 'launched') LIMIT 1",
            [task_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Datos de una tarea nueva importada.
pub struct NewImported<'a> {
    pub id: String,
    pub project_id: &'a str,
    pub repo_id: &'a str,
    pub link: &'a SourceLink,
    pub item: &'a ExternalItem,
    pub status: TaskStatus,
    pub acceptance: Vec<String>,
    pub now: i64,
}

/// Inserta la tarea asignando número (contador del proyecto) y posición al final de su
/// columna. Llamar dentro de una transacción.
pub fn insert_imported(conn: &Connection, n: NewImported) -> Result<Task, DbError> {
    let number: i64 = conn
        .query_row(
            "UPDATE projects SET next_task_number = next_task_number + 1 WHERE id = ?1
             RETURNING next_task_number - 1",
            [n.project_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| DbError::Invalid("Project not found.".into()))?;
    let position: f64 = conn.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM tasks WHERE project_id = ?1 AND status = ?2",
        params![n.project_id, n.status],
        |r| r.get(0),
    )?;
    let closed = matches!(n.status, TaskStatus::Done | TaskStatus::Canceled).then_some(n.now);
    let task = Task {
        id: n.id,
        project_id: n.project_id.into(),
        repo_id: n.repo_id.into(),
        number,
        title: n.item.title.clone(),
        status: n.status,
        priority: n.item.priority,
        labels: n.item.labels.clone(),
        position,
        plan: PlanRef::Text,
        plan_overridden: false,
        acceptance: n.acceptance,
        assignee: None,
        isolation: None,
        finish: None,
        review: None,
        worktree: None,
        source: Some(TaskSource {
            provider: n.link.provider.clone(),
            link_id: Some(n.link.id.clone()),
            external_id: n.item.external_id.clone(),
            identifier: n.item.identifier.clone(),
            url: n.item.url.clone(),
            external_state: Some(n.item.state.clone()),
            last_synced_at: Some(n.now),
            sync_error: None,
            unmapped: false,
        }),
        created_at: n.now,
        updated_at: n.now,
        closed_at: closed,
    };
    rows::insert_task(conn, &task)?;
    Ok(task)
}

/// Resultado del pull de una tarea (ver `sync::decide_pull`).
pub struct PullUpdate<'a> {
    pub task_id: &'a str,
    pub title: Option<&'a str>,
    pub status: Option<TaskStatus>,
    /// `None`: no tocar el estado externo guardado.
    pub external_state: Option<&'a ExternalState>,
    pub sync_error: Option<&'a str>,
    /// Sin error propio del pull, conservar el que haya (lo dejó el push de esta pasada).
    pub keep_error: bool,
    /// El estado externo no está en el mapeo pull.
    pub unmapped: bool,
    pub now: i64,
}

pub fn apply_pull(conn: &Connection, u: &PullUpdate) -> Result<(), DbError> {
    let changed = u.title.is_some() || u.status.is_some();
    conn.execute(
        "UPDATE tasks SET
           title = COALESCE(?2, title),
           status = COALESCE(?3, status),
           closed_at = CASE WHEN ?3 IS NULL THEN closed_at
                            WHEN ?3 IN ('done', 'canceled') THEN ?6 ELSE NULL END,
           src_state_json = COALESCE(?4, src_state_json), src_last_synced_at = ?6,
           src_sync_error = CASE WHEN ?5 IS NULL AND ?8 THEN src_sync_error ELSE ?5 END,
           src_unmapped = ?9,
           updated_at = CASE WHEN ?7 THEN ?6 ELSE updated_at END
         WHERE id = ?1 AND src_provider IS NOT NULL",
        params![
            u.task_id,
            u.title,
            u.status,
            u.external_state.map(json).transpose()?,
            u.sync_error,
            u.now,
            changed,
            u.keep_error,
            u.unmapped
        ],
    )?;
    Ok(())
}

pub fn set_sync_error(conn: &Connection, task_id: &str, error: Option<&str>) -> Result<(), DbError> {
    // Una tarea desvinculada mientras el sync iba a la red no recibe `src_*` de nuevo.
    conn.execute(
        "UPDATE tasks SET src_sync_error = ?2 WHERE id = ?1 AND src_provider IS NOT NULL",
        params![task_id, error],
    )?;
    Ok(())
}

/// Tras un push de estado exitoso: el estado externo guardado pasa a ser el empujado, así el
/// próximo pull no lo ve como un cambio.
pub fn set_external_state(conn: &Connection, task_id: &str, state: &ExternalState, now: i64) -> Result<(), DbError> {
    conn.execute(
        "UPDATE tasks SET src_state_json = ?2, src_last_synced_at = ?3, src_sync_error = NULL, src_unmapped = 0
         WHERE id = ?1 AND src_provider IS NOT NULL",
        params![task_id, json(state)?, now],
    )?;
    Ok(())
}

/// Unlink: la tarea queda local (se limpian todos los `src_*` y su outbox).
pub fn unlink_task(conn: &Connection, task_id: &str, now: i64) -> Result<Task, DbError> {
    let src: Option<(Option<String>, Option<String>)> = conn
        .query_row("SELECT src_provider, src_external_id FROM tasks WHERE id = ?1", [task_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    if let Some((Some(provider), Some(ext))) = src {
        conn.execute(
            "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            params![unlinked_key(&provider, &ext), now.to_string()],
        )?;
    }
    conn.execute("DELETE FROM sync_outbox WHERE task_id = ?1", [task_id])?;
    let n = conn.execute(
        "UPDATE tasks SET src_provider = NULL, src_link_id = NULL, src_external_id = NULL, src_identifier = NULL,
                          src_url = NULL, src_state_json = NULL, src_last_synced_at = NULL, src_sync_error = NULL,
                          updated_at = ?2
         WHERE id = ?1",
        params![task_id, now],
    )?;
    if n == 0 {
        return Err(DbError::Invalid("Task not found.".into()));
    }
    rows::get_task(conn, task_id)?.ok_or_else(|| DbError::Invalid("Task not found.".into()))
}

/// Disconnect: desvincula las tareas del link (quedan locales) y lo borra, en una transacción.
pub fn disconnect_link(conn: &mut Connection, link_id: &str, now: i64) -> Result<usize, DbError> {
    let tx = conn.transaction()?;
    let ids: Vec<String> = {
        let mut stmt = tx.prepare("SELECT id FROM tasks WHERE src_link_id = ?1")?;
        let ids = stmt.query_map([link_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        ids
    };
    for id in &ids {
        unlink_task(&tx, id, now)?;
    }
    let n = tx.execute("DELETE FROM source_links WHERE id = ?1", [link_id])?;
    if n == 0 {
        return Err(DbError::Invalid("Source not found.".into()));
    }
    tx.commit()?;
    Ok(ids.len())
}

// ---------- Outbox ----------
// `enqueue_*` los llama la cola (`work::ops::push_status`), en la transacción del cambio.

fn task_provider(conn: &Connection, task_id: &str) -> Result<Option<String>, DbError> {
    Ok(conn
        .query_row("SELECT src_provider FROM tasks WHERE id = ?1", [task_id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten())
}

/// Encola el push del estado Nodal `status` para una tarea importada. El destino externo se
/// resuelve al drenar, con el mapeo vigente (así un cambio de mapeo o un mapeo pendiente se
/// respetan). Reemplaza cualquier push de estado anterior todavía pendiente: solo importa el
/// último. No hace nada con tareas locales. Devuelve si encoló.
///
/// Contrato del payload: `SetState.state_id` lleva el `TaskStatus` (`"in_review"`); un valor
/// que no sea un `TaskStatus` se toma como id de estado externo literal.
pub fn enqueue_status(conn: &Connection, task_id: &str, status: TaskStatus, now: i64) -> Result<bool, DbError> {
    let Some(provider) = task_provider(conn, task_id)? else { return Ok(false) };
    conn.execute("DELETE FROM sync_outbox WHERE task_id = ?1 AND kind = 'set_state'", [task_id])?;
    let payload = OutboxPayload::SetState { state_id: status.as_str().to_string() };
    insert(conn, task_id, &provider, payload, now)?;
    Ok(true)
}

/// Encola un comentario para una tarea importada (se manda aunque el mapeo esté pendiente).
pub fn enqueue_comment(conn: &Connection, task_id: &str, body: &str, now: i64) -> Result<bool, DbError> {
    let Some(provider) = task_provider(conn, task_id)? else { return Ok(false) };
    insert(conn, task_id, &provider, OutboxPayload::Comment { body: body.to_string() }, now)?;
    Ok(true)
}

fn insert(conn: &Connection, task_id: &str, provider: &str, payload: OutboxPayload, now: i64) -> Result<i64, DbError> {
    rows::insert_outbox(
        conn,
        &OutboxItem {
            id: 0,
            task_id: task_id.into(),
            provider: provider.into(),
            payload,
            attempts: 0,
            next_attempt_at: now,
            last_error: None,
            created_at: now,
        },
    )
}

/// Filas vencidas de un proveedor, en orden de creación (el comentario de cierre sale después
/// del cambio de estado que lo acompaña). Una fila no sale mientras haya una anterior de su
/// misma tarea todavía en backoff: el orden por tarea se respeta entre pasadas.
pub fn due_outbox(conn: &Connection, provider: &str, now: i64) -> Result<Vec<OutboxItem>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM sync_outbox o WHERE provider = ?1 AND next_attempt_at <= ?2
           AND NOT EXISTS (SELECT 1 FROM sync_outbox p
                           WHERE p.task_id = o.task_id AND p.id < o.id AND p.next_attempt_at > ?2)
         ORDER BY id",
    )?;
    let out = stmt.query_map(params![provider, now], outbox_from_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(out)
}

pub fn outbox_done(conn: &Connection, id: i64) -> Result<(), DbError> {
    conn.execute("DELETE FROM sync_outbox WHERE id = ?1", [id])?;
    Ok(())
}

pub fn outbox_retry(conn: &Connection, id: i64, attempts: i64, next_at: i64, error: &str) -> Result<(), DbError> {
    conn.execute(
        "UPDATE sync_outbox SET attempts = ?2, next_attempt_at = ?3, last_error = ?4 WHERE id = ?1",
        params![id, attempts, next_at, error],
    )?;
    Ok(())
}
