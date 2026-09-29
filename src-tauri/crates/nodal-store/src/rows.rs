//! Row ↔ struct conversion for each `domain` entity.
//! Only the minimum (insert, read by id): the full CRUD lives in `board`/`execution`/`chats`.

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, Type, ValueRef};
use rusqlite::{named_params, OptionalExtension, Row};

use crate::json::{get_json, get_opt_json, opt_json, to_json};
use crate::Conn as Connection;
use crate::DbError;
use nodal_domain::model::*;

// ---------- Column helpers ----------

/// A `domain` enum's TEXT representation (`as_str`/`parse`, from `str_enum!`).
trait SqlText: Copy {
    const NAME: &'static str;
    fn as_str(self) -> &'static str;
    fn parse(s: &str) -> Option<Self>;
}

/// `ToSql`/`FromSql` for a `domain` enum, through a local newtype: `domain`'s enums are a
/// foreign type and `rusqlite`'s traits are foreign traits, so a direct `impl` would violate
/// the orphan rule. Binds become `SqlEnum(x)` (`&SqlEnum(x)` inside `params!`/`named_params!`);
/// reads become `row.get::<_, SqlEnum<T>>(..)?.0`.
pub struct SqlEnum<T>(pub T);

impl<T: SqlText> ToSql for SqlEnum<T> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.0.as_str()))
    }
}

impl<T: SqlText> FromSql for SqlEnum<T> {
    fn column_result(v: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = v.as_str()?;
        T::parse(s).map(SqlEnum).ok_or_else(|| FromSqlError::Other(format!("invalid {} \"{s}\"", T::NAME).into()))
    }
}

macro_rules! sql_enum {
    ($($ty:ident),+) => {$(
        impl SqlText for $ty {
            const NAME: &'static str = stringify!($ty);
            fn as_str(self) -> &'static str { $ty::as_str(self) }
            fn parse(s: &str) -> Option<Self> { $ty::parse(s) }
        }
    )+};
}
sql_enum!(TaskStatus, Priority, Isolation, Finish, RelationKind, RunKind, RunStatus, RunOutcome);

// ---------- Project ----------

pub fn project_from_row(row: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get("id")?,
        name: row.get("name")?,
        key: row.get("key")?,
        next_task_number: row.get("next_task_number")?,
        color: row.get("color")?,
        description: row.get("description")?,
        root_path: row.get("root_path")?,
        default_executor: get_opt_json(row, "default_executor_json")?,
        reviewer: row.get("reviewer")?,
        created_at: row.get("created_at")?,
        archived_at: row.get("archived_at")?,
    })
}

pub fn insert_project(conn: &Connection, p: &Project) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO projects (id, name, key, next_task_number, color, default_executor_json,
                               reviewer, created_at, archived_at, description, root_path)
         VALUES (:id, :name, :key, :next, :color, :exec, :reviewer, :created, :archived, :description,
                 :root_path)",
        named_params! {
            ":id": p.id, ":name": p.name, ":key": p.key, ":next": p.next_task_number,
            ":color": p.color, ":exec": opt_json(&p.default_executor)?, ":reviewer": p.reviewer,
            ":created": p.created_at, ":archived": p.archived_at, ":description": p.description,
            ":root_path": p.root_path,
        },
    )?;
    Ok(())
}

pub fn get_project(conn: &Connection, id: &str) -> Result<Option<Project>, DbError> {
    Ok(conn.query_row("SELECT * FROM projects WHERE id = ?1", [id], project_from_row).optional()?)
}

// ---------- Repo ----------

pub fn repo_from_row(row: &Row) -> rusqlite::Result<Repo> {
    Ok(Repo {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        path: row.get("path")?,
        name: row.get("name")?,
        launch: LaunchOptions {
            model: row.get("model")?,
            effort: row.get("effort")?,
            permission_mode: row.get("permission_mode")?,
        },
        default_executor: get_opt_json(row, "default_executor_json")?,
        default_isolation: row.get::<_, SqlEnum<Isolation>>("default_isolation")?.0,
        default_finish: row.get::<_, SqlEnum<Finish>>("default_finish")?.0,
        default_review: row.get("default_review")?,
        reviewer: row.get("reviewer")?,
        position: row.get("position")?,
        created_at: row.get("created_at")?,
    })
}

