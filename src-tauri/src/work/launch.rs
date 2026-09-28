//! Run building: executor and option resolution (run → task → repo → project), worktree,
//! prompt per executor and launch flags.
//!
//! Prompts are pure functions (with snapshots in the tests); `enqueue_work`,
//! `prepare_review` and `enqueue_review` gather the data (database, disk, git) and build the
//! `Run`. Prompts don't start with `#` or `/` (Claude Code would take them as a shortcut or
//! command).
//!
//! Constants, resolution and prompt building moved to `nodal_domain::execution::prompts`;
//! re-exported here so current uses don't break.

use std::path::Path;

use crate::db::{Connection, DbGuard};

use crate::db::queries::{projects, repos, runs as qruns, tasks};
use crate::db::{rows, Db};
use crate::domain::*;
use crate::util::new_id;

use super::dto::LaunchInput;
use super::executors::{self, ExecutorInfo};
use super::transitions::executor_label;
use super::{diff, ops, validate, worktree, Cleaning, Env, CLEANING_ERR};

pub use nodal_domain::execution::prompts::*;

/// Test commands of the repo in `cwd`, detected by its files (fixed list: never text from
/// the repo). They go as `Bash(<cmd>:*)` in the reviewer's allowed tools.
pub fn test_commands(cwd: &Path) -> Vec<String> {
    let has = |f: &str| cwd.join(f).is_file();
    let mut out: Vec<&str> = Vec::new();
    let npm_test = std::fs::read_to_string(cwd.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["scripts"]["test"].is_string());
    if npm_test {
        out.push(if has("pnpm-lock.yaml") {
            "pnpm test"
        } else if has("yarn.lock") {
            "yarn test"
        } else if has("bun.lockb") || has("bun.lock") {
            "bun run test"
        } else {
            "npm test"
        });
    }
    if has("Cargo.toml") {
        out.push("cargo test");
    }
    if has("go.mod") {
        out.push("go test");
    }
    if has("pytest.ini") || has("pyproject.toml") || has("setup.cfg") || has("tox.ini") {
        out.extend(["pytest", "python -m pytest"]);
    }
    if has("mix.exs") {
        out.push("mix test");
    }
    let make_test = std::fs::read_to_string(cwd.join("Makefile"))
        .is_ok_and(|t| t.lines().any(|l| l.starts_with("test:")));
    if make_test {
        out.push("make test");
    }
    out.into_iter().map(String::from).collect()
}

/// Task context for the prompt, in `cwd` (worktree or repo).
pub fn task_context(
    env: &Env,
    project: &Project,
    task: &Task,
    repo: &Repo,
    cwd: &str,
    worktree: Option<&WorktreeRef>,
) -> Result<TaskContext, String> {
    let plan_abs = ops::plan_path(env, task, repo)?;
    let plan_text = ops::read_plan(&plan_abs)?;
    // A repo plan is read from the cwd (in a worktree, the branch's copy).
    let plan_path = match plan_abs.strip_prefix(&repo.path) {
        // If the plan isn't committed, it doesn't exist in the worktree: use the repo path.
        Ok(rel) if matches!(task.plan, PlanRef::File { .. }) && Path::new(cwd).join(rel).is_file() => {
            Path::new(cwd).join(rel)
        }
        _ => plan_abs.clone(),
    };
    Ok(TaskContext {
        key: task_key(&project.key, task.number),
        title: task.title.clone(),
        source: task.source.as_ref().map(|s| (s.identifier.clone(), s.url.clone())),
        repo_path: repo.path.clone(),
        cwd: cwd.to_string(),
        worktree: worktree.cloned(),
        plan_path: plan_path.to_string_lossy().into_owned(),
        plan_text: Some(clip_bytes(&plan_text, PLAN_INLINE_MAX, "plan")),
        acceptance: task.acceptance.clone(),
    })
}

