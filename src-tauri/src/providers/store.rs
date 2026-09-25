//! Provider SQL: source links, linked tasks, outbox and the active-runs query.
//! Lives here (and not in `db/queries`) so it doesn't collide with F1-B's CRUD; uses `db::rows`
//! for row ↔ struct conversions.

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::rows::{self, outbox_from_row, source_link_from_row, task_from_row};
use crate::db::DbError;
use crate::domain::{
    ExtProject, ExternalState, MovedInfo, OutboxItem, OutboxPayload, PlanRef, SourceLink, StateChanges, Task,
    TaskSource, TaskStatus,
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

/// Saves the editable fields (default repo, rules, mapping, auto-import).
pub fn save_link(conn: &Connection, link: &SourceLink) -> Result<(), DbError> {
    conn.execute(
        "UPDATE source_links SET default_repo_id = ?2, repo_rules_json = ?3, state_map_json = ?4, auto_import = ?5
         WHERE id = ?1",
        params![link.id, link.default_repo_id, json(&link.repo_rules)?, json(&link.state_map)?, link.auto_import],
    )?;
    Ok(())
}

/// Saves the mapping and clears `pending_state_changes` (the user already reviewed the states).
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

/// Result of the sync pass over a link (`error: None` = it went fine).
pub fn set_link_sync(conn: &Connection, link_id: &str, now: i64, error: Option<&str>) -> Result<(), DbError> {
    conn.execute(
        "UPDATE source_links SET last_synced_at = ?2, last_sync_error = ?3 WHERE id = ?1",
        params![link_id, now, error],
    )?;
    Ok(())
}

/// State additions/removals against `known_states`; empty → `NULL`.
pub fn set_pending_changes(conn: &Connection, link_id: &str, changes: &StateChanges) -> Result<(), DbError> {
    let v = if changes.is_empty() { None } else { Some(json(changes)?) };
    conn.execute("UPDATE source_links SET pending_state_changes = ?2 WHERE id = ?1", params![link_id, v])?;
    Ok(())
}

/// Project a repo belongs to.
pub fn repo_project(conn: &Connection, repo_id: &str) -> Result<Option<String>, DbError> {
    Ok(conn.query_row("SELECT project_id FROM repos WHERE id = ?1", [repo_id], |r| r.get(0)).optional()?)
}

pub fn project_exists(conn: &Connection, id: &str) -> Result<bool, DbError> {
    Ok(conn.query_row("SELECT 1 FROM projects WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
}

/// Rejects a repo that isn't from the project.
pub fn check_repo_in_project(conn: &Connection, repo_id: &str, project_id: &str) -> Result<(), DbError> {
    match repo_project(conn, repo_id)? {
        Some(p) if p == project_id => Ok(()),
        Some(_) => Err(DbError::Invalid("That repo belongs to another project.".into())),
        None => Err(DbError::Invalid("Repo not found.".into())),
    }
}

// ---------- Linked tasks ----------

pub fn task_by_external(conn: &Connection, provider: &str, external_id: &str) -> Result<Option<Task>, DbError> {
    Ok(conn
        .query_row(
            "SELECT * FROM tasks WHERE src_provider = ?1 AND src_external_id = ?2",
            [provider, external_id],
            task_from_row,
        )
        .optional()?)
}

/// Tasks closed longer ago than this stop being refreshed on pull (API volume).
pub const PULL_CLOSED_WINDOW_MS: i64 = 7 * 24 * 3_600_000;

/// Tasks imported by a link (or by any link with `None`) that the pull refreshes: open, or
/// closed less than `PULL_CLOSED_WINDOW_MS` ago.
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

/// "Tombstone" of an unlinked item: auto-import doesn't bring it back. Stored in `settings`
/// (prefixed key; `load_settings` ignores keys it doesn't know) so the schema stays
/// untouched. TODO(F1 integration): own table if `settings` ever gets listed in full.
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

/// A run that hasn't finished yet (queued, launching or running).
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

/// Data for a new imported task.
pub struct NewImported<'a> {
    pub id: String,
    pub project_id: &'a str,
    pub repo_id: &'a str,
    pub link: &'a SourceLink,
    pub item: &'a ExternalItem,
    pub status: TaskStatus,
    pub acceptance: Vec<String>,
    /// Project rule it arrives through (see `TaskSource.rule_id`).
    pub rule_id: Option<String>,
    pub now: i64,
}

/// Inserts the task assigning a number (project counter) and a position at the end of its
/// column. Call inside a transaction.
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
            project: n.item.project(),
            rule_id: n.rule_id,
            moved: None,
        }),
        created_at: n.now,
        updated_at: n.now,
        closed_at: closed,
    };
    rows::insert_task(conn, &task)?;
    Ok(task)
}