pub fn insert_repo(conn: &Connection, r: &Repo) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO repos (id, project_id, path, name, model, effort, permission_mode,
                            default_executor_json, default_isolation, default_finish,
                            default_review, reviewer, position, created_at)
         VALUES (:id, :project, :path, :name, :model, :effort, :perm, :exec, :iso, :finish,
                 :review, :reviewer, :pos, :created)",
        named_params! {
            ":id": r.id, ":project": r.project_id, ":path": r.path, ":name": r.name,
            ":model": r.launch.model, ":effort": r.launch.effort, ":perm": r.launch.permission_mode,
            ":exec": opt_json(&r.default_executor)?, ":iso": SqlEnum(r.default_isolation),
            ":finish": SqlEnum(r.default_finish), ":review": r.default_review, ":reviewer": r.reviewer,
            ":pos": r.position, ":created": r.created_at,
        },
    )?;
    Ok(())
}

pub fn get_repo(conn: &Connection, id: &str) -> Result<Option<Repo>, DbError> {
    Ok(conn.query_row("SELECT * FROM repos WHERE id = ?1", [id], repo_from_row).optional()?)
}

// ---------- Chat ----------

pub fn chat_from_row(row: &Row) -> rusqlite::Result<Chat> {
    Ok(Chat {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        repo_id: row.get("repo_id")?,
        title: row.get("title")?,
        session_title: row.get("session_title")?,
        session_id: row.get("session_id")?,
        launch: LaunchOptions {
            model: row.get("model")?,
            effort: row.get("effort")?,
            permission_mode: row.get("permission_mode")?,
        },
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn insert_chat(conn: &Connection, c: &Chat) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO chats (id, project_id, repo_id, title, session_title, session_id, model, effort,
                            permission_mode, created_at, updated_at)
         VALUES (:id, :project, :repo, :title, :stitle, :session, :model, :effort, :perm, :created, :updated)",
        named_params! {
            ":id": c.id, ":project": c.project_id, ":repo": c.repo_id, ":title": c.title, ":stitle": c.session_title,
            ":session": c.session_id, ":model": c.launch.model, ":effort": c.launch.effort,
            ":perm": c.launch.permission_mode, ":created": c.created_at, ":updated": c.updated_at,
        },
    )?;
    Ok(())
}

pub fn get_chat(conn: &Connection, id: &str) -> Result<Option<Chat>, DbError> {
    Ok(conn.query_row("SELECT * FROM chats WHERE id = ?1", [id], chat_from_row).optional()?)
}

// ---------- Task ----------

pub fn task_from_row(row: &Row) -> rusqlite::Result<Task> {
    let plan = match row.get::<_, String>("plan_kind")?.as_str() {
        "text" => PlanRef::Text,
        "file" => PlanRef::File { path: row.get("plan_path")? },
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                Type::Text,
                format!("invalid plan_kind \"{other}\"").into(),
            ))
        }
    };
    let worktree = match row.get::<_, Option<String>>("wt_path")? {
        Some(path) => Some(WorktreeRef { path, branch: row.get("wt_branch")?, base: row.get("wt_base")? }),
        None => None,
    };
    let source = match row.get::<_, Option<String>>("src_provider")? {
        Some(provider) => Some(TaskSource {
            provider,
            link_id: row.get("src_link_id")?,
            external_id: row.get("src_external_id")?,
            identifier: row.get("src_identifier")?,
            url: row.get("src_url")?,
            external_state: get_opt_json(row, "src_state_json")?,
            last_synced_at: row.get("src_last_synced_at")?,
            sync_error: row.get("src_sync_error")?,
            unmapped: row.get("src_unmapped")?,
            project: match row.get::<_, Option<String>>("src_project_id")? {
                Some(id) => Some(ExtProject { id, name: row.get::<_, Option<String>>("src_project_name")?.unwrap_or_default() }),
                None => None,
            },
            rule_id: row.get("src_rule_id")?,
            moved: get_opt_json(row, "src_moved")?,
        }),
        None => None,
    };
    Ok(Task {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        repo_id: row.get("repo_id")?,
        number: row.get("number")?,
        title: row.get("title")?,
        status: row.get::<_, SqlEnum<TaskStatus>>("status")?.0,
        priority: row.get::<_, SqlEnum<Priority>>("priority")?.0,
        labels: get_json(row, "labels_json")?,
        position: row.get("position")?,
        plan,
        plan_overridden: row.get("plan_overridden")?,
        acceptance: get_json(row, "acceptance_json")?,
        assignee: get_opt_json(row, "assignee_json")?,
        isolation: row.get::<_, Option<SqlEnum<Isolation>>>("isolation")?.map(|s| s.0),
        finish: row.get::<_, Option<SqlEnum<Finish>>>("finish")?.map(|s| s.0),
        review: row.get("review")?,
        worktree,
        source,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        closed_at: row.get("closed_at")?,
    })
}

