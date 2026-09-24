//! Migraciones versionadas con `PRAGMA user_version`.
//! `MIGRATIONS[i]` lleva la base de la versión `i` a la `i + 1`. Nunca se edita una
//! migración publicada: los cambios van en una nueva al final.
//!
//! Convenciones: ids TEXT, fechas INTEGER (epoch ms), booleanos INTEGER 0/1, enums TEXT
//! (el `as_str` de `domain`), estructuras anidadas en columnas `*_json`.

use rusqlite::Connection;

use super::DbError;

/// v1: esquema completo del plan.
///
/// FKs:
/// - borrar un proyecto borra en cascada sus repos, tareas y source links;
/// - borrar un repo con tareas falla (`tasks.repo_id` es NO ACTION: se comprueba al final de
///   la sentencia, así el cascade desde el proyecto no choca con el orden de borrado);
/// - la tarea apunta a `(repo_id, project_id)`: no puede quedar en un repo de otro proyecto;
/// - borrar un source link con tareas vinculadas falla: primero hay que desvincularlas
///   (limpiar todos los `src_*` en la misma transacción) para que queden locales;
/// - borrar una tarea borra sus relaciones y su outbox; sus runs quedan con `task_id` NULL.
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
    -- `external_id` tiene que ser único por proveedor (con la org como prefijo si hace falta).
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
    -- `related` es simétrica: se guarda una sola vez, con los ids ordenados.
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

-- Idempotencia de "Import data from a previous version": una fila por objeto importado.
CREATE TABLE legacy_imports (
    source_key  TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    target_id   TEXT,
    imported_at INTEGER NOT NULL
);
"#;

/// v2: aditiva (solo `ADD COLUMN`, sin reconstruir tablas).
/// - `projects.description`;
/// - `source_links.last_synced_at`/`last_sync_error`: resultado de la última pasada del sync
///   por link; `pending_state_changes`: JSON `{added, removed}` con los estados del proveedor
///   que cambiaron contra `known_states` (se limpia al guardar el mapeo);
/// - `tasks.src_unmapped`: el estado externo actual no está en el mapeo pull;
/// - `runs.tokens`: tokens del transcript (agente/Claude/revisor) sumados al cerrar.
///
/// `Settings.defaultExecutor` es una fila más de `settings`: no necesita DDL.
const V2: &str = r#"
ALTER TABLE projects ADD COLUMN description TEXT;
ALTER TABLE source_links ADD COLUMN last_synced_at INTEGER;
ALTER TABLE source_links ADD COLUMN last_sync_error TEXT;
ALTER TABLE source_links ADD COLUMN pending_state_changes TEXT;
ALTER TABLE tasks ADD COLUMN src_unmapped INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN tokens INTEGER;
"#;

/// v3: aditiva. Reglas de ruteo por proyecto del proveedor (van en `repo_rules_json`, sin
/// DDL) y, en la tarea:
/// - `src_project_id`/`src_project_name`: proyecto del proveedor visto en el último pull;
/// - `src_rule_id`: regla de proyecto por la que llegó a su repo;
/// - `src_moved`: JSON `{fromProject, toProject, suggestedRepoId}` si la issue cambió de
///   proyecto y falta que el usuario decida (`resolve_moved_task`).
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

/// Aplica las migraciones pendientes, cada una en su transacción. Idempotente.
/// Falla si la base es de una versión más nueva que esta app.
///
/// Una migración futura que reconstruya tablas necesita las FKs apagadas, y
/// `PRAGMA foreign_keys` no hace nada dentro de una transacción: habrá que apagarlas antes de
/// `transaction()` y correr `PRAGMA foreign_key_check` antes del commit.
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
