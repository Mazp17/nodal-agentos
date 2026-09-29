#![allow(clippy::disallowed_methods)] // builds fixture folders/files directly on disk

use std::path::PathBuf;
use std::sync::Arc;

use nodal_domain::board::dto::{NewProject, NewRepo};
use nodal_domain::testutil::run_of;
use nodal_host::adapters::{HostClaudeConfig, HostGit, HostLocalFs, HostPlanFiles};
use nodal_host::testutil::TempDir;
use nodal_store::execution::runs as qruns;
use nodal_store::Db;
use serde_json::json;

use super::*;

/// `Env` for tests: real (blocking, synchronous) host adapters rooted at `data_dir`/
/// `worktrees_root`, so callers keep their own `TempDir` and path names. Local copy of the
/// old `work::test_env` helper (agent_api owns its own tests; it doesn't reach into
/// `nodal_app::testutil`).
fn test_env(data_dir: PathBuf, worktrees_root: PathBuf, claude_dir: Option<PathBuf>) -> Env {
    Env {
        data_dir,
        worktrees_root,
        claude_dir,
        plans: Arc::new(HostPlanFiles),
        fs: Arc::new(HostLocalFs),
        git: Arc::new(HostGit),
        claude_config: Arc::new(HostClaudeConfig),
    }
}

struct Fx {
    t: TempDir,
    env: Env,
}

fn fx(name: &str) -> Fx {
    let t = TempDir::new(name);
    let env = test_env(t.0.join("data"), t.0.join("wt"), None);
    Fx { t, env }
}

/// Project PAY with repos `web` and `api`.
fn seed(c: &Connection, f: &Fx) -> (Project, Repo, Repo) {
    let p = ops::create_project(
        c,
        &f.env,
        &NewProject {
            name: "Pay".into(),
            key: "PAY".into(),
            color: None,
            description: None,
            root_path: None,
        },
        1,
    )
    .unwrap();
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
    let db = Db::open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    let (p, web, api) = seed(&c, &f);

    let out = call_ok(
        &mut c,
        &f,
        "create_task",
        json!({
            "repo": "web", "title": "Add a logo", "plan": "# Plan\nDo it",
            "acceptance": ["The logo shows"], "priority": "high", "labels": ["ui"],
            "executor": {"kind": "workflow", "name": "ship"}
        }),
    );
    assert_eq!(out.changed.as_deref(), Some(p.id.as_str()));
    assert_eq!(out.value["key"], "PAY-1");
    assert_eq!(out.value["repoId"], web.id.as_str());
    assert_eq!(out.value["status"], "todo");
    assert_eq!(out.value["priority"], "high");
    assert_eq!(out.value["acceptance"], json!(["The logo shows"]));
    assert_eq!(
        out.value["assignee"],
        json!({"kind": "workflow", "name": "ship"})
    );
    let id = out.value["id"].as_str().unwrap().to_string();
    assert_eq!(
        ops::read_task_plan(&c, &f.env, &id).unwrap(),
        "# Plan\nDo it"
    );

    // Repo by path (inside the repo), plan from a file of the repo; numbering continues.
    let inside = f.t.0.join("api/docs").to_string_lossy().into_owned();
    let out = call_ok(
        &mut c,
        &f,
        "create_task",
        json!({"repo": inside, "title": "Second", "planFile": "docs/plan.md", "status": "backlog"}),
    );
    assert_eq!(
        (out.value["key"].as_str(), out.value["repoId"].as_str()),
        (Some("PAY-2"), Some(api.id.as_str()))
    );
    assert_eq!(out.value["status"], "backlog");

    // Same validation as the UI.
    let err =
        |c: &mut Connection, args: Value| call(c, &f.env, "create_task", args, 10).unwrap_err();
    assert!(err(
        &mut c,
        json!({"repo": "web", "title": "x".repeat(201), "plan": "x"})
    )
    .contains("too long"));
    assert!(err(&mut c, json!({"repo": "web", "title": "x", "plan": " "})).contains("plan"));
    assert!(err(&mut c, json!({"repo": "web", "title": "x"})).contains("needs a plan"));
    assert!(err(
        &mut c,
        json!({"repo": "web", "title": "x", "plan": "x", "planFile": "y.md"})
    )
    .contains("not both"));
    assert!(
        err(&mut c, json!({"repo": "nope", "title": "x", "plan": "x"})).contains("No repo matches")
    );
    assert!(err(
        &mut c,
        json!({"repo": "web", "title": "x", "plan": "x", "project": "ZZZ"})
    )
    .contains("No project"));
    assert!(err(
        &mut c,
        json!({"repo": "web", "title": "x", "plan": "x", "bogus": 1})
    )
    .contains("Invalid arguments"));
    assert!(err(
        &mut c,
        json!({"repo": "web", "title": "x", "plan": "x", "priority": "asap"})
    )
    .contains("Invalid arguments"));
    assert_eq!(projects::get(&c, &p.id).unwrap().next_task_number, 3);
}