/// Inserts the task as is (with its `number` already assigned: doesn't touch the project counter).
pub fn insert_task(conn: &Connection, t: &Task) -> Result<(), DbError> {
    let (plan_kind, plan_path) = match &t.plan {
        PlanRef::Text => ("text", None),
        PlanRef::File { path } => ("file", Some(path.as_str())),
    };
    let wt = t.worktree.as_ref();
    let src = t.source.as_ref();
    conn.execute(
        "INSERT INTO tasks (id, project_id, repo_id, number, title, status, priority, labels_json,
                            position, plan_kind, plan_path, plan_overridden, acceptance_json,
                            assignee_json, isolation, finish, review, wt_path, wt_branch, wt_base,
                            src_provider, src_link_id, src_external_id, src_identifier, src_url,
                            src_state_json, src_last_synced_at, src_sync_error, src_unmapped,
                            src_project_id, src_project_name, src_rule_id, src_moved,
                            created_at, updated_at, closed_at)
         VALUES (:id, :project, :repo, :number, :title, :status, :priority, :labels,
                 :position, :plan_kind, :plan_path, :plan_overridden, :acceptance,
                 :assignee, :isolation, :finish, :review, :wt_path, :wt_branch, :wt_base,
                 :src_provider, :src_link, :src_ext, :src_ident, :src_url,
                 :src_state, :src_synced, :src_error, :src_unmapped,
                 :src_project, :src_project_name, :src_rule, :src_moved,
                 :created, :updated, :closed)",
        named_params! {
            ":id": t.id, ":project": t.project_id, ":repo": t.repo_id, ":number": t.number,
            ":title": t.title, ":status": SqlEnum(t.status), ":priority": SqlEnum(t.priority),
            ":labels": to_json(&t.labels)?, ":position": t.position,
            ":plan_kind": plan_kind, ":plan_path": plan_path, ":plan_overridden": t.plan_overridden,
            ":acceptance": to_json(&t.acceptance)?, ":assignee": opt_json(&t.assignee)?,
            ":isolation": t.isolation.map(SqlEnum), ":finish": t.finish.map(SqlEnum), ":review": t.review,
            ":wt_path": wt.map(|w| &w.path), ":wt_branch": wt.map(|w| &w.branch),
            ":wt_base": wt.map(|w| &w.base),
            ":src_provider": src.map(|s| &s.provider), ":src_link": src.and_then(|s| s.link_id.as_ref()),
            ":src_ext": src.map(|s| &s.external_id), ":src_ident": src.map(|s| &s.identifier),
            ":src_url": src.map(|s| &s.url),
            ":src_state": src.and_then(|s| s.external_state.as_ref()).map(to_json).transpose()?,
            ":src_synced": src.and_then(|s| s.last_synced_at),
            ":src_error": src.and_then(|s| s.sync_error.as_ref()),
            ":src_unmapped": src.is_some_and(|s| s.unmapped),
            ":src_project": src.and_then(|s| s.project.as_ref()).map(|p| &p.id),
            ":src_project_name": src.and_then(|s| s.project.as_ref()).map(|p| &p.name),
            ":src_rule": src.and_then(|s| s.rule_id.as_ref()),
            ":src_moved": src.and_then(|s| s.moved.as_ref()).map(to_json).transpose()?,
            ":created": t.created_at, ":updated": t.updated_at, ":closed": t.closed_at,
        },
    )?;
    Ok(())
}

