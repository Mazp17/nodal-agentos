//! MCP tools: their schemas and what each one runs. Every call goes through the same `ops`
//! and queries as the Tauri commands in `work::commands`, so validation, numbering and
//! side effects (Linear outbox, positions, `closedAt`) match the UI.

use std::path::Path;

use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::db::queries::{double_option, projects, repos, runs as qruns, tasks};
use crate::domain::*;
use crate::util::{is_valid_id, paths};
use crate::work::dto::{NewTask, PlanInput, TaskPatch};
use crate::work::{ops, Env};

/// Result of a tool, plus the project whose tasks changed (for `nodal://changed`).
#[derive(Debug)]
pub struct Outcome {
    pub value: Value,
    pub changed: Option<String>,
}

impl Outcome {
    fn read(value: Value) -> Self {
        Outcome { value, changed: None }
    }
}

const RUNS_IN_TASK: usize = 10;

pub fn call(conn: &mut Connection, env: &Env, name: &str, args: Value, now: i64) -> Result<Outcome, String> {
    let args = if args.is_null() { json!({}) } else { args };
    match name {
        "list_projects" => list_projects(conn, parse(args)?).map(Outcome::read),
        "list_tasks" => list_tasks(conn, parse(args)?).map(Outcome::read),
        "get_task" => get_task(conn, env, parse(args)?).map(Outcome::read),
        "create_task" => create_task(conn, env, parse(args)?, now),
        "update_task" => update_task(conn, env, parse(args)?, now),
        "get_run" => get_run(conn, parse(args)?).map(Outcome::read),
        _ => Err(format!("Unknown tool: {name}.")),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("Invalid arguments: {e}."))
}

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("Internal error: {e}"))
}

// ---------- Resolución de proyecto, repo y tarea ----------

/// Id or key (`PAY`, any case).
fn resolve_project(conn: &Connection, s: &str) -> Result<Project, String> {
    let s = s.trim();
    projects::list(conn, true)?
        .into_iter()
        .find(|p| p.id == s || p.key.eq_ignore_ascii_case(s))
        .ok_or_else(|| format!("No project with id or key \"{s}\". Use list_projects to see them."))
}