/// Looks up the executor in the repo's catalog (it must exist).
fn lookup(env: &Env, repo: &Repo, executor: &Executor) -> Result<Option<ExecutorInfo>, String> {
    validate::executor(executor)?;
    let claude = env.claude_dir.as_deref();
    let repo_path = Path::new(&repo.path);
    match executor {
        Executor::Claude => Ok(None),
        Executor::Workflow { name } => executors::find_workflow(claude, Some(repo_path), name)
            .map(Some)
            .ok_or_else(|| format!("Workflow \"{name}\" not found in ~/.claude/workflows or {}/.claude/workflows.", repo.path)),
        Executor::Agent { name, .. } => {
            let found = executors::list_agents(claude, Some(repo_path)).into_iter().find(|a| &a.name == name);
            found
                .map(|a| {
                    Some(ExecutorInfo {
                        executor: Executor::Agent { name: a.name, source: a.source },
                        description: a.description,
                        tools: a.tools,
                        manages_source: None,
                        reviews: false,
                        path: Some(a.path.to_string_lossy().into_owned()),
                        source: Some(a.source),
                    })
                })
                .ok_or_else(|| format!("Agent \"{name}\" not found in ~/.claude/agents, the repo or an enabled plugin."))
        }
    }
}

/// No pending runs for the task (except `ignore`: the migrated run being replaced).
fn check_no_pending(conn: &Connection, task_id: &str, ignore: Option<&str>) -> Result<(), String> {
    let pending = qruns::pending_for_task(conn, task_id)?;
    match pending.iter().find(|r| Some(r.id.as_str()) != ignore) {
        None => Ok(()),
        Some(r) if super::queue::awaiting_confirmation(r) => Err(
            "This task has a run migrated from a previous version waiting for confirmation: confirm or cancel it first."
                .into(),
        ),
        Some(r) if r.status == RunStatus::Queued => Err("This task already has a queued run.".into()),
        Some(_) => Err("This task already has a run in progress.".into()),
    }
}

pub const TASK_CHANGED_ERR: &str = "The task changed while the run was being prepared: try again.";

/// The connection (a panic while holding the lock doesn't leave it inconsistent: see `with_db`).
pub fn lock(db: &Db) -> DbGuard<'_> {
    db.guard()
}

fn previous_step(prev: &Run, cwd: &str, base: Option<&str>) -> PreviousStep {
    let diff = diff::collect(Path::new(cwd), base).ok().map(|(p, _)| clip_bytes(&p, DIFF_PROMPT_MAX, "diff"));
    PreviousStep {
        label: executor_label(&prev.executor),
        kind: prev.kind,
        outcome: prev.outcome,
        summary: prev.summary.clone().or_else(|| prev.verdict.as_ref().and_then(|v| v.summary.clone())),
        verdict: prev.verdict.clone(),
        diff,
        diff_base: base.unwrap_or("HEAD").to_string(),
    }
}

fn blank_run(id: String, task: &Task, repo: &Repo, now: i64) -> Run {
    Run {
        id,
        task_id: Some(task.id.clone()),
        repo_id: Some(repo.id.clone()),
        cwd: repo.path.clone(),
        executor: Executor::Claude,
        kind: RunKind::Work,
        parent_run_id: None,
        prompt: String::new(),
        extra_instructions: None,
        options: repo.launch.clone(),
        finish: repo.default_finish,
        isolation: None,
        review: false,
        verdict: None,
        status: RunStatus::Queued,
        // Assigned in the transaction that inserts it.
        queue_position: 0.0,
        claude_run_id: None,
        session_id: None,
        queued_at: now,
        launched_at: None,
        finished_at: None,
        outcome: None,
        summary: None,
        pr_url: None,
        branch: None,
        error: None,
        legacy_label: None,
        tokens: None,
    }
}