pub fn get_task(conn: &Connection, id: &str) -> Result<Option<Task>, DbError> {
    Ok(conn.query_row("SELECT * FROM tasks WHERE id = ?1", [id], task_from_row).optional()?)
}

// ---------- TaskRelation ----------

pub fn relation_from_row(row: &Row) -> rusqlite::Result<TaskRelation> {
    Ok(TaskRelation {
        task_id: row.get("task_id")?,
        other_id: row.get("other_id")?,
        kind: row.get::<_, SqlEnum<RelationKind>>("kind")?.0,
    })
}

/// `Related` is symmetric: stored with the ids ordered (a CHECK requires it).
pub fn insert_relation(conn: &Connection, r: &TaskRelation) -> Result<(), DbError> {
    let (a, b) = match r.kind {
        RelationKind::Related if r.task_id > r.other_id => (&r.other_id, &r.task_id),
        _ => (&r.task_id, &r.other_id),
    };
    conn.execute(
        "INSERT INTO task_relations (task_id, other_id, kind) VALUES (?1, ?2, ?3)",
        rusqlite::params![a, b, SqlEnum(r.kind)],
    )?;
    Ok(())
}

/// Relations the task takes part in, on either side.
pub fn relations_of(conn: &Connection, task_id: &str) -> Result<Vec<TaskRelation>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT * FROM task_relations WHERE task_id = ?1 OR other_id = ?1 ORDER BY task_id, other_id, kind",
    )?;
    let rows = stmt.query_map([task_id], relation_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---------- Run ----------

pub fn run_from_row(row: &Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        repo_id: row.get("repo_id")?,
        cwd: row.get("cwd")?,
        executor: get_json(row, "executor_json")?,
        kind: row.get::<_, SqlEnum<RunKind>>("kind")?.0,
        parent_run_id: row.get("parent_run_id")?,
        prompt: row.get("prompt")?,
        extra_instructions: row.get("extra_instructions")?,
        options: get_json(row, "options_json")?,
        finish: row.get::<_, SqlEnum<Finish>>("finish")?.0,
        isolation: row.get::<_, Option<SqlEnum<Isolation>>>("isolation")?.map(|s| s.0),
        review: row.get("review")?,
        verdict: get_opt_json(row, "verdict_json")?,
        status: row.get::<_, SqlEnum<RunStatus>>("status")?.0,
        queue_position: row.get("queue_position")?,
        claude_run_id: row.get("claude_run_id")?,
        session_id: row.get("session_id")?,
        queued_at: row.get("queued_at")?,
        launched_at: row.get("launched_at")?,
        finished_at: row.get("finished_at")?,
        outcome: row.get::<_, Option<SqlEnum<RunOutcome>>>("outcome")?.map(|s| s.0),
        summary: row.get("summary")?,
        pr_url: row.get("pr_url")?,
        branch: row.get("branch")?,
        error: row.get("error")?,
        legacy_label: row.get("legacy_label")?,
        tokens: row.get("tokens")?,
    })
}

pub fn insert_run(conn: &Connection, r: &Run) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO runs (id, task_id, repo_id, cwd, executor_json, kind, parent_run_id, prompt,
                           extra_instructions, options_json, finish, isolation, review, verdict_json, status,
                           queue_position, claude_run_id, session_id, queued_at, launched_at,
                           finished_at, outcome, summary, pr_url, branch, error, legacy_label, tokens)
         VALUES (:id, :task, :repo, :cwd, :exec, :kind, :parent, :prompt,
                 :extra, :options, :finish, :isolation, :review, :verdict, :status,
                 :qpos, :claude_id, :session, :queued, :launched,
                 :finished, :outcome, :summary, :pr, :branch, :error, :legacy, :tokens)",
        named_params! {
            ":id": r.id, ":task": r.task_id, ":repo": r.repo_id, ":cwd": r.cwd,
            ":exec": to_json(&r.executor)?, ":kind": SqlEnum(r.kind), ":parent": r.parent_run_id,
            ":prompt": r.prompt, ":extra": r.extra_instructions, ":options": to_json(&r.options)?,
            ":finish": SqlEnum(r.finish), ":isolation": r.isolation.map(SqlEnum), ":review": r.review,
            ":verdict": opt_json(&r.verdict)?, ":status": SqlEnum(r.status),
            ":qpos": r.queue_position, ":claude_id": r.claude_run_id, ":session": r.session_id,
            ":queued": r.queued_at, ":launched": r.launched_at, ":finished": r.finished_at,
            ":outcome": r.outcome.map(SqlEnum), ":summary": r.summary, ":pr": r.pr_url, ":branch": r.branch,
            ":error": r.error, ":legacy": r.legacy_label, ":tokens": r.tokens,
        },
    )?;
    Ok(())
}

