//! Versioned migrations with `PRAGMA user_version`.
//! `MIGRATIONS[i]` takes the database from version `i` to `i + 1`. A published migration is
//! never edited: changes go in a new one at the end.
//!
//! Conventions: TEXT ids, INTEGER dates (epoch ms), INTEGER 0/1 booleans, TEXT enums
//! (the `as_str` from `domain`), nested structures in `*_json` columns.

use rusqlite::Connection;

use super::DbError;

/// v1: the plan's full schema.
///
/// FKs:
/// - deleting a project cascades to its repos, tasks and source links;
/// - deleting a repo with tasks fails (`tasks.repo_id` is NO ACTION: it is checked at the end
///   of the statement, so the cascade from the project doesn't clash with the deletion order);
/// - the task points to `(repo_id, project_id)`: it can't end up in another project's repo;
/// - deleting a source link with linked tasks fails: they must be unlinked first
///   (clear every `src_*` in the same transaction) so they become local;
/// - deleting a task deletes its relations and its outbox; its runs keep a NULL `task_id`.
const V1: &str = r#"
CREATE TABLE projects (
    id                    TEXT PRIMARY KEY,
    name                  TEXT NOT NULL,
    key                   TEXT NOT NULL UNIQUE CHECK (key = upper(key) AND length(key) > 0),
    next_task_number      INTEGER NOT NULL DEFAULT 1,
    color                 TEXT NOT NULL,
    default_executor_json TEXT,
    reviewer              TEXT,
    created_at            INTEGER NOT NULL,
    archived_at           INTEGER
);

CREATE TABLE repos (
    id                    TEXT PRIMARY KEY,
    project_id            TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    path                  TEXT NOT NULL UNIQUE,
    name                  TEXT NOT NULL,
    model                 TEXT,
    effort                TEXT,
    permission_mode       TEXT,
    default_executor_json TEXT,
    default_isolation     TEXT NOT NULL DEFAULT 'worktree',
    default_finish        TEXT NOT NULL DEFAULT 'pr',
    default_review        INTEGER NOT NULL DEFAULT 1,
    reviewer              TEXT,
    position              INTEGER NOT NULL DEFAULT 0,
    created_at            INTEGER NOT NULL,
    UNIQUE (id, project_id)
);
CREATE INDEX repos_project ON repos(project_id, position);

CREATE TABLE source_links (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider        TEXT NOT NULL,
    scope_kind      TEXT NOT NULL,
    scope_id        TEXT NOT NULL,
    scope_name      TEXT NOT NULL,
    default_repo_id TEXT REFERENCES repos(id) ON DELETE SET NULL,
    repo_rules_json TEXT NOT NULL DEFAULT '[]',
    state_map_json  TEXT NOT NULL DEFAULT '{}',
    auto_import     INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    UNIQUE (project_id, provider, scope_id)
);
CREATE INDEX source_links_repo ON source_links(default_repo_id);

CREATE TABLE tasks (
    id                 TEXT PRIMARY KEY,
    project_id         TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    repo_id            TEXT NOT NULL,
    number             INTEGER NOT NULL,
    title              TEXT NOT NULL,
    status             TEXT NOT NULL,
    priority           TEXT NOT NULL DEFAULT 'none',
    labels_json        TEXT NOT NULL DEFAULT '[]',
    position           REAL NOT NULL DEFAULT 0,
    plan_kind          TEXT NOT NULL,
    plan_path          TEXT,
    plan_overridden    INTEGER NOT NULL DEFAULT 0,
    acceptance_json    TEXT NOT NULL DEFAULT '[]',
    assignee_json      TEXT,
    isolation          TEXT,
    finish             TEXT,
    review             INTEGER,
    wt_path            TEXT,
    wt_branch          TEXT,
    wt_base            TEXT,
    src_provider       TEXT,
    src_link_id        TEXT REFERENCES source_links(id),
    src_external_id    TEXT,
    src_identifier     TEXT,
    src_url            TEXT,
    src_state_json     TEXT,
    src_last_synced_at INTEGER,
    src_sync_error     TEXT,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    closed_at          INTEGER,
    FOREIGN KEY (repo_id, project_id) REFERENCES repos(id, project_id),
    UNIQUE (project_id, number),
    -- `external_id` must be unique per provider (prefixed with the org if needed).
    UNIQUE (src_provider, src_external_id),
    CHECK (plan_kind IN ('text', 'file') AND (plan_kind = 'file') = (plan_path IS NOT NULL)),
    CHECK ((wt_path IS NULL) = (wt_branch IS NULL) AND (wt_path IS NULL) = (wt_base IS NULL)),
    CHECK ((src_provider IS NULL) = (src_external_id IS NULL)
       AND (src_provider IS NULL) = (src_identifier IS NULL)
       AND (src_provider IS NULL) = (src_url IS NULL)),
    CHECK (src_link_id IS NULL OR src_provider IS NOT NULL)
);
CREATE INDEX tasks_board ON tasks(project_id, status, position);
CREATE INDEX tasks_repo ON tasks(repo_id);
CREATE INDEX tasks_link ON tasks(src_link_id);