/// Enqueues a work run: resolves executor and options, creates or reuses the worktree, builds
/// the prompt and inserts the run with the task In Progress (and the outbox). With `handoff`,
/// the prompt carries the previous step's summary and diff.
///
/// In three steps so the database isn't held during git and disk reads: read → build
/// (without the lock) → a short transaction that re-checks there are no pending runs, that
/// the task hasn't changed (`updated_at`) and that its worktree isn't being cleaned up.
pub fn enqueue_work(
    db: &Db,
    env: &Env,
    cleaning: &Cleaning,
    task_id: &str,
    input: &LaunchInput,
    handoff: bool,
    now: i64,
) -> Result<Run, String> {
    enqueue_work_replacing(db, env, cleaning, task_id, input, handoff, now, None)
}

#[allow(clippy::too_many_arguments)]
fn enqueue_work_replacing(
    db: &Db,
    env: &Env,
    cleaning: &Cleaning,
    task_id: &str,
    input: &LaunchInput,
    handoff: bool,
    now: i64,
    replaces: Option<&Run>,
) -> Result<Run, String> {
    let ignore = replaces.map(|r| r.id.as_str());
    // 1. Read.
    let (task, repo, project, settings, prev) = {
        let conn = lock(db);
        let task = tasks::get(&conn, task_id)?;
        if cleaning.contains(task_id) {
            return Err(CLEANING_ERR.into());
        }
        check_no_pending(&conn, task_id, ignore)?;
        let repo = repos::get(&conn, &task.repo_id)?;
        let project = projects::get(&conn, &task.project_id)?;
        let settings = rows::load_settings(&conn)?;
        let prev = if handoff { qruns::last_finished(&conn, task_id)? } else { None };
        (task, repo, project, settings, prev)
    };

    // 2. Build: catalog, worktree, plan and diff (disk and git, without the database).
    let extra = validate::extra_instructions(input.extra_instructions.as_deref())?;
    let executor = pick_executor(&task, &repo, &project, &settings, input);
    let info = lookup(env, &repo, &executor)?;
    let executor = info.as_ref().map(|i| i.executor.clone()).unwrap_or(executor);
    let reviews = info.as_ref().is_some_and(|i| i.reviews);
    let manages_source = info.as_ref().and_then(|i| i.manages_source.clone());
    let res = resolve(&task, &repo, input, &executor, reviews);
    let options = crate::runs::options::normalize(&res.options).map_err(|e| e.join("\n"))?;
    let (cwd, wt) = if res.isolation == Some(Isolation::Worktree) {
        // Checked again right before touching the worktree: if a cleanup started, it isn't
        // recreated (the transaction would reject it anyway, but it would be left on disk).
        if cleaning.contains(task_id) {
            return Err(CLEANING_ERR.into());
        }
        let slug = worktree::task_slug(&project.key, task.number, &task.title);
        let dir = worktree::dir_for(&env.worktrees_root, &repo.name, &slug);
        let wt = worktree::ensure(Path::new(&repo.path), &dir, &worktree::branch_for(&slug), task.worktree.as_ref())?;
        (wt.path.clone(), Some(wt))
    } else {
        (repo.path.clone(), None)
    };
    let worktree_changed = wt.is_some() && task.worktree != wt;
    let ctx = task_context(env, &project, &task, &repo, &cwd, wt.as_ref())?;
    let prompt = match &executor {
        Executor::Workflow { name } => workflow_prompt(
            name,
            &ctx,
            task.source.as_ref().map(|s| s.provider.as_str()),
            res.finish,
            extra.as_deref(),
        )?,
        _ => {
            let step = prev.as_ref().map(|p| previous_step(p, &cwd, wt.as_ref().map(|w| w.base.as_str())));
            work_prompt(&ctx, res.finish, extra.as_deref(), step.as_ref())
        }
    };
    let mut run = blank_run(new_id('u', now), &task, &repo, now);
    run.cwd = cwd;
    run.executor = executor;
    run.parent_run_id = prev.map(|p| p.id);
    run.prompt = prompt;
    run.extra_instructions = extra;
    run.options = options;
    run.finish = res.finish;
    run.isolation = res.isolation;
    run.review = res.review;

    // 3. Short transaction.
    let mut conn = lock(db);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if cleaning.contains(task_id) {
        return Err(CLEANING_ERR.into());
    }
    check_no_pending(&tx, task_id, ignore)?;
    let mut current = tasks::get(&tx, task_id)?;
    if current.updated_at != task.updated_at {
        return Err(TASK_CHANGED_ERR.into());
    }
    if worktree_changed {
        current.worktree = wt;
        current.updated_at = now;
        tasks::update(&tx, &current)?;
    }
    if let Some(old) = replaces {
        let mut closed = qruns::get(&tx, &old.id)?;
        if !super::queue::awaiting_confirmation(&closed) {
            return Err("That run isn't waiting for confirmation.".into());
        }
        closed.status = RunStatus::Canceled;
        closed.finished_at = Some(now);
        closed.error = Some(format!("Confirmed and re-queued as {}.", run.id));
        qruns::update(&tx, &closed)?;
    }
    run.queue_position = qruns::next_queue_position(&tx)?;
    qruns::insert(&tx, &run)?;
    let next = super::transitions::on_enqueue_work(current.status);
    ops::apply_task_transition(&tx, &current, next, None, manages_source.as_deref(), now)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(run)
}

