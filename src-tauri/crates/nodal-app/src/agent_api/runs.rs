//! MCP tools that drive the run queue (`launch_run`, `review_task`, `cancel_run`, `get_queue`).
//! Unlike `tools::call`, they go through the async `Execution` facade, the same as the Tauri
//! commands, so the queue pass, notifications and checks match the UI.

use serde::Deserialize;
use serde_json::Value;

use nodal_domain::board::dto::LaunchInput;
use nodal_domain::model::*;
use nodal_store::execution::runs as qruns;

use super::tools::{parse, resolve_project, resolve_task, run_values, to_value};
use super::AgentApi;

pub(super) const TOOLS: [&str; 4] = ["launch_run", "review_task", "cancel_run", "get_queue"];

pub(super) fn call(api: &AgentApi, tool: &str, args: Value) -> Result<Value, String> {
    match tool {
        "launch_run" => launch_run(api, parse(args)?),
        "review_task" => review_task(api, parse(args)?),
        "cancel_run" => cancel_run(api, parse(args)?),
        "get_queue" => get_queue(api, parse(args)?),
        _ => Err(format!("Unknown tool: {tool}.")),
    }
}

/// The run as `list_runs` shows it.
fn run_value(api: &AgentApi, run: Run) -> Result<Value, String> {
    let conn = api.core.db.guard();
    let mut list = run_values(&conn, vec![RunLight::from(run)])?;
    Ok(list
        .as_array_mut()
        .and_then(|l| l.pop())
        .unwrap_or(Value::Null))
}

fn task_id(api: &AgentApi, task: &str) -> Result<String, String> {
    Ok(resolve_task(&api.core.db.guard(), task)?.id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaunchRun {
    task: String,
    executor: Option<Executor>,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    isolation: Option<Isolation>,
    finish: Option<Finish>,
    review: Option<bool>,
    extra_instructions: Option<String>,
}

fn launch_run(api: &AgentApi, a: LaunchRun) -> Result<Value, String> {
    let task_id = task_id(api, &a.task)?;
    let options = LaunchOptions {
        model: a.model,
        effort: a.effort,
        permission_mode: a.permission_mode,
    };
    let input = LaunchInput {
        executor: a.executor,
        isolation: a.isolation,
        finish: a.finish,
        review: a.review,
        extra_instructions: a.extra_instructions,
        options: Some(options),
    };
    let run = api
        .block_on(api.execution.launch_task(task_id, Some(input)))?
        .map_err(|e| e.to_string())?;
    run_value(api, run)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReviewTask {
    task: String,
    reviewer: Option<String>,
}

fn review_task(api: &AgentApi, a: ReviewTask) -> Result<Value, String> {
    let task_id = task_id(api, &a.task)?;
    let run = api
        .block_on(api.execution.review_now(task_id, a.reviewer))?
        .map_err(|e| e.to_string())?;
    run_value(api, run)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CancelRun {
    run_id: String,
}

fn cancel_run(api: &AgentApi, a: CancelRun) -> Result<Value, String> {
    let run = api
        .block_on(api.execution.cancel_run(a.run_id.trim().to_string()))?
        .map_err(|e| e.to_string())?;
    run_value(api, run)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetQueue {
    project: Option<String>,
}

/// `work_summary` plus the pending runs (queued, launching and running) in queue order.
fn get_queue(api: &AgentApi, a: GetQueue) -> Result<Value, String> {
    let project_id = match a.project.as_deref() {
        Some(p) => Some(resolve_project(&api.core.db.guard(), p)?.id),
        None => None,
    };
    let summary = api
        .block_on(api.execution.work_summary(project_id.clone()))?
        .map_err(|e| e.to_string())?;
    let conn = api.core.db.guard();
    let runs = qruns::pending_of(&conn, project_id.as_deref())?
        .into_iter()
        .map(RunLight::from)
        .collect();
    let mut out = to_value(&summary)?;
    if let Value::Object(m) = &mut out {
        m.insert("runs".into(), run_values(&conn, runs)?);
    }
    Ok(out)
}