/// Result of pulling a task (see `sync::decide_pull`).
pub struct PullUpdate<'a> {
    pub task_id: &'a str,
    pub title: Option<&'a str>,
    pub status: Option<TaskStatus>,
    /// `None`: don't touch the saved external state.
    pub external_state: Option<&'a ExternalState>,
    pub sync_error: Option<&'a str>,
    /// With no pull error of its own, keep whatever is there (left by this pass's push).
    pub keep_error: bool,
    /// The external state isn't in the pull mapping.
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

/// Provider project seen on pull, originating rule and pending project change (see
/// `sync::decide_project`). Writes all three as given.
pub fn set_src_project(
    conn: &Connection,
    task_id: &str,
    project: Option<&ExtProject>,
    rule_id: Option<&str>,
    moved: Option<&MovedInfo>,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE tasks SET src_project_id = ?2, src_project_name = ?3, src_rule_id = ?4, src_moved = ?5
         WHERE id = ?1 AND src_provider IS NOT NULL",
        params![
            task_id,
            project.map(|p| &p.id),
            project.map(|p| &p.name),
            rule_id,
            moved.map(json).transpose()?
        ],
    )?;
    Ok(())
}

/// A rule's backfill found these tasks already in its repo: they become "arrived through the
/// rule" (they warn if the issue changes project). Only those of that link with no pending warning.
pub fn tag_rule(conn: &Connection, link_id: &str, rule_id: &str, repo_id: &str, task_ids: &[String]) -> Result<(), DbError> {
    for id in task_ids {
        conn.execute(
            "UPDATE tasks SET src_rule_id = ?3
             WHERE id = ?1 AND src_link_id = ?2 AND repo_id = ?4 AND src_moved IS NULL",
            params![id, link_id, rule_id, repo_id],
        )?;
    }
    Ok(())
}