/// What `build_review` needs from the database.
pub struct ReviewInputs {
    pub repo: Repo,
    pub project: Project,
    pub settings: Settings,
}

impl ReviewInputs {
    pub fn read(conn: &Connection, task: &Task) -> Result<Self, String> {
        Ok(ReviewInputs {
            repo: repos::get(conn, &task.repo_id)?,
            project: projects::get(conn, &task.project_id)?,
            settings: rows::load_settings(conn)?,
        })
    }
}

/// Builds (without inserting or touching the database: reads disk and git) the reviewer run
/// for the task. `parent`: the work run it reviews (if any). `reviewer`: override; otherwise,
/// the configured one. `queue_position` is assigned on insert.
pub fn build_review(
    env: &Env,
    inputs: &ReviewInputs,
    task: &Task,
    parent: Option<&Run>,
    reviewer: Option<&str>,
    extra: Option<&str>,
    now: i64,
) -> Result<Run, String> {
    let ReviewInputs { repo, project, settings } = inputs;
    let name = match reviewer.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => validate::reviewer(r)?,
        None => reviewer_name(repo, project, settings),
    };
    let executor = lookup(env, repo, &Executor::Agent { name: name.clone(), source: AgentSource::User })?
        .map(|i| i.executor)
        .unwrap_or(Executor::Agent { name, source: AgentSource::User });

    // Where and what to review.
    let live_wt = task.worktree.as_ref().filter(|w| Path::new(&w.path).is_dir());
    let parent_branch = parent.filter(|p| matches!(p.executor, Executor::Workflow { .. })).and_then(|p| p.branch.clone());
    // A workflow works in its own worktree: its branch is reviewed (not the task's worktree,
    // which may hold a previous step's work).
    let live_wt = if parent_branch.is_some() { None } else { live_wt };
    let (cwd, hint) = match (live_wt, parent_branch) {
        (_, Some(branch)) => {
            let base = worktree::current_base(Path::new(&repo.path)).unwrap_or_else(|_| "HEAD".into());
            (
                repo.path.clone(),
                format!("The changes are on branch `{branch}`: run `git diff {base}...{branch}` (don't check it out)."),
            )
        }
        (Some(wt), None) => (
            wt.path.clone(),
            format!(
                "The changes for this task, in the working directory: `git diff {}...HEAD` for the branch's commits, `git diff HEAD` for uncommitted work and `git status` for new files.",
                wt.base
            ),
        ),
        (None, None) => (
            repo.path.clone(),
            "The changes for this task are in the repository folder: run `git diff HEAD` and `git status` for uncommitted work, and `git log` for recent commits of this task.".into(),
        ),
    };
    let ctx = task_context(env, project, task, repo, &cwd, live_wt)?;
    let work = parent.map(|p| (executor_label(&p.executor), p.summary.clone()));
    let prompt = review_prompt(&ctx, &hint, work.as_ref().map(|(l, s)| (l.as_str(), s.as_deref())), extra);

    let mut run = blank_run(new_id('u', now), task, repo, now);
    // Same lock as the work, based on where it actually runs: an `in_place` run doesn't start
    // while the repo folder is being reviewed.
    run.isolation = Some(review_isolation(&cwd, &repo.path));
    run.cwd = cwd;
    run.executor = executor;
    run.kind = RunKind::Review;
    run.parent_run_id = parent.map(|p| p.id.clone());
    run.prompt = prompt;
    run.extra_instructions = extra.map(String::from);
    run.finish = parent.map(|p| p.finish).unwrap_or(task.finish.unwrap_or(repo.default_finish));
    run.options.permission_mode = Some(REVIEW_PERMISSION_MODE.into());
    Ok(run)
}

