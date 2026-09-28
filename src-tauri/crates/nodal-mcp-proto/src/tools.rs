//! MCP tool schemas: what `tools/list` advertises to agents. Dispatch (running a call) is
//! `nodal_lib::mcp::tools::call`, which stays in the shell until it moves to
//! `app::agent_api::tools`.
use serde_json::{json, Value};

/// Chat-only tool: a task the chat shows as a card for the user to create. Nothing is written.
pub const PROPOSE_TASK: &str = "propose_task";

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

/// Public (not per the frozen surface) so the old crate's test file, which stays in place until
/// the dispatch code moves too, can keep using them via a bridge import.
pub const STATUSES: [&str; 7] = [
    "backlog",
    "todo",
    "in_progress",
    "in_review",
    "blocked",
    "done",
    "canceled",
];
pub const PRIORITIES: [&str; 5] = ["urgent", "high", "medium", "low", "none"];

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

/// `tools/list` of a Nodal chat (`nodal-mcp --chat`): every tool plus `propose_task`, which
/// only makes sense where the chat UI renders it.
pub fn chat_definitions() -> Value {
    let mut defs = definitions();
    if let Value::Array(list) = &mut defs {
        list.push(json!({
            "name": PROPOSE_TASK,
            "description": "Propose a task: the user sees it as a task card in this chat and creates it with one click. It does not create anything. Prefer it to create_task unless the user asks you to create the task yourself.",
            "inputSchema": {"type": "object", "properties": {
                "repo": {"type": "string", "description": "Repo id, name, or a path at or inside the repo."},
                "project": {"type": "string", "description": "Project key or id, when several repos share a name."},
                "title": {"type": "string", "description": "One line, up to 200 characters."},
                "plan": {"type": "string", "description": "The plan, in markdown."},
                "acceptance": {"type": "array", "items": {"type": "string"}, "description": "Acceptance criteria the review checks."},
                "priority": {"type": "string", "enum": PRIORITIES},
                "labels": {"type": "array", "items": {"type": "string"}},
                "executor": executor_schema(),
                "status": {"type": "string", "enum": STATUSES, "description": "Defaults to todo."}
            }, "required": ["repo", "title", "plan"]}
        }));
    }
    defs
}