/// Id, name, or a path at or inside the repo (the deepest repo wins).
fn resolve_repo(conn: &Connection, s: &str, project: Option<&Project>) -> Result<Repo, String> {
    let s = s.trim();
    let all = repos::list(conn, project.map(|p| p.id.as_str()))?;
    if let Some(r) = all.iter().find(|r| r.id == s) {
        return Ok(r.clone());
    }
    let by_name: Vec<&Repo> = all.iter().filter(|r| r.name.eq_ignore_ascii_case(s)).collect();
    match by_name.as_slice() {
        [r] => return Ok((*r).clone()),
        [_, _, ..] => return Err(format!("Several repos are named \"{s}\": pass `project` or the repo id.")),
        [] => {}
    }
    let path = paths::expand_home(s);
    let path = path.canonicalize().unwrap_or(path);
    all.iter()
        .filter(|r| path.starts_with(Path::new(&r.path)))
        .max_by_key(|r| r.path.len())
        .cloned()
        .ok_or_else(|| format!("No repo matches \"{s}\". Use list_projects to see the repos of each project."))
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
    Ok(projects::list(conn, true)?.into_iter().map(|p| (p.id, p.key)).collect())
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
    let project = a.project.as_deref().map(|p| resolve_project(conn, p)).transpose()?;
    let statuses = match a.status {
        Some(OneOrMany::One(s)) => vec![s],
        Some(OneOrMany::Many(v)) => v,
        None => vec![],
    };
    let keys = project_keys(conn)?;
    tasks::list(conn, project.as_ref().map(|p| p.id.as_str()))?
        .iter()
        .filter(|t| statuses.is_empty() || statuses.contains(&t.status))
        .map(|t| task_value(t, keys.get(&t.project_id).map(String::as_str).unwrap_or_default()))
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
fn plan_input(plan: Option<String>, plan_file: Option<String>) -> Result<Option<PlanInput>, String> {
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

fn create_task(conn: &mut Connection, env: &Env, a: CreateTask, now: i64) -> Result<Outcome, String> {
    let project = a.project.as_deref().map(|p| resolve_project(conn, p)).transpose()?;
    let repo = resolve_repo(conn, &a.repo, project.as_ref())?;
    let plan = plan_input(a.plan, a.plan_file)?.ok_or("A task needs a plan: pass `plan` (markdown) or `planFile`.")?;
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
    Ok(Outcome { value: task_value(&t, &key_of(conn, &t)?)?, changed: Some(t.project_id) })
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

fn update_task(conn: &mut Connection, env: &Env, a: UpdateTask, now: i64) -> Result<Outcome, String> {
    let t = resolve_task(conn, &a.task)?;
    let repo_id = match a.repo.as_deref() {
        Some(r) => Some(resolve_repo(conn, r, Some(&projects::get(conn, &t.project_id)?))?.id),
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
    Ok(Outcome { value: task_value(&t, &key_of(conn, &t)?)?, changed: Some(t.project_id) })
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
            qruns::last_work(conn, &t.id)?.ok_or_else(|| format!("{} has no runs yet.", a.task.unwrap_or_default()))?
        }
        _ => return Err("Pass either `runId` or `task`.".into()),
    };
    let review = match (run.kind, &run.task_id) {
        (RunKind::Review, _) => Some(run.clone()),
        (RunKind::Work, Some(task_id)) => qruns::list_filtered(conn, None, Some(task_id))?
            .into_iter()
            .find(|r| r.kind == RunKind::Review && r.parent_run_id.as_deref() == Some(run.id.as_str())),
        (RunKind::Work, None) => None,
    }
    .or_else(|| run.verdict.is_some().then(|| run.clone()));
    let review = review.map(|r| json!({"runId": r.id, "status": r.status, "outcome": r.outcome, "verdict": r.verdict}));
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

// ---------- Schemas ----------

fn executor_schema() -> Value {
    json!({
        "type": "object",
        "description": "Who runs the task. {\"kind\":\"claude\"}, {\"kind\":\"agent\",\"name\":\"<agent>\",\"source\":\"user|repo|plugin\"} or {\"kind\":\"workflow\",\"name\":\"<workflow>\"}. Omit to use the repo's default.",
        "properties": {
            "kind": {"type": "string", "enum": ["claude", "agent", "workflow"]},
            "name": {"type": "string"},
            "source": {"type": "string", "enum": ["user", "repo", "plugin"]}
        },
        "required": ["kind"]
    })
}

const STATUSES: [&str; 7] = ["backlog", "todo", "in_progress", "in_review", "blocked", "done", "canceled"];
const PRIORITIES: [&str; 5] = ["urgent", "high", "medium", "low", "none"];

/// `tools/list`: name, description and JSON schema of each tool.
pub fn definitions() -> Value {
    let status = json!({"type": "string", "enum": STATUSES});
    let priority = json!({"type": "string", "enum": PRIORITIES});
    let task_ref = json!({"type": "string", "description": "Task key (PAY-12) or internal id."});
    let strings = json!({"type": "array", "items": {"type": "string"}});
    json!([
        {
            "name": "list_projects",
            "description": "Nodal projects with their repos (id, name, path). Every task belongs to one repo of one project.",
            "inputSchema": {"type": "object", "properties": {
                "includeArchived": {"type": "boolean", "description": "Also list archived projects."}
            }}
        },
        {
            "name": "list_tasks",
            "description": "Tasks on the board, optionally for one project and some statuses. Each task has its visible key (PAY-12).",
            "inputSchema": {"type": "object", "properties": {
                "project": {"type": "string", "description": "Project key (PAY) or id."},
                "status": {"description": "One status or a list.", "anyOf": [status.clone(), {"type": "array", "items": status.clone()}]}
            }}
        },
        {
            "name": "get_task",
            "description": "One task with its plan text and its latest runs (newest first).",
            "inputSchema": {"type": "object", "properties": {"task": task_ref.clone()}, "required": ["task"]}
        },
        {
            "name": "create_task",
            "description": "Create a task on the board (it does not launch it). Needs a repo, a title and a plan.",
            "inputSchema": {"type": "object", "properties": {
                "repo": {"type": "string", "description": "Repo id, name, or a path at or inside the repo."},
                "project": {"type": "string", "description": "Project key or id, when several repos share a name."},
                "title": {"type": "string", "description": "One line, up to 200 characters."},
                "plan": {"type": "string", "description": "The plan, in markdown."},
                "planFile": {"type": "string", "description": "Instead of `plan`: a .md file inside the repo (absolute or relative to the repo)."},
                "acceptance": {"type": "array", "items": {"type": "string"}, "description": "Acceptance criteria the review checks."},
                "priority": priority.clone(),
                "labels": strings.clone(),
                "executor": executor_schema(),
                "status": {"type": "string", "enum": STATUSES, "description": "Defaults to todo."}
            }, "required": ["repo", "title"]}
        },
        {
            "name": "update_task",
            "description": "Change a task's status or fields; omitted fields stay as they are. Tasks imported from Linear only accept status, plan, acceptance, executor and repo. Setting in_progress does not launch a run.",
            "inputSchema": {"type": "object", "properties": {
                "task": task_ref.clone(),
                "status": status,
                "title": {"type": "string"},
                "plan": {"type": "string", "description": "New plan, in markdown."},
                "planFile": {"type": "string", "description": "Instead of `plan`: a .md file inside the repo."},
                "acceptance": {"type": "array", "items": {"type": "string"}},
                "priority": priority,
                "labels": strings,
                "executor": {"anyOf": [executor_schema(), {"type": "null"}], "description": "null goes back to the repo's default."},
                "repo": {"type": "string", "description": "Move to another repo of the same project (id, name or path)."}
            }, "required": ["task"]}
        },
        {
            "name": "get_run",
            "description": "A run's status, outcome, summary, PR and branch, and its review result (verdict with unmet criteria). Pass a run id, or a task to get its latest work run.",
            "inputSchema": {"type": "object", "properties": {
                "runId": {"type": "string"},
                "task": task_ref
            }}
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::db::queries::runs as qruns;
    use crate::util::paths::tests::TempDir;
    use crate::work::dto::{NewProject, NewRepo};
    use crate::work::testutil::run_of;

    struct Fx {
        t: TempDir,
        env: Env,
    }

    fn fx(name: &str) -> Fx {
        let t = TempDir::new(name);
        let env = Env { data_dir: t.0.join("data"), worktrees_root: t.0.join("wt"), claude_dir: None };
        Fx { t, env }
    }

    /// Project PAY with repos `web` and `api`.
    fn seed(c: &Connection, f: &Fx) -> (Project, Repo, Repo) {
        let p = ops::create_project(c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None, description: None }, 1).unwrap();
        let add = |name: &str| {
            let dir = f.t.0.join(name);
            std::fs::create_dir_all(dir.join("docs")).unwrap();
            std::fs::write(dir.join("docs/plan.md"), "# Plan from the repo").unwrap();
            ops::add_repo(c, &p.id, &NewRepo::default(), &dir, 2).unwrap()
        };
        let (web, api) = (add("web"), add("api"));
        (p, web, api)
    }

    fn call_ok(c: &mut Connection, f: &Fx, name: &str, args: Value) -> Outcome {
        call(c, &f.env, name, args, 10).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    #[test]
    fn create_task_numbers_validates_and_reports_the_project() {
        let f = fx("mcp-create");
        let db = open_in_memory().unwrap();
        let mut c = db.lock().unwrap();
        let (p, web, api) = seed(&c, &f);

        let out = call_ok(&mut c, &f, "create_task", json!({
            "repo": "web", "title": "Add a logo", "plan": "# Plan\nDo it",
            "acceptance": ["The logo shows"], "priority": "high", "labels": ["ui"],
            "executor": {"kind": "workflow", "name": "ship"}
        }));
        assert_eq!(out.changed.as_deref(), Some(p.id.as_str()));
        assert_eq!(out.value["key"], "PAY-1");
        assert_eq!(out.value["repoId"], web.id.as_str());
        assert_eq!(out.value["status"], "todo");
        assert_eq!(out.value["priority"], "high");
        assert_eq!(out.value["acceptance"], json!(["The logo shows"]));
        assert_eq!(out.value["assignee"], json!({"kind": "workflow", "name": "ship"}));
        let id = out.value["id"].as_str().unwrap().to_string();
        assert_eq!(ops::read_task_plan(&c, &f.env, &id).unwrap(), "# Plan\nDo it");

        // Repo by path (inside the repo), plan from a file of the repo; numbering continues.
        let inside = f.t.0.join("api/docs").to_string_lossy().into_owned();
        let out = call_ok(&mut c, &f, "create_task", json!({"repo": inside, "title": "Second", "planFile": "docs/plan.md", "status": "backlog"}));
        assert_eq!((out.value["key"].as_str(), out.value["repoId"].as_str()), (Some("PAY-2"), Some(api.id.as_str())));
        assert_eq!(out.value["status"], "backlog");

        // Same validation as the UI.
        let err = |c: &mut Connection, args: Value| call(c, &f.env, "create_task", args, 10).unwrap_err();
        assert!(err(&mut c, json!({"repo": "web", "title": "x".repeat(201), "plan": "x"})).contains("too long"));
        assert!(err(&mut c, json!({"repo": "web", "title": "x", "plan": " "})).contains("plan"));
        assert!(err(&mut c, json!({"repo": "web", "title": "x"})).contains("needs a plan"));
        assert!(err(&mut c, json!({"repo": "web", "title": "x", "plan": "x", "planFile": "y.md"})).contains("not both"));
        assert!(err(&mut c, json!({"repo": "nope", "title": "x", "plan": "x"})).contains("No repo matches"));
        assert!(err(&mut c, json!({"repo": "web", "title": "x", "plan": "x", "project": "ZZZ"})).contains("No project"));
        assert!(err(&mut c, json!({"repo": "web", "title": "x", "plan": "x", "bogus": 1})).contains("Invalid arguments"));
        assert!(err(&mut c, json!({"repo": "web", "title": "x", "plan": "x", "priority": "asap"})).contains("Invalid arguments"));
        assert_eq!(projects::get(&c, &p.id).unwrap().next_task_number, 3);
    }

    #[test]
    fn list_get_and_update_tasks_by_key() {
        let f = fx("mcp-update");
        let db = open_in_memory().unwrap();
        let mut c = db.lock().unwrap();
        let (p, _, api) = seed(&c, &f);
        for title in ["One", "Two"] {
            call_ok(&mut c, &f, "create_task", json!({"repo": "web", "title": title, "plan": "x"}));
        }

        let projects = call_ok(&mut c, &f, "list_projects", Value::Null).value;
        assert_eq!(projects[0]["key"], "PAY");
        assert_eq!(projects[0]["repos"].as_array().unwrap().len(), 2);

        let out = call_ok(&mut c, &f, "update_task", json!({"task": "pay-2", "status": "done", "labels": ["later"], "repo": "api"}));
        assert_eq!(out.changed.as_deref(), Some(p.id.as_str()));
        assert_eq!((out.value["status"].as_str(), out.value["repoId"].as_str()), (Some("done"), Some(api.id.as_str())));
        assert!(out.value["closedAt"].is_i64());

        let done = call_ok(&mut c, &f, "list_tasks", json!({"project": "PAY", "status": "done"})).value;
        assert_eq!(done.as_array().unwrap().iter().map(|t| t["key"].as_str().unwrap()).collect::<Vec<_>>(), ["PAY-2"]);
        let open = call_ok(&mut c, &f, "list_tasks", json!({"status": ["todo", "backlog"]})).value;
        assert_eq!(open.as_array().unwrap().len(), 1);
        assert_eq!(call_ok(&mut c, &f, "list_tasks", json!({})).value.as_array().unwrap().len(), 2);

        let t = call_ok(&mut c, &f, "get_task", json!({"task": "PAY-1"})).value;
        assert_eq!((t["planText"].as_str(), t["runs"].as_array().map(Vec::len)), (Some("x"), Some(0)));
        let id = t["id"].as_str().unwrap().to_string();
        assert_eq!(call_ok(&mut c, &f, "get_task", json!({"task": id})).value["key"], "PAY-1");

        // `executor: null` clears; omitted fields stay.
        call_ok(&mut c, &f, "update_task", json!({"task": "PAY-1", "executor": {"kind": "claude"}}));
        let t = call_ok(&mut c, &f, "update_task", json!({"task": "PAY-1", "executor": null})).value;
        assert!(t["assignee"].is_null());
        assert_eq!(t["title"], "One");

        let err = |c: &mut Connection, name: &str, args: Value| call(c, &f.env, name, args, 10).unwrap_err();
        assert!(err(&mut c, "update_task", json!({"task": "PAY-9", "status": "done"})).contains("No task"));
        assert!(err(&mut c, "update_task", json!({"task": "PAY-1", "title": ""})).contains("title"));
        assert!(err(&mut c, "get_task", json!({"task": "../etc"})).contains("No task"));
        assert!(err(&mut c, "launch_task", json!({})).contains("Unknown tool"));
    }

    #[test]
    fn get_run_reports_the_review_result() {
        let f = fx("mcp-run");
        let db = open_in_memory().unwrap();
        let mut c = db.lock().unwrap();
        seed(&c, &f);
        let t = call_ok(&mut c, &f, "create_task", json!({"repo": "web", "title": "One", "plan": "x"})).value;
        let task_id = t["id"].as_str().unwrap().to_string();

        let err = call(&mut c, &f.env, "get_run", json!({"task": "PAY-1"}), 10).unwrap_err();
        assert!(err.contains("no runs"), "{err}");

        let mut work = run_of(Executor::Claude, RunKind::Work, true);
        work.id = "run-work".into();
        work.task_id = Some(task_id.clone());
        work.repo_id = None;
        work.status = RunStatus::Finished;
        work.outcome = Some(RunOutcome::Green);
        work.summary = Some("Added the logo".into());
        qruns::insert(&c, &work).unwrap();

        let out = call_ok(&mut c, &f, "get_run", json!({"task": "PAY-1"})).value;
        assert_eq!((out["run"]["id"].as_str(), out["taskKey"].as_str()), (Some("run-work"), Some("PAY-1")));
        assert_eq!((out["run"]["outcome"].as_str(), out["run"]["summary"].as_str()), (Some("green"), Some("Added the logo")));
        assert!(out["run"].get("prompt").is_none());
        assert!(out["review"].is_null());

        let mut review = run_of(Executor::Claude, RunKind::Review, false);
        review.id = "run-review".into();
        review.task_id = Some(task_id);
        review.repo_id = None;
        review.parent_run_id = Some("run-work".into());
        review.queued_at = 5;
        review.status = RunStatus::Finished;
        review.verdict = Some(Verdict { pass: false, unmet: vec!["The logo shows".into()], nits: vec![], summary: Some("Missing".into()) });
        qruns::insert(&c, &review).unwrap();

        let out = call_ok(&mut c, &f, "get_run", json!({"runId": "run-work"})).value;
        assert_eq!(out["review"]["runId"], "run-review");
        assert_eq!(out["review"]["verdict"]["pass"], false);
        assert_eq!(out["review"]["verdict"]["unmet"], json!(["The logo shows"]));
        // The last work run of the task is still the work run, not its review.
        assert_eq!(call_ok(&mut c, &f, "get_run", json!({"task": "PAY-1"})).value["run"]["id"], "run-work");
        assert_eq!(call_ok(&mut c, &f, "get_run", json!({"runId": "run-review"})).value["review"]["runId"], "run-review");
        assert!(call(&mut c, &f.env, "get_run", json!({}), 10).unwrap_err().contains("either"));
    }

    #[test]
    fn definitions_cover_every_tool_and_nothing_launches() {
        let defs = definitions();
        let names: Vec<&str> = defs.as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["list_projects", "list_tasks", "get_task", "create_task", "update_task", "get_run"]);
        for d in defs.as_array().unwrap() {
            assert_eq!(d["inputSchema"]["type"], "object");
            assert!(!d["description"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn the_skill_describes_every_tool_and_value() {
        let skill = include_str!("../../../skills/nodal-tasks/SKILL.md");
        assert!(skill.starts_with("---\nname: nodal-tasks\ndescription: "));
        let defs = definitions();
        let tools = defs.as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap());
        for word in tools.chain(STATUSES).chain(PRIORITIES).chain(["planFile", "executor", "verdict"]) {
            assert!(skill.contains(&format!("`{word}`")), "SKILL.md does not mention `{word}`");
        }
    }
}