pub fn get_run(conn: &Connection, id: &str) -> Result<Option<Run>, DbError> {
    let mut stmt = conn.prepare_cached("SELECT * FROM runs WHERE id = ?1")?;
    Ok(stmt.query_row([id], run_from_row).optional()?)
}

/// Explicit columns of `run_from_row`, minus `prompt`/`extra_instructions`: for lists that
/// map straight to `RunLight` (history, board), so the prompt blob never leaves SQLite.
pub const RUN_LIGHT_COLUMNS: &str = "r.id, r.task_id, r.repo_id, r.cwd, r.executor_json, r.kind, \
    r.parent_run_id, r.options_json, r.finish, r.isolation, r.review, r.verdict_json, r.status, \
    r.queue_position, r.claude_run_id, r.session_id, r.queued_at, r.launched_at, r.finished_at, \
    r.outcome, r.summary, r.pr_url, r.branch, r.error, r.legacy_label, r.tokens";

pub fn run_light_from_row(row: &Row) -> rusqlite::Result<RunLight> {
    Ok(RunLight {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        repo_id: row.get("repo_id")?,
        cwd: row.get("cwd")?,
        executor: get_json(row, "executor_json")?,
        kind: row.get::<_, SqlEnum<RunKind>>("kind")?.0,
        parent_run_id: row.get("parent_run_id")?,
        options: get_json(row, "options_json")?,
        finish: row.get::<_, SqlEnum<Finish>>("finish")?.0,
        isolation: row.get::<_, Option<SqlEnum<Isolation>>>("isolation")?.map(|s| s.0),
        review: row.get("review")?,
        verdict: get_opt_json(row, "verdict_json")?,
        status: row.get::<_, SqlEnum<RunStatus>>("status")?.0,
        queue_position: row.get("queue_position")?,
        claude_run_id: row.get("claude_run_id")?,
        session_id: row.get("session_id")?,
        queued_at: row.get("queued_at")?,
        launched_at: row.get("launched_at")?,
        finished_at: row.get("finished_at")?,
        outcome: row.get::<_, Option<SqlEnum<RunOutcome>>>("outcome")?.map(|s| s.0),
        summary: row.get("summary")?,
        pr_url: row.get("pr_url")?,
        branch: row.get("branch")?,
        error: row.get("error")?,
        legacy_label: row.get("legacy_label")?,
        tokens: row.get("tokens")?,
    })
}

// ---------- SourceLink ----------

pub fn source_link_from_row(row: &Row) -> rusqlite::Result<SourceLink> {
    Ok(SourceLink {
        id: row.get("id")?,
        project_id: row.get("project_id")?,
        provider: row.get("provider")?,
        scope: ScopeRef { kind: row.get("scope_kind")?, id: row.get("scope_id")?, name: row.get("scope_name")? },
        default_repo_id: row.get("default_repo_id")?,
        repo_rules: {
            let mut rules: Vec<RepoRule> = get_json(row, "repo_rules_json")?;
            normalize_legacy_rules(&mut rules, row.get("created_at")?);
            rules
        },
        state_map: get_json(row, "state_map_json")?,
        auto_import: row.get("auto_import")?,
        created_at: row.get("created_at")?,
        last_synced_at: row.get("last_synced_at")?,
        last_sync_error: row.get("last_sync_error")?,
        pending_state_changes: get_opt_json(row, "pending_state_changes")?,
    })
}