#[test]
fn list_get_and_update_tasks_by_key() {
    let f = fx("mcp-update");
    let db = Db::open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    let (p, _, api) = seed(&c, &f);
    for title in ["One", "Two"] {
        call_ok(
            &mut c,
            &f,
            "create_task",
            json!({"repo": "web", "title": title, "plan": "x"}),
        );
    }

    let projects = call_ok(&mut c, &f, "list_projects", Value::Null).value;
    assert_eq!(projects[0]["key"], "PAY");
    assert_eq!(projects[0]["repos"].as_array().unwrap().len(), 2);

    let out = call_ok(
        &mut c,
        &f,
        "update_task",
        json!({"task": "pay-2", "status": "done", "labels": ["later"], "repo": "api"}),
    );
    assert_eq!(out.changed.as_deref(), Some(p.id.as_str()));
    assert_eq!(
        (out.value["status"].as_str(), out.value["repoId"].as_str()),
        (Some("done"), Some(api.id.as_str()))
    );
    assert!(out.value["closedAt"].is_i64());

    let done = call_ok(
        &mut c,
        &f,
        "list_tasks",
        json!({"project": "PAY", "status": "done"}),
    )
    .value;
    assert_eq!(
        done.as_array()
            .unwrap()
            .iter()
            .map(|t| t["key"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["PAY-2"]
    );
    let open = call_ok(
        &mut c,
        &f,
        "list_tasks",
        json!({"status": ["todo", "backlog"]}),
    )
    .value;
    assert_eq!(open.as_array().unwrap().len(), 1);
    assert_eq!(
        call_ok(&mut c, &f, "list_tasks", json!({}))
            .value
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let t = call_ok(&mut c, &f, "get_task", json!({"task": "PAY-1"})).value;
    assert_eq!(
        (t["planText"].as_str(), t["runs"].as_array().map(Vec::len)),
        (Some("x"), Some(0))
    );
    let id = t["id"].as_str().unwrap().to_string();
    assert_eq!(
        call_ok(&mut c, &f, "get_task", json!({"task": id})).value["key"],
        "PAY-1"
    );

    // `executor: null` clears; omitted fields stay.
    call_ok(
        &mut c,
        &f,
        "update_task",
        json!({"task": "PAY-1", "executor": {"kind": "claude"}}),
    );
    let t = call_ok(
        &mut c,
        &f,
        "update_task",
        json!({"task": "PAY-1", "executor": null}),
    )
    .value;
    assert!(t["assignee"].is_null());
    assert_eq!(t["title"], "One");

    let err =
        |c: &mut Connection, name: &str, args: Value| call(c, &f.env, name, args, 10).unwrap_err();
    assert!(err(
        &mut c,
        "update_task",
        json!({"task": "PAY-9", "status": "done"})
    )
    .contains("No task"));
    assert!(err(&mut c, "update_task", json!({"task": "PAY-1", "title": ""})).contains("title"));
    assert!(err(&mut c, "get_task", json!({"task": "../etc"})).contains("No task"));
    assert!(err(&mut c, "launch_task", json!({})).contains("Unknown tool"));
}

#[test]
fn get_run_reports_the_review_result() {
    let f = fx("mcp-run");
    let db = Db::open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    seed(&c, &f);
    let t = call_ok(
        &mut c,
        &f,
        "create_task",
        json!({"repo": "web", "title": "One", "plan": "x"}),
    )
    .value;
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
    assert_eq!(
        (out["run"]["id"].as_str(), out["taskKey"].as_str()),
        (Some("run-work"), Some("PAY-1"))
    );
    assert_eq!(
        (
            out["run"]["outcome"].as_str(),
            out["run"]["summary"].as_str()
        ),
        (Some("green"), Some("Added the logo"))
    );
    assert!(out["run"].get("prompt").is_none());
    assert!(out["review"].is_null());

    let mut review = run_of(Executor::Claude, RunKind::Review, false);
    review.id = "run-review".into();
    review.task_id = Some(task_id);
    review.repo_id = None;
    review.parent_run_id = Some("run-work".into());
    review.queued_at = 5;
    review.status = RunStatus::Finished;
    review.verdict = Some(Verdict {
        pass: false,
        unmet: vec!["The logo shows".into()],
        nits: vec![],
        summary: Some("Missing".into()),
    });
    qruns::insert(&c, &review).unwrap();

    let out = call_ok(&mut c, &f, "get_run", json!({"runId": "run-work"})).value;
    assert_eq!(out["review"]["runId"], "run-review");
    assert_eq!(out["review"]["verdict"]["pass"], false);
    assert_eq!(out["review"]["verdict"]["unmet"], json!(["The logo shows"]));
    // The last work run of the task is still the work run, not its review.
    assert_eq!(
        call_ok(&mut c, &f, "get_run", json!({"task": "PAY-1"})).value["run"]["id"],
        "run-work"
    );
    assert_eq!(
        call_ok(&mut c, &f, "get_run", json!({"runId": "run-review"})).value["review"]["runId"],
        "run-review"
    );
    assert!(call(&mut c, &f.env, "get_run", json!({}), 10)
        .unwrap_err()
        .contains("either"));
}

#[test]
fn definitions_cover_every_tool_and_nothing_launches() {
    let defs = definitions();
    let names: Vec<&str> = defs
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_projects",
            "list_tasks",
            "get_task",
            "create_task",
            "update_task",
            "get_run"
        ]
    );
    for d in defs.as_array().unwrap() {
        assert_eq!(d["inputSchema"]["type"], "object");
        assert!(!d["description"].as_str().unwrap().is_empty());
    }
}

#[test]
fn propose_task_validates_and_creates_nothing() {
    let f = fx("mcp-propose");
    let db = Db::open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    let (p, web, _) = seed(&c, &f);

    let out = call_ok(
        &mut c,
        &f,
        "propose_task",
        json!({
            "repo": "web", "title": "  Add a\n logo ", "plan": "# Plan\nDo it",
            "acceptance": ["The logo shows", " "], "priority": "high", "labels": ["ui"],
            "executor": {"kind": "claude"}
        }),
    );
    assert_eq!(out.changed, None);
    assert_eq!(out.value["created"], false);
    assert_eq!(
        (
            out.value["projectKey"].as_str(),
            out.value["repoName"].as_str()
        ),
        (Some("PAY"), Some(web.name.as_str()))
    );
    let t = &out.value["newTask"];
    assert_eq!(
        (t["projectId"].as_str(), t["repoId"].as_str()),
        (Some(p.id.as_str()), Some(web.id.as_str()))
    );
    assert_eq!(
        (
            t["title"].as_str(),
            t["status"].as_str(),
            t["priority"].as_str()
        ),
        (Some("Add a logo"), Some("todo"), Some("high"))
    );
    assert_eq!(t["plan"], json!({"kind": "text", "text": "# Plan\nDo it"}));
    assert_eq!(t["acceptance"], json!(["The logo shows"]));
    assert_eq!(t["assignee"], json!({"kind": "claude"}));
    let input: NewTask = serde_json::from_value(t.clone()).expect("the card creates it as is");
    assert_eq!(
        input.plan,
        PlanInput::Text {
            text: "# Plan\nDo it".into()
        }
    );

    assert!(tasks::list(&c, Some(&p.id)).unwrap().is_empty());
    assert_eq!(projects::get(&c, &p.id).unwrap().next_task_number, 1);

    let err =
        |c: &mut Connection, args: Value| call(c, &f.env, "propose_task", args, 10).unwrap_err();
    assert!(err(
        &mut c,
        json!({"repo": "web", "title": "x".repeat(201), "plan": "x"})
    )
    .contains("too long"));
    assert!(err(&mut c, json!({"repo": "web", "title": "x", "plan": " "})).contains("plan"));
    assert!(err(&mut c, json!({"repo": "web", "title": "x"})).contains("Invalid arguments"));
    assert!(
        err(&mut c, json!({"repo": "nope", "title": "x", "plan": "x"})).contains("No repo matches")
    );
    assert!(err(
        &mut c,
        json!({"repo": "web", "title": "x", "plan": "x", "planFile": "a.md"})
    )
    .contains("Invalid arguments"));
}

#[test]
fn only_chats_list_propose_task() {
    let names = |defs: Value| {
        defs.as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let chat = names(chat_definitions());
    assert_eq!(chat.last().map(String::as_str), Some(PROPOSE_TASK));
    assert_eq!(chat[..chat.len() - 1], names(definitions())[..]);
    assert!(!names(definitions()).iter().any(|n| n == PROPOSE_TASK));
    let schema = &chat_definitions()[chat.len() - 1]["inputSchema"];
    assert_eq!(schema["required"], json!(["repo", "title", "plan"]));
}

#[test]
fn the_skill_describes_every_tool_and_value() {
    let skill = include_str!("../../../../../../skills/nodal-tasks/SKILL.md");
    assert!(skill.starts_with("---\nname: nodal-tasks\ndescription: "));
    let defs = chat_definitions();
    let tools = defs
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap());
    for word in tools
        .chain(STATUSES)
        .chain(PRIORITIES)
        .chain(["planFile", "executor", "verdict"])
    {
        assert!(
            skill.contains(&format!("`{word}`")),
            "SKILL.md does not mention `{word}`"
        );
    }
}
