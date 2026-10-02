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
pub const RUN_STATUSES: [&str; 6] = ["queued", "launching", "launched", "finished", "failed", "canceled"];
/// Mirrors `nodal_domain::execution::options` (this crate doesn't depend on nodal-domain);
/// a nodal-app test checks they match.
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
pub const PERMISSION_MODES: [&str; 7] = [
    "default",
    "manual",
    "acceptEdits",
    "auto",
    "dontAsk",
    "plan",
    "bypassPermissions",
];

/// `tools/list`: name, description and JSON schema of each tool.
pub fn definitions() -> Value {
    let status = json!({"type": "string", "enum": STATUSES});
    let priority = json!({"type": "string", "enum": PRIORITIES});
    let run_status = json!({"type": "string", "enum": RUN_STATUSES});
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
                "task": task_ref.clone()
            }}
        },
        {
            "name": "list_runs",
            "description": "Runs, newest first, optionally for one project or task and some statuses. Finished ones come from the last 500 runs. Each run is listed without its prompt; get_run has the details.",
            "inputSchema": {"type": "object", "properties": {
                "project": {"type": "string", "description": "Project key (PAY) or id."},
                "task": task_ref.clone(),
                "status": {"description": "One status or a list.", "anyOf": [run_status.clone(), {"type": "array", "items": run_status}]},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Defaults to 20."}
            }}
        },
        {
            "name": "list_executors",
            "description": "Agents and workflows a repo's tasks can run with (the `executor` of launch_run, create_task and update_task), plus the accepted models, efforts and permission modes.",
            "inputSchema": {"type": "object", "properties": {
                "repo": {"type": "string", "description": "Repo id, name, or a path at or inside the repo. Omit for the user's own agents and workflows only."},
                "project": {"type": "string", "description": "Project key or id, when several repos share a name."}
            }}
        },
        {
            "name": "launch_run",
            "description": "Queue a work run for a task; Nodal launches it when there is a free slot. Omitted options fall back to the task's, then the repo's. Fails if the task already has a queued or running run. Follow it with get_run.",
            "inputSchema": {"type": "object", "properties": {
                "task": task_ref.clone(),
                "executor": executor_schema(),
                "model": {"type": "string", "description": "fable, opus, sonnet, haiku, or a full name like claude-sonnet-5-5 (optionally with [1m])."},
                "effort": {"type": "string", "enum": EFFORTS},
                "permissionMode": {"type": "string", "enum": PERMISSION_MODES},
                "isolation": {"type": "string", "enum": ["worktree", "in_place"], "description": "Ignored for workflows, which manage their own."},
                "finish": {"type": "string", "enum": ["changes", "commit", "pr"], "description": "Leave uncommitted changes, commit, or commit, push and open a PR."},
                "review": {"type": "boolean", "description": "Queue a review run when the work finishes."},
                "extraInstructions": {"type": "string", "description": "Added to the prompt for this run only."}
            }, "required": ["task"]}
        },
        {
            "name": "review_task",
            "description": "Queue a review run for a task's current work (its worktree or repo).",
            "inputSchema": {"type": "object", "properties": {
                "task": task_ref.clone(),
                "reviewer": {"type": "string", "description": "Reviewer agent; defaults to the repo's, then the project's."}
            }, "required": ["task"]}
        },
        {
            "name": "cancel_run",
            "description": "Dequeue a queued run, or stop a running one: what it left half-done is saved as a patch and the task moves to blocked.",
            "inputSchema": {"type": "object", "properties": {"runId": {"type": "string"}}, "required": ["runId"]}
        },
        {
            "name": "get_queue",
            "description": "The run queue: occupied slots and capacity, how many are queued, what waits on the user, the last queue error, and the queued and running runs in order.",
            "inputSchema": {"type": "object", "properties": {
                "project": {"type": "string", "description": "Project key or id; omit for everything."}
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