CREATE TABLE task_relations (
    task_id  TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    other_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL,
    PRIMARY KEY (task_id, other_id, kind),
    CHECK (task_id <> other_id),
    -- `related` is symmetric: stored only once, with the ids ordered.
    CHECK (kind <> 'related' OR task_id < other_id)
);
CREATE INDEX task_relations_other ON task_relations(other_id);

CREATE TABLE runs (
    id                 TEXT PRIMARY KEY,
    task_id            TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    repo_id            TEXT REFERENCES repos(id) ON DELETE SET NULL,
    cwd                TEXT NOT NULL,
    executor_json      TEXT NOT NULL,
    kind               TEXT NOT NULL,
    parent_run_id      TEXT REFERENCES runs(id) ON DELETE SET NULL,
    prompt             TEXT NOT NULL,
    extra_instructions TEXT,
    options_json       TEXT NOT NULL DEFAULT '{}',
    finish             TEXT NOT NULL,
    isolation          TEXT,
    review             INTEGER NOT NULL DEFAULT 0,
    verdict_json       TEXT,
    status             TEXT NOT NULL,
    queue_position     REAL NOT NULL,
    claude_run_id      TEXT,
    session_id         TEXT,
    queued_at          INTEGER NOT NULL,
    launched_at        INTEGER,
    finished_at        INTEGER,
    outcome            TEXT,
    summary            TEXT,
    pr_url             TEXT,
    branch             TEXT,
    error              TEXT,
    legacy_label       TEXT
);
CREATE INDEX runs_task ON runs(task_id, queued_at);
CREATE INDEX runs_status ON runs(status, queue_position);
CREATE INDEX runs_parent ON runs(parent_run_id);
CREATE INDEX runs_repo ON runs(repo_id, status);

CREATE TABLE sync_outbox (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id         TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    provider        TEXT NOT NULL,
    kind            TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL,
    last_error      TEXT,
    created_at      INTEGER NOT NULL
);
CREATE INDEX sync_outbox_due ON sync_outbox(next_attempt_at);
CREATE INDEX sync_outbox_task ON sync_outbox(task_id);

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
);

-- Idempotency for "Import data from a previous version": one row per imported object.
CREATE TABLE legacy_imports (
    source_key  TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    target_id   TEXT,
    imported_at INTEGER NOT NULL
);
"#;

/// v2: additive (only `ADD COLUMN`, no table rebuilds).
/// - `projects.description`;
/// - `source_links.last_synced_at`/`last_sync_error`: result of the last sync pass per
///   link; `pending_state_changes`: JSON `{added, removed}` with the provider states that
///   changed against `known_states` (cleared when the mapping is saved);
/// - `tasks.src_unmapped`: the current external state is not in the pull mapping;
/// - `runs.tokens`: transcript tokens (agent/Claude/reviewer) summed on close.
///
/// `Settings.defaultExecutor` is just another `settings` row: it needs no DDL.
const V2: &str = r#"
ALTER TABLE projects ADD COLUMN description TEXT;
ALTER TABLE source_links ADD COLUMN last_synced_at INTEGER;
ALTER TABLE source_links ADD COLUMN last_sync_error TEXT;
ALTER TABLE source_links ADD COLUMN pending_state_changes TEXT;
ALTER TABLE tasks ADD COLUMN src_unmapped INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN tokens INTEGER;
"#;

/// v3: additive. Routing rules by provider project (they go in `repo_rules_json`, no
/// DDL) and, on the task:
/// - `src_project_id`/`src_project_name`: provider project seen on the last pull;
/// - `src_rule_id`: project rule through which it reached its repo;
/// - `src_moved`: JSON `{fromProject, toProject, suggestedRepoId}` if the issue changed
///   project and the user still has to decide (`resolve_moved_task`).
const V3: &str = r#"
ALTER TABLE tasks ADD COLUMN src_project_id TEXT;
ALTER TABLE tasks ADD COLUMN src_project_name TEXT;
ALTER TABLE tasks ADD COLUMN src_rule_id TEXT;
ALTER TABLE tasks ADD COLUMN src_moved TEXT;
"#;

pub const MIGRATIONS: &[&str] = &[V1, V2, V3];

pub fn user_version(conn: &Connection) -> Result<i64, DbError> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Applies the pending migrations, each in its own transaction. Idempotent.
/// Fails if the database is from a newer version than this app.
///
/// A future migration that rebuilds tables needs FKs turned off, and
/// `PRAGMA foreign_keys` does nothing inside a transaction: they'll have to be turned off
/// before `transaction()` and `PRAGMA foreign_key_check` run before the commit.
pub fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    let current = user_version(conn)?;
    let target = MIGRATIONS.len() as i64;
    if current > target {
        return Err(DbError::Invalid(format!(
            "The database is from a newer version of Nodal (schema v{current}, this build supports v{target})."
        )));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
