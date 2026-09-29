//! MCP tools: their schemas and what each one runs. Every call goes through the same
//! `board::ops` and queries as the Tauri commands, so validation, numbering and side effects
//! (Linear outbox, positions, `closedAt`) match the UI.

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use nodal_domain::board::dto::{NewTask, PlanInput, TaskPatch};
use nodal_domain::board::validate;
use nodal_domain::model::*;
use nodal_domain::serde_util::double_option;
use nodal_domain::util::is_valid_id;
use nodal_store::board::{projects, repos, tasks};
use nodal_store::execution::runs as qruns;
use nodal_store::Conn as Connection;

use crate::board::ops;
use crate::core::Env;

/// Schemas moved to `nodal_mcp_proto::tools` (W1). `PROPOSE_TASK` is still used by `call` below
/// and by `chats::context`, but `definitions`/`chat_definitions`'s only production caller,
/// `mcp::stdio`, moved with them, so they (and `STATUSES`/`PRIORITIES`) are only reachable from
/// the tests below now; `#[cfg(test)]` keeps that bridge from warning as unused otherwise.
pub use nodal_mcp_proto::tools::PROPOSE_TASK;
#[cfg(test)]
use nodal_mcp_proto::tools::{chat_definitions, definitions, PRIORITIES, STATUSES};

/// Result of a tool, plus the project whose tasks changed (for `nodal://changed`).
#[derive(Debug)]
pub struct Outcome {
    pub value: Value,
    pub changed: Option<String>,
}

impl Outcome {
    fn read(value: Value) -> Self {
        Outcome {
            value,
            changed: None,
        }
    }
}

const RUNS_IN_TASK: usize = 10;

pub fn call(
    conn: &mut Connection,
    env: &Env,
    name: &str,
    args: Value,
    now: i64,
) -> Result<Outcome, String> {
    let args = if args.is_null() { json!({}) } else { args };
    match name {
        "list_projects" => list_projects(conn, parse(args)?).map(Outcome::read),
        "list_tasks" => list_tasks(conn, parse(args)?).map(Outcome::read),
        "get_task" => get_task(conn, env, parse(args)?).map(Outcome::read),
        "create_task" => create_task(conn, env, parse(args)?, now),
        "update_task" => update_task(conn, env, parse(args)?, now),
        "get_run" => get_run(conn, parse(args)?).map(Outcome::read),
        PROPOSE_TASK => propose_task(conn, env, parse(args)?).map(Outcome::read),
        _ => Err(format!("Unknown tool: {name}.")),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("Invalid arguments: {e}."))
}

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("Internal error: {e}"))
}

// ---------- Project, repo and task resolution ----------

/// Id or key (`PAY`, any case).
fn resolve_project(conn: &Connection, s: &str) -> Result<Project, String> {
    let s = s.trim();
    projects::list(conn, true)?
        .into_iter()
        .find(|p| p.id == s || p.key.eq_ignore_ascii_case(s))
        .ok_or_else(|| format!("No project with id or key \"{s}\". Use list_projects to see them."))
}

/// Id, name, or a path at or inside the repo (the deepest repo wins).
fn resolve_repo(
    conn: &Connection,
    env: &Env,
    s: &str,
    project: Option<&Project>,
) -> Result<Repo, String> {
    let s = s.trim();
    let all = repos::list(conn, project.map(|p| p.id.as_str()))?;
    if let Some(r) = all.iter().find(|r| r.id == s) {
        return Ok(r.clone());
    }
    let by_name: Vec<&Repo> = all
        .iter()
        .filter(|r| r.name.eq_ignore_ascii_case(s))
        .collect();
    match by_name.as_slice() {
        [r] => return Ok((*r).clone()),
        [_, _, ..] => {
            return Err(format!(
                "Several repos are named \"{s}\": pass `project` or the repo id."
            ))
        }
        [] => {}
    }
    let path = env.fs.expand_home(s);
    let path = env.fs.canonicalize(&path).unwrap_or(path);
    all.iter()
        .filter(|r| path.starts_with(Path::new(&r.path)))
        .max_by_key(|r| r.path.len())
        .cloned()
        .ok_or_else(|| {
            format!("No repo matches \"{s}\". Use list_projects to see the repos of each project.")
        })
}

/// Internal id (`t01…`) or visible key (`PAY-12`, any case).
fn resolve_task(conn: &Connection, s: &str) -> Result<Task, String> {
    let s = s.trim();
    if is_valid_id(s) {
        if let Ok(t) = tasks::get(conn, s) {
            return Ok(t);
        }
    }
    let missing = || format!("No task with id or key \"{s}\". Use list_tasks to find it.");
    let (key, number) = s.rsplit_once('-').ok_or_else(missing)?;
    let number: i64 = number.parse().map_err(|_| missing())?;
    let project = resolve_project(conn, key).map_err(|_| missing())?;
    tasks::find_by_number(conn, &project.id, number)?.ok_or_else(missing)
}