pub fn insert_source_link(conn: &Connection, l: &SourceLink) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO source_links (id, project_id, provider, scope_kind, scope_id, scope_name,
                                   default_repo_id, repo_rules_json, state_map_json, auto_import,
                                   created_at, last_synced_at, last_sync_error, pending_state_changes)
         VALUES (:id, :project, :provider, :skind, :sid, :sname, :repo, :rules, :map, :auto, :created,
                 :synced, :sync_error, :pending)",
        named_params! {
            ":id": l.id, ":project": l.project_id, ":provider": l.provider,
            ":skind": l.scope.kind, ":sid": l.scope.id, ":sname": l.scope.name,
            ":repo": l.default_repo_id, ":rules": to_json(&l.repo_rules)?,
            ":map": to_json(&l.state_map)?, ":auto": l.auto_import, ":created": l.created_at,
            ":synced": l.last_synced_at, ":sync_error": l.last_sync_error,
            ":pending": opt_json(&l.pending_state_changes)?,
        },
    )?;
    Ok(())
}

pub fn get_source_link(conn: &Connection, id: &str) -> Result<Option<SourceLink>, DbError> {
    Ok(conn.query_row("SELECT * FROM source_links WHERE id = ?1", [id], source_link_from_row).optional()?)
}

// ---------- Outbox ----------

pub fn outbox_from_row(row: &Row) -> rusqlite::Result<OutboxItem> {
    Ok(OutboxItem {
        id: row.get("id")?,
        task_id: row.get("task_id")?,
        provider: row.get("provider")?,
        payload: get_json(row, "payload_json")?,
        attempts: row.get("attempts")?,
        next_attempt_at: row.get("next_attempt_at")?,
        last_error: row.get("last_error")?,
        created_at: row.get("created_at")?,
    })
}

/// Inserts ignoring `item.id` and returns the assigned id.
pub fn insert_outbox(conn: &Connection, o: &OutboxItem) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO sync_outbox (task_id, provider, kind, payload_json, attempts,
                                  next_attempt_at, last_error, created_at)
         VALUES (:task, :provider, :kind, :payload, :attempts, :next, :error, :created)",
        named_params! {
            ":task": o.task_id, ":provider": o.provider, ":kind": o.payload.kind(),
            ":payload": to_json(&o.payload)?, ":attempts": o.attempts, ":next": o.next_attempt_at,
            ":error": o.last_error, ":created": o.created_at,
        },
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
pub fn get_outbox(conn: &Connection, id: i64) -> Result<Option<OutboxItem>, DbError> {
    Ok(conn.query_row("SELECT * FROM sync_outbox WHERE id = ?1", [id], outbox_from_row).optional()?)
}

// ---------- Settings ----------

/// One row per `Settings` field. Missing or unreadable ones take their default
/// (a corrupt value doesn't invalidate the rest); `concurrency` is clamped to `1..=MAX_CONCURRENCY`.
pub fn load_settings(conn: &Connection) -> Result<Settings, DbError> {
    let serde_json::Value::Object(mut obj) = serde_json::to_value(Settings::default()).expect("Settings serializes")
    else {
        unreachable!("Settings serializes as an object");
    };
    let mut stmt = conn.prepare("SELECT key, value_json FROM settings")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (k, v) = row?;
        let Some(default) = obj.get(&k).cloned() else { continue };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&v) else { continue };
        obj.insert(k.clone(), value);
        if serde_json::from_value::<Settings>(serde_json::Value::Object(obj.clone())).is_err() {
            obj.insert(k, default);
        }
    }
    let mut out: Settings = serde_json::from_value(serde_json::Value::Object(obj)).expect("valid defaults");
    out.concurrency = out.concurrency.clamp(1, MAX_CONCURRENCY);
    Ok(out)
}

/// Saves every field in a single transaction.
pub fn save_settings(conn: &mut Connection, s: &Settings) -> Result<(), DbError> {
    let serde_json::Value::Object(obj) = serde_json::to_value(s).expect("Settings serializes") else {
        unreachable!("Settings serializes as an object");
    };
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
        )?;
        for (k, v) in obj {
            stmt.execute(rusqlite::params![k, v.to_string()])?;
        }
    }
    tx.commit()?;
    Ok(())
}