/// "Review now": enqueues the reviewer on the task's last work run. Same three steps as
/// `enqueue_work`.
pub fn enqueue_review(
    db: &Db,
    env: &Env,
    cleaning: &Cleaning,
    task_id: &str,
    reviewer: Option<&str>,
    now: i64,
) -> Result<Run, String> {
    let (task, parent, inputs) = {
        let conn = lock(db);
        let task = tasks::get(&conn, task_id)?;
        if cleaning.contains(task_id) {
            return Err(CLEANING_ERR.into());
        }
        check_no_pending(&conn, task_id, None)?;
        let parent = qruns::last_work(&conn, task_id)?;
        let inputs = ReviewInputs::read(&conn, &task)?;
        (task, parent, inputs)
    };
    let mut run = build_review(env, &inputs, &task, parent.as_ref(), reviewer, None, now)?;
    let mut conn = lock(db);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if cleaning.contains(task_id) {
        return Err(CLEANING_ERR.into());
    }
    check_no_pending(&tx, task_id, None)?;
    if tasks::get(&tx, task_id)?.updated_at != task.updated_at {
        return Err(TASK_CHANGED_ERR.into());
    }
    run.queue_position = qruns::next_queue_position(&tx)?;
    qruns::insert(&tx, &run)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(run)
}

/// Confirms a migrated run that was left queued. The old one is kept cancelled as history,
/// in the same transaction that enqueues the new one (if anything fails, it keeps awaiting
/// confirmation).
/// - With a task: it's re-enqueued with `enqueue_work` (same executor), so the prompt, the
///   worktree and the move to In Progress come from the migrated task, not the old prompt.
/// - Without a task (old issue run): it's cloned as is at the end of the queue.
pub fn confirm_legacy(db: &Db, env: &Env, cleaning: &Cleaning, run_id: &str, now: i64) -> Result<Run, String> {
    let old = qruns::get(&lock(db), run_id)?;
    if !super::queue::awaiting_confirmation(&old) {
        return Err("That run isn't waiting for confirmation.".into());
    }
    if let Some(task_id) = old.task_id.clone() {
        let input = LaunchInput { executor: Some(old.executor.clone()), ..Default::default() };
        return enqueue_work_replacing(db, env, cleaning, &task_id, &input, false, now, Some(&old));
    }

    let mut conn = lock(db);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut closed = qruns::get(&tx, run_id)?;
    if !super::queue::awaiting_confirmation(&closed) {
        return Err("That run isn't waiting for confirmation.".into());
    }
    let mut run = closed.clone();
    run.id = new_id('u', now);
    run.legacy_label = None;
    run.queued_at = now;
    run.queue_position = qruns::next_queue_position(&tx)?;
    closed.status = RunStatus::Canceled;
    closed.finished_at = Some(now);
    closed.error = Some(format!("Confirmed and re-queued as {}.", run.id));
    qruns::update(&tx, &closed)?;
    qruns::insert(&tx, &run)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(run)
}

#[cfg(test)]
mod tests;