fn project_keys(conn: &Connection) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(projects::list(conn, true)?
        .into_iter()
        .map(|p| (p.id, p.key))
        .collect())
}

/// The task as the UI sees it, plus its visible `key`.
fn task_value(t: &Task, project_key: &str) -> Result<Value, String> {
    let mut v = to_value(t)?;
    if let Value::Object(m) = &mut v {
        m.insert("key".into(), Value::String(task_key(project_key, t.number)));
    }
    Ok(v)
}

fn key_of(conn: &Connection, t: &Task) -> Result<String, String> {
    Ok(projects::get(conn, &t.project_id)?.key)
}

// ---------- Tools ----------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListProjects {
    #[serde(default)]
    include_archived: bool,
}

fn list_projects(conn: &Connection, a: ListProjects) -> Result<Value, String> {
    let mut out = Vec::new();
    for p in projects::list(conn, a.include_archived)? {
        let repos = repos::list(conn, Some(&p.id))?;
        let mut v = to_value(&p)?;
        if let Value::Object(m) = &mut v {
            m.insert("repos".into(), to_value(&repos)?);
        }
        out.push(v);
    }
    Ok(Value::Array(out))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListTasks {
    project: Option<String>,
    status: Option<OneOrMany<TaskStatus>>,
}

fn list_tasks(conn: &Connection, a: ListTasks) -> Result<Value, String> {
    let project = a
        .project
        .as_deref()
        .map(|p| resolve_project(conn, p))
        .transpose()?;
    let statuses = match a.status {
        Some(OneOrMany::One(s)) => vec![s],
        Some(OneOrMany::Many(v)) => v,
        None => vec![],
    };
    let keys = project_keys(conn)?;
    tasks::list(conn, project.as_ref().map(|p| p.id.as_str()))?
        .iter()
        .filter(|t| statuses.is_empty() || statuses.contains(&t.status))
        .map(|t| {
            task_value(
                t,
                keys.get(&t.project_id)
                    .map(String::as_str)
                    .unwrap_or_default(),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetTask {
    task: String,
}

/// The task, its plan text and its latest runs (newest first).
fn get_task(conn: &Connection, env: &Env, a: GetTask) -> Result<Value, String> {
    let t = resolve_task(conn, &a.task)?;
    let mut v = task_value(&t, &key_of(conn, &t)?)?;
    let plan = ops::read_task_plan(conn, env, &t.id);
    let runs: Vec<RunLight> = qruns::list_filtered(conn, None, Some(&t.id))?
        .into_iter()
        .take(RUNS_IN_TASK)
        .map(RunLight::from)
        .collect();
    if let Value::Object(m) = &mut v {
        match plan {
            Ok(text) => m.insert("planText".into(), Value::String(text)),
            Err(e) => m.insert("planError".into(), Value::String(e)),
        };
        m.insert("runs".into(), to_value(&runs)?);
    }
    Ok(v)
}

/// Plan as markdown text or as a `.md` file of the repo; at most one of the two.
fn plan_input(
    plan: Option<String>,
    plan_file: Option<String>,
) -> Result<Option<PlanInput>, String> {
    match (plan, plan_file) {
        (Some(_), Some(_)) => Err("Pass either `plan` or `planFile`, not both.".into()),
        (Some(text), None) => Ok(Some(PlanInput::Text { text })),
        (None, Some(path)) => Ok(Some(PlanInput::File { path })),
        (None, None) => Ok(None),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateTask {
    repo: String,
    project: Option<String>,
    title: String,
    plan: Option<String>,
    plan_file: Option<String>,
    #[serde(default)]
    acceptance: Vec<String>,
    priority: Option<Priority>,
    #[serde(default)]
    labels: Vec<String>,
    executor: Option<Executor>,
    status: Option<TaskStatus>,
}

fn create_task(
    conn: &mut Connection,
    env: &Env,
    a: CreateTask,
    now: i64,
) -> Result<Outcome, String> {
    let project = a
        .project
        .as_deref()
        .map(|p| resolve_project(conn, p))
        .transpose()?;
    let repo = resolve_repo(conn, env, &a.repo, project.as_ref())?;
    let plan = plan_input(a.plan, a.plan_file)?
        .ok_or("A task needs a plan: pass `plan` (markdown) or `planFile`.")?;
    let input = NewTask {
        project_id: repo.project_id.clone(),
        repo_id: repo.id,
        title: a.title,
        plan,
        status: a.status,
        priority: a.priority,
        labels: Some(a.labels),
        acceptance: Some(a.acceptance),
        assignee: a.executor,
        isolation: None,
        finish: None,
        review: None,
    };
    let t = ops::create_task(conn, env, &input, now)?;
    Ok(Outcome {
        value: task_value(&t, &key_of(conn, &t)?)?,
        changed: Some(t.project_id),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProposeTask {
    repo: String,
    project: Option<String>,
    title: String,
    plan: String,
    #[serde(default)]
    acceptance: Vec<String>,
    priority: Option<Priority>,
    #[serde(default)]
    labels: Vec<String>,
    executor: Option<Executor>,
    status: Option<TaskStatus>,
}

/// Validates like `create_task` and returns the `NewTask` the card creates on accept
/// (mirror of `TaskProposal` in `api.ts`), without touching the board.
fn propose_task(conn: &Connection, env: &Env, a: ProposeTask) -> Result<Value, String> {
    let project = a
        .project
        .as_deref()
        .map(|p| resolve_project(conn, p))
        .transpose()?;
    let repo = resolve_repo(conn, env, &a.repo, project.as_ref())?;
    let title = validate::title(&a.title)?;
    validate::plan_text(&a.plan)?;
    if let Some(e) = &a.executor {
        validate::executor(e)?;
    }
    let labels = validate::labels(&a.labels)?;
    let acceptance = validate::acceptance(&a.acceptance)?;
    let project = projects::get(conn, &repo.project_id)?;
    Ok(json!({
        "newTask": {
            "projectId": project.id,
            "repoId": repo.id,
            "title": title,
            "plan": {"kind": "text", "text": a.plan},
            "status": a.status.unwrap_or(TaskStatus::Todo),
            "priority": a.priority.unwrap_or_default(),
            "labels": labels,
            "acceptance": acceptance,
            "assignee": a.executor,
        },
        "projectKey": project.key,
        "repoName": repo.name,
        "created": false,
        "note": "Shown to the user as a task card; nothing was created. The user creates it from the card."
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateTask {
    task: String,
    repo: Option<String>,
    title: Option<String>,
    plan: Option<String>,
    plan_file: Option<String>,
    status: Option<TaskStatus>,
    priority: Option<Priority>,
    labels: Option<Vec<String>>,
    acceptance: Option<Vec<String>>,
    /// `null` goes back to the repo's default executor.
    #[serde(default, deserialize_with = "double_option")]
    executor: Option<Option<Executor>>,
}

fn update_task(
    conn: &mut Connection,
    env: &Env,
    a: UpdateTask,
    now: i64,
) -> Result<Outcome, String> {
    let t = resolve_task(conn, &a.task)?;
    let repo_id = match a.repo.as_deref() {
        Some(r) => Some(resolve_repo(conn, env, r, Some(&projects::get(conn, &t.project_id)?))?.id),
        None => None,
    };
    let patch = TaskPatch {
        repo_id,
        title: a.title,
        plan: plan_input(a.plan, a.plan_file)?,
        status: a.status,
        priority: a.priority,
        labels: a.labels,
        acceptance: a.acceptance,
        assignee: a.executor,
        ..TaskPatch::default()
    };
    let t = ops::update_task(conn, env, &t.id, &patch, now)?;
    Ok(Outcome {
        value: task_value(&t, &key_of(conn, &t)?)?,
        changed: Some(t.project_id),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetRun {
    run_id: Option<String>,
    task: Option<String>,
}

/// A run and the result of its review. The review is its own run (`kind: review`, pointing
/// to the work run with `parentRunId`); a workflow that reviews itself leaves the verdict on
/// the work run.
fn get_run(conn: &Connection, a: GetRun) -> Result<Value, String> {
    let run = match (a.run_id.as_deref().map(str::trim), a.task.as_deref()) {
        (Some(id), None) => qruns::get(conn, id)?,
        (None, Some(task)) => {
            let t = resolve_task(conn, task)?;
            qruns::last_work(conn, &t.id)?
                .ok_or_else(|| format!("{} has no runs yet.", a.task.unwrap_or_default()))?
        }
        _ => return Err("Pass either `runId` or `task`.".into()),
    };
    let review = match (run.kind, &run.task_id) {
        (RunKind::Review, _) => Some(run.clone()),
        (RunKind::Work, Some(task_id)) => qruns::list_filtered(conn, None, Some(task_id))?
            .into_iter()
            .find(|r| {
                r.kind == RunKind::Review && r.parent_run_id.as_deref() == Some(run.id.as_str())
            }),
        (RunKind::Work, None) => None,
    }
    .or_else(|| run.verdict.is_some().then(|| run.clone()));
    let review = review.map(
        |r| json!({"runId": r.id, "status": r.status, "outcome": r.outcome, "verdict": r.verdict}),
    );
    let task_key = match &run.task_id {
        Some(id) => match tasks::get(conn, id) {
            Ok(t) => Some(Value::String(task_key(&key_of(conn, &t)?, t.number))),
            Err(_) => None,
        },
        None => None,
    };
    let mut out = Map::new();
    out.insert("run".into(), to_value(&RunLight::from(run))?);
    out.insert("taskKey".into(), task_key.unwrap_or(Value::Null));
    out.insert("review".into(), review.unwrap_or(Value::Null));
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests;
