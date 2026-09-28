use rusqlite::named_params;

use crate::error::not_found;
use crate::rows::{chat_from_row, get_chat, insert_chat};
use crate::Conn as Connection;
use crate::DbError;
use nodal_domain::model::Chat;

/// The project's chats, most recently used first.
pub fn list(conn: &Connection, project_id: &str) -> Result<Vec<Chat>, DbError> {
    let mut stmt = conn.prepare("SELECT * FROM chats WHERE project_id = ?1 ORDER BY updated_at DESC, id DESC")?;
    let rows = stmt.query_map([project_id], chat_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every project's chats, most recently used first (for the command palette).
pub fn list_all(conn: &Connection) -> Result<Vec<Chat>, DbError> {
    let mut stmt = conn.prepare("SELECT * FROM chats ORDER BY updated_at DESC, id DESC")?;
    let rows = stmt.query_map([], chat_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn get(conn: &Connection, id: &str) -> Result<Chat, DbError> {
    get_chat(conn, id)?.ok_or_else(|| not_found("chat"))
}

pub fn insert(conn: &Connection, c: &Chat) -> Result<(), DbError> {
    insert_chat(conn, c)
}

/// Saves everything except `project_id`, `session_id`, `session_title` and `created_at`.
pub fn update(conn: &Connection, c: &Chat) -> Result<(), DbError> {
    let n = conn.execute(
        "UPDATE chats SET repo_id = :repo, title = :title, model = :model, effort = :effort,
                          permission_mode = :perm, updated_at = :updated
         WHERE id = :id",
        named_params! {
            ":id": c.id, ":repo": c.repo_id, ":title": c.title, ":model": c.launch.model,
            ":effort": c.launch.effort, ":perm": c.launch.permission_mode, ":updated": c.updated_at,
        },
    )?;
    if n == 0 {
        return Err(not_found("chat"));
    }
    Ok(())
}

/// Stores the session id Claude Code reported; `false` if it was already that one.
pub fn set_session(conn: &Connection, id: &str, session_id: &str) -> Result<bool, DbError> {
    let n = conn.execute(
        "UPDATE chats SET session_id = ?2 WHERE id = ?1 AND session_id IS NOT ?2",
        [id, session_id],
    )?;
    Ok(n > 0)
}

/// Stores Claude Code's name for the session; `false` if it was already that one.
pub fn set_session_title(conn: &Connection, id: &str, title: &str) -> Result<bool, DbError> {
    let n = conn.execute(
        "UPDATE chats SET session_title = ?2 WHERE id = ?1 AND session_title IS NOT ?2",
        [id, title],
    )?;
    Ok(n > 0)
}

pub fn delete(conn: &Connection, id: &str) -> Result<(), DbError> {
    if conn.execute("DELETE FROM chats WHERE id = ?1", [id])? == 0 {
        return Err(not_found("chat"));
    }
    Ok(())
}