/// Tasks with an undecided project change (for the "need you" counter).
pub fn moved_ids(conn: &Connection, project_id: Option<&str>) -> Result<Vec<String>, DbError> {
    let mut stmt =
        conn.prepare("SELECT id FROM tasks WHERE src_moved IS NOT NULL AND (?1 IS NULL OR project_id = ?1)")?;
    let out = stmt.query_map([project_id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(out)
}

/// What to do with a task whose issue changed project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovedAction {
    /// Move it to the suggested repo.
    Move,
    /// Keep it in its repo.
    Keep,
}

/// Resolves the project-change warning. `Move` rejects with an active run, a worktree or a
/// plan file outside the new repo. In both cases the task gets tied to the current project's
/// rule if that rule points to its repo (so the next pull doesn't warn again about the same
/// change).
pub fn resolve_moved(conn: &Connection, task_id: &str, action: MovedAction, now: i64) -> Result<Task, DbError> {
    let task = rows::get_task(conn, task_id)?.ok_or_else(|| DbError::Invalid("Task not found.".into()))?;
    let Some(src) = task.source.as_ref() else {
        return Err(DbError::Invalid("This task is no longer linked to its source.".into()));
    };
    let Some(moved) = src.moved.as_ref() else {
        return Err(DbError::Invalid("This task has no pending project change.".into()));
    };
    let link = match src.link_id.as_deref() {
        Some(id) => rows::get_source_link(conn, id)?,
        None => None,
    };
    let repo_id = match action {
        MovedAction::Keep => task.repo_id.clone(),
        MovedAction::Move => {
            let Some(target) = moved.suggested_repo_id.clone() else {
                return Err(DbError::Invalid("No repo is suggested for its new project: keep it or move it by hand.".into()));
            };
            if target != task.repo_id {
                check_repo_in_project(conn, &target, &task.project_id)?;
                if has_active_run(conn, &task.id)? {
                    return Err(DbError::Invalid(
                        "The task has a queued or running run: wait for it or cancel it before moving it to another repo."
                            .into(),
                    ));
                }
                if task.worktree.is_some() {
                    return Err(DbError::Invalid(
                        "Clean up the task's worktree before moving it to another repo.".into(),
                    ));
                }
                if let PlanRef::File { path } = &task.plan {
                    let repo = rows::get_repo(conn, &target)?.ok_or_else(|| DbError::Invalid("Repo not found.".into()))?;
                    crate::work::validate::plan_file(std::path::Path::new(&repo.path), path).map_err(|_| {
                        DbError::Invalid("The plan file is in the old repo: pick a new plan for this task first.".into())
                    })?;
                }
            }
            target
        }
    };
    let current = moved.to_project.as_ref().map(|p| p.id.as_str());
    let rule_id = link
        .as_ref()
        .and_then(|l| super::import::project_rule(l, current))
        .filter(|r| r.repo_id == repo_id)
        .map(|r| r.id.clone());
    conn.execute(
        "UPDATE tasks SET repo_id = ?2, src_rule_id = ?3, src_moved = NULL, updated_at = ?4 WHERE id = ?1",
        params![task.id, repo_id, rule_id, now],
    )?;
    rows::get_task(conn, &task.id)?.ok_or_else(|| DbError::Invalid("Task not found.".into()))
}

pub fn set_sync_error(conn: &Connection, task_id: &str, error: Option<&str>) -> Result<(), DbError> {
    // A task unlinked while the sync was on the network doesn't get `src_*` again.
    conn.execute(
        "UPDATE tasks SET src_sync_error = ?2 WHERE id = ?1 AND src_provider IS NOT NULL",
        params![task_id, error],
    )?;
    Ok(())
}

/// After a successful state push: the saved external state becomes the pushed one, so the
/// next pull doesn't see it as a change.
pub fn set_external_state(conn: &Connection, task_id: &str, state: &ExternalState, now: i64) -> Result<(), DbError> {
    conn.execute(
        "UPDATE tasks SET src_state_json = ?2, src_last_synced_at = ?3, src_sync_error = NULL, src_unmapped = 0
         WHERE id = ?1 AND src_provider IS NOT NULL",
        params![task_id, json(state)?, now],
    )?;
    Ok(())
}

/// Unlink: the task stays local (all `src_*` and its outbox are cleared).
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
                          src_unmapped = 0, src_project_id = NULL, src_project_name = NULL, src_rule_id = NULL,
                          src_moved = NULL, updated_at = ?2
         WHERE id = ?1",
        params![task_id, now],
    )?;
    if n == 0 {
        return Err(DbError::Invalid("Task not found.".into()));
    }
    rows::get_task(conn, task_id)?.ok_or_else(|| DbError::Invalid("Task not found.".into()))
}

/// Disconnect: unlinks the link's tasks (they stay local) and deletes it, in one transaction.
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
// `enqueue_*` are called by the queue (`work::ops::push_status`), in the change's transaction.

fn task_provider(conn: &Connection, task_id: &str) -> Result<Option<String>, DbError> {
    Ok(conn
        .query_row("SELECT src_provider FROM tasks WHERE id = ?1", [task_id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten())
}

/// Enqueues the push of Nodal status `status` for an imported task. The external target is
/// resolved when draining, with the current mapping (so a mapping change or a pending mapping
/// are respected). Replaces any earlier state push still pending: only the last one matters.
/// Does nothing for local tasks. Returns whether it enqueued.
///
/// Payload contract: `SetState.state_id` carries the `TaskStatus` (`"in_review"`); a value
/// that isn't a `TaskStatus` is taken as a literal external state id.
pub fn enqueue_status(conn: &Connection, task_id: &str, status: TaskStatus, now: i64) -> Result<bool, DbError> {
    let Some(provider) = task_provider(conn, task_id)? else { return Ok(false) };
    conn.execute("DELETE FROM sync_outbox WHERE task_id = ?1 AND kind = 'set_state'", [task_id])?;
    let payload = OutboxPayload::SetState { state_id: status.as_str().to_string() };
    insert(conn, task_id, &provider, payload, now)?;
    Ok(true)
}

/// Enqueues a comment for an imported task (sent even if the mapping is pending).
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

/// Due rows of a provider, in creation order (the closing comment goes out after the state
/// change that accompanies it). A row doesn't go out while an earlier row of the same task
/// is still in backoff: per-task order is respected across passes.
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

/// A Nodal state change not yet pushed: the pull doesn't overwrite it.
pub fn has_pending_state_push(conn: &Connection, task_id: &str) -> Result<bool, DbError> {
    Ok(conn
        .query_row("SELECT 1 FROM sync_outbox WHERE task_id = ?1 AND kind = 'set_state' LIMIT 1", [task_id], |_| Ok(()))
        .optional()?
        .is_some())
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
