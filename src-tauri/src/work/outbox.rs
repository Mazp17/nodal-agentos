//! Puente al outbox de proveedores (contrato de F1-C). Al integrar, estas funciones pasan a
//! llamar a `providers::enqueue_status` y `providers::enqueue_comment`; mientras tanto
//! escriben las filas con el mismo formato:
//! - `set_state` lleva el `TaskStatus` (`"in_review"`), no el id externo: el proveedor lo
//!   resuelve con el `state_map` al drenar (y ahí aplica "mapeo pendiente" y "No sincronizar");
//! - un push de estado nuevo reemplaza al pendiente de la misma tarea.
//!
//! Siempre dentro de la transacción del cambio de estado.

use rusqlite::{Connection, OptionalExtension};

use crate::db::rows::insert_outbox;
use crate::domain::{OutboxItem, OutboxPayload, TaskStatus};

fn provider_of(conn: &Connection, task_id: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT src_provider FROM tasks WHERE id = ?1", [task_id], |r| r.get::<_, Option<String>>(0))
        .optional()
        .map(Option::flatten)
        .map_err(|e| e.to_string())
}

fn insert(conn: &Connection, task_id: &str, provider: String, payload: OutboxPayload, now: i64) -> Result<(), String> {
    insert_outbox(
        conn,
        &OutboxItem {
            id: 0,
            task_id: task_id.to_string(),
            provider,
            payload,
            attempts: 0,
            next_attempt_at: now,
            last_error: None,
            created_at: now,
        },
    )?;
    Ok(())
}

/// Encola el push del estado Nodal de una tarea vinculada (no hace nada si es local).
pub fn enqueue_status(conn: &Connection, task_id: &str, status: TaskStatus, now: i64) -> Result<(), String> {
    let Some(provider) = provider_of(conn, task_id)? else { return Ok(()) };
    conn.execute("DELETE FROM sync_outbox WHERE task_id = ?1 AND kind = 'set_state'", [task_id])
        .map_err(|e| e.to_string())?;
    insert(conn, task_id, provider, OutboxPayload::SetState { state_id: status.as_str().to_string() }, now)
}

/// Encola un comentario en el ítem del proveedor (no hace nada si la tarea es local).
pub fn enqueue_comment(conn: &Connection, task_id: &str, body: &str, now: i64) -> Result<(), String> {
    let Some(provider) = provider_of(conn, task_id)? else { return Ok(()) };
    insert(conn, task_id, provider, OutboxPayload::Comment { body: body.to_string() }, now)
}
