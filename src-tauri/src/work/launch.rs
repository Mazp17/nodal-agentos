//! Armado de runs: resolución de ejecutor y opciones (run → tarea → repo → proyecto),
//! worktree, prompt según el ejecutor y flags de lanzamiento.
//!
//! Los prompts son funciones puras (con snapshots en los tests); `enqueue_work`,
//! `prepare_review` y `enqueue_review` juntan los datos (base, disco, git) y arman el `Run`.
//! Los prompts no empiezan con `#` ni `/` (Claude Code los tomaría como atajo o comando).

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::db::queries::{projects, repos, runs as qruns, tasks};
use crate::db::rows;
use crate::domain::*;
use crate::runs::ExtraFlags;
use crate::util::new_id;

use super::dto::LaunchInput;
use super::executors::{self, ExecutorInfo};
use super::transitions::executor_label;
use super::{diff, ops, validate, worktree, Env};

/// Tools que el revisor no puede usar (`--disallowedTools`).
pub const REVIEW_DISALLOWED: [&str; 3] = ["Edit", "Write", "NotebookEdit"];
/// Tope del diff que se pega en el prompt de un traspaso.
pub const DIFF_PROMPT_MAX: usize = 30 * 1024;
/// Tope del plan que se pega en el prompt.
pub const PLAN_INLINE_MAX: usize = 16 * 1024;

// ---------- Resolución ----------

/// Ejecutor efectivo: el del run → la tarea → el repo → el proyecto → Settings → Claude.
pub fn pick_executor(task: &Task, repo: &Repo, project: &Project, settings: &Settings, input: &LaunchInput) -> Executor {
    input
        .executor
        .clone()
        .or_else(|| task.assignee.clone())
        .or_else(|| repo.default_executor.clone())
        .or_else(|| project.default_executor.clone())
        .or_else(|| settings.default_executor.clone())
        .unwrap_or(Executor::Claude)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub isolation: Option<Isolation>,
    pub finish: Finish,
    pub review: bool,
    pub options: LaunchOptions,
}

/// Opciones efectivas. Los workflows manejan su propio worktree (`isolation: None`) y los
/// que revisan (`reviews: true`) se saltan el gate.
pub fn resolve(task: &Task, repo: &Repo, input: &LaunchInput, executor: &Executor, executor_reviews: bool) -> Resolved {
    let is_workflow = matches!(executor, Executor::Workflow { .. });
    let isolation = if is_workflow {
        None
    } else {
        Some(input.isolation.or(task.isolation).unwrap_or(repo.default_isolation))
    };
    let review = input.review.or(task.review).unwrap_or(repo.default_review) && !executor_reviews;
    let mut options = repo.launch.clone();
    if let Some(o) = &input.options {
        if o.model.is_some() {
            options.model = o.model.clone();
        }
        if o.effort.is_some() {
            options.effort = o.effort.clone();
        }
        if o.permission_mode.is_some() {
            options.permission_mode = o.permission_mode.clone();
        }
    }
    Resolved { isolation, finish: input.finish.or(task.finish).unwrap_or(repo.default_finish), review, options }
}

/// Revisor: el del repo → el del proyecto → el de Settings.
pub fn reviewer_name(repo: &Repo, project: &Project, settings: &Settings) -> String {
    repo.reviewer.clone().or_else(|| project.reviewer.clone()).unwrap_or_else(|| settings.reviewer.clone())
}

/// Flags de `claude --bg` según el ejecutor y el tipo de run.
pub fn extra_flags(run: &Run) -> ExtraFlags {
    match (&run.executor, run.kind) {
        (Executor::Agent { name, .. }, RunKind::Review) => ExtraFlags {
            agent: Some(name.clone()),
            disallowed_tools: REVIEW_DISALLOWED.iter().map(|s| s.to_string()).collect(),
        },
        (Executor::Agent { name, .. }, RunKind::Work) => ExtraFlags { agent: Some(name.clone()), ..Default::default() },
        (_, RunKind::Review) => ExtraFlags {
            disallowed_tools: REVIEW_DISALLOWED.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        },
        _ => ExtraFlags::default(),
    }
}

// ---------- Prompts ----------

/// Datos de la tarea que van al prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskContext {
    /// `PAY-1`.
    pub key: String,
    pub title: String,
    /// `(identifier, url)` si es importada.
    pub source: Option<(String, String)>,
    pub repo_path: String,
    pub cwd: String,
    pub worktree: Option<WorktreeRef>,
    /// Dónde leer el plan (dentro del cwd si el plan es un archivo del repo).
    pub plan_path: String,
    /// El plan pegado (recortado), para no depender de leer fuera del cwd.
    pub plan_text: Option<String>,
    pub acceptance: Vec<String>,
}

/// Paso anterior de la cadena, para un traspaso.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviousStep {
    pub label: String,
    pub kind: RunKind,
    pub outcome: Option<RunOutcome>,
    pub summary: Option<String>,
    pub verdict: Option<Verdict>,
    /// Diff de la rama contra la base (recortado), o lo sin commitear en `in_place`.
    pub diff: Option<String>,
    pub diff_base: String,
}

fn header(ctx: &TaskContext, out: &mut Vec<String>) {
    if let Some((ident, url)) = &ctx.source {
        out.push(format!("Imported from {ident}: {url}"));
    }
    out.push(format!("Repository: {}", ctx.repo_path));
    out.push(format!("Working directory: {}", ctx.cwd));
    match &ctx.worktree {
        Some(wt) => out.push(format!(
            "This is a git worktree dedicated to this task, on branch `{}` (based on `{}`). Work only here and stay on this branch.",
            wt.branch, wt.base
        )),
        None => out.push("This is the repository folder itself. Don't switch branches.".into()),
    }
}

fn plan_section(ctx: &TaskContext, out: &mut Vec<String>) {
    out.push(String::new());
    out.push("## Plan".into());
    out.push(format!("The plan is at {}.", ctx.plan_path));
    if let Some(text) = &ctx.plan_text {
        out.push("<plan>".into());
        out.push(text.trim_end().to_string());
        out.push("</plan>".into());
    }
}

fn criteria_section(ctx: &TaskContext, out: &mut Vec<String>, for_reviewer: bool) {
    out.push(String::new());
    out.push("## Acceptance criteria".into());
    if ctx.acceptance.is_empty() {
        out.push(if for_reviewer {
            "None were given: infer them from the plan and say so in \"summary\".".into()
        } else {
            "None were given: infer them from the plan.".into()
        });
    } else {
        out.extend(ctx.acceptance.iter().map(|c| format!("- {c}")));
    }
}

fn extra_section(extra: Option<&str>, out: &mut Vec<String>) {
    if let Some(e) = extra {
        out.push(String::new());
        out.push("## Extra instructions for this run".into());
        out.push(e.trim().to_string());
    }
}

pub fn finish_instructions(finish: Finish) -> &'static str {
    match finish {
        Finish::Changes => "Leave your changes uncommitted in the working directory. Don't commit, push or open a pull request.",
        Finish::Commit => "Commit your changes on the current branch with clear messages. Don't push and don't open a pull request.",
        Finish::Pr => "Commit your changes, push the branch and open a pull request with `gh pr create`. Never merge it.",
    }
}

fn outcome_label(o: Option<RunOutcome>) -> &'static str {
    match o {
        Some(RunOutcome::Green) => "done",
        Some(RunOutcome::Yellow) => "done with warnings",
        Some(RunOutcome::Red) => "blocked",
        Some(RunOutcome::Stopped) => "stopped",
        Some(RunOutcome::Unknown) | None => "finished without a clear result",
    }
}

/// Prompt para un agente o para Claude.
pub fn work_prompt(ctx: &TaskContext, finish: Finish, extra: Option<&str>, prev: Option<&PreviousStep>) -> String {
    let mut out = vec![format!("Task {}: {}", ctx.key, ctx.title)];
    header(ctx, &mut out);
    plan_section(ctx, &mut out);
    criteria_section(ctx, &mut out, false);
    if let Some(p) = prev {
        out.push(String::new());
        out.push("## Previous step".into());
        let what = if p.kind == RunKind::Review { "reviewed this work" } else { "worked on this task" };
        out.push(format!("{} {what} before you: {}.", p.label, outcome_label(p.outcome)));
        if let Some(s) = &p.summary {
            out.push(format!("Its summary: {}", s.trim()));
        }
        if let Some(v) = &p.verdict {
            if !v.unmet.is_empty() {
                out.push("Unmet acceptance criteria to fix:".into());
                out.extend(v.unmet.iter().map(|u| format!("- {u}")));
            }
            if !v.nits.is_empty() {
                out.push("Nits:".into());
                out.extend(v.nits.iter().map(|n| format!("- {n}")));
            }
        }
        match &p.diff {
            Some(d) if !d.trim().is_empty() => {
                out.push(format!("Changes so far (diff against `{}`):", p.diff_base));
                out.push("```diff".into());
                out.push(d.trim_end().to_string());
                out.push("```".into());
            }
            _ => out.push("There are no changes in the working directory yet.".into()),
        }
        out.push("You don't have its conversation: continue from the current state of the working directory.".into());
    }
    extra_section(extra, &mut out);
    out.push(String::new());
    out.push("## When you finish".into());
    out.push(finish_instructions(finish).into());
    out.push("End your final message with a fenced JSON block, exactly in this shape:".into());
    out.push("```json".into());
    out.push(r#"{"status": "done", "summary": "<one paragraph: what you did and anything a reviewer should know>", "pr": null, "branch": "<branch name, or null>"}"#.into());
    out.push("```".into());
    out.push(r#"Use "status": "blocked" and explain why in "summary" if you couldn't finish. Put the pull request URL in "pr" if you opened one."#.into());
    out.join("\n")
}

/// Prompt del revisor (solo lectura).
pub fn review_prompt(
    ctx: &TaskContext,
    diff_hint: &str,
    work: Option<(&str, Option<&str>)>,
    extra: Option<&str>,
) -> String {
    let mut out = vec![format!("Review of task {}: {}", ctx.key, ctx.title)];
    header(ctx, &mut out);
    out.push("You are the reviewer: don't modify any file. Read the code, run read-only commands (tests, linters) and report.".into());
    out.push(String::new());
    out.push("## What to review".into());
    out.push(diff_hint.to_string());
    if let Some((label, summary)) = work {
        match summary {
            Some(s) => out.push(format!("{label} did the work and reported: {}", s.trim())),
            None => out.push(format!("{label} did the work and left no report.")),
        }
    }
    plan_section(ctx, &mut out);
    criteria_section(ctx, &mut out, true);
    extra_section(extra, &mut out);
    out.push(String::new());
    out.push("## Verdict".into());
    out.push("End your final message with a fenced JSON block, exactly in this shape:".into());
    out.push("```json".into());
    out.push(r#"{"verdict": "pass", "unmet": [], "nits": [], "summary": "<one paragraph>"}"#.into());
    out.push("```".into());
    out.push(r#"Use "pass" only if every acceptance criterion is met; otherwise "fail", with each unmet criterion or blocking problem in "unmet". Minor suggestions go in "nits"."#.into());
    out.join("\n")
}

#[derive(Serialize)]
struct PlanArgs<'a> {
    plan: &'a str,
    title: &'a str,
    finish: &'a str,
    #[serde(rename = "extraWork", skip_serializing_if = "Option::is_none")]
    extra_work: Option<&'a str>,
}

#[derive(Serialize)]
struct IssueArgs<'a> {
    issue: &'a str,
    #[serde(rename = "extraWork")]
    extra_work: &'a str,
}

/// `finish` que entiende `plan-task`: `pr` → `"pr"`; `changes`/`commit` → `"branch"`.
pub fn workflow_finish(f: Finish) -> &'static str {
    match f {
        Finish::Pr => "pr",
        Finish::Changes | Finish::Commit => "branch",
    }
}

/// `/<wf> <args>`. `linear-issue` recibe el identifier de la tarea importada; el resto, el
/// JSON `{plan, title, finish}` de `plan-task` (armado por serde: nunca a mano).
pub fn workflow_prompt(
    name: &str,
    ctx: &TaskContext,
    provider: Option<&str>,
    finish: Finish,
    extra: Option<&str>,
) -> Result<String, String> {
    if name == "linear-issue" {
        let (ident, _) = ctx
            .source
            .as_ref()
            .filter(|_| provider == Some("linear"))
            .ok_or("linear-issue only runs imported Linear tasks.")?;
        if !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || ident.starts_with('-') {
            return Err(format!("Invalid issue identifier \"{ident}\"."));
        }
        return Ok(match extra {
            None => format!("/{name} {ident}"),
            Some(e) => format!(
                "/{name} {}",
                serde_json::to_string(&IssueArgs { issue: ident, extra_work: e }).map_err(|e| e.to_string())?
            ),
        });
    }
    let json = serde_json::to_string(&PlanArgs {
        plan: &ctx.plan_path,
        title: &ctx.title,
        finish: workflow_finish(finish),
        extra_work: extra,
    })
    .map_err(|e| e.to_string())?;
    Ok(format!("/{name} {json}"))
}

// ---------- Preparación ----------

fn clip_bytes(s: &str, max: usize, what: &str) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n… ({what} truncated: {} KB in total)", &s[..cut], s.len() / 1024)
}

/// Contexto de la tarea para el prompt, en `cwd` (worktree o repo).
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
    // Un plan del repo se lee desde el cwd (en un worktree, su copia de la rama).
    let plan_path = match plan_abs.strip_prefix(&repo.path) {
        // Si el plan no está commiteado, en el worktree no existe: la ruta del repo.
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

/// Busca el ejecutor en el catálogo del repo (tiene que existir).
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

fn check_no_pending(conn: &Connection, task_id: &str) -> Result<(), String> {
    let pending = qruns::pending_for_task(conn, task_id)?;
    match pending.first() {
        None => Ok(()),
        Some(r) if super::queue::awaiting_confirmation(r) => Err(
            "This task has a run migrated from a previous version waiting for confirmation: confirm or cancel it first."
                .into(),
        ),
        Some(r) if r.status == RunStatus::Queued => Err("This task already has a queued run.".into()),
        Some(_) => Err("This task already has a run in progress.".into()),
    }
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

fn blank_run(id: String, task: &Task, repo: &Repo, now: i64, queue_position: f64) -> Run {
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
        queue_position,
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

/// Encola un run de trabajo: resuelve ejecutor y opciones, crea o reusa el worktree, arma
/// el prompt e inserta el run con la tarea en In Progress (y el outbox) en una transacción.
/// Con `handoff`, el prompt lleva el resumen y el diff del paso anterior.
pub fn enqueue_work(conn: &mut Connection, env: &Env, task_id: &str, input: &LaunchInput, handoff: bool, now: i64) -> Result<Run, String> {
    let mut task = tasks::get(conn, task_id)?;
    let repo = repos::get(conn, &task.repo_id)?;
    let project = projects::get(conn, &task.project_id)?;
    check_no_pending(conn, task_id)?;
    let extra = validate::extra_instructions(input.extra_instructions.as_deref())?;
    let settings = rows::load_settings(conn)?;
    let executor = pick_executor(&task, &repo, &project, &settings, input);
    let info = lookup(env, &repo, &executor)?;
    let executor = info.as_ref().map(|i| i.executor.clone()).unwrap_or(executor);
    let reviews = info.as_ref().is_some_and(|i| i.reviews);
    let manages_source = info.as_ref().and_then(|i| i.manages_source.clone());
    let res = resolve(&task, &repo, input, &executor, reviews);
    let options = crate::runs::options::normalize(&res.options).map_err(|e| e.join("\n"))?;

    // Carpeta de trabajo.
    let mut worktree_changed = false;
    let (cwd, wt) = if res.isolation == Some(Isolation::Worktree) {
        let slug = worktree::task_slug(&project.key, task.number, &task.title);
        let dir = worktree::dir_for(&env.worktrees_root, &repo.name, &slug);
        let wt = worktree::ensure(Path::new(&repo.path), &dir, &worktree::branch_for(&slug), task.worktree.as_ref())?;
        worktree_changed = task.worktree.as_ref() != Some(&wt);
        (wt.path.clone(), Some(wt))
    } else {
        (repo.path.clone(), None)
    };

    let ctx = task_context(env, &project, &task, &repo, &cwd, wt.as_ref())?;
    let prev = if handoff { qruns::last_finished(conn, task_id)? } else { None };
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

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    check_no_pending(&tx, task_id)?;
    if worktree_changed {
        task.worktree = wt.clone();
        task.updated_at = now;
        tasks::update(&tx, &task)?;
    }
    let mut run = blank_run(new_id('u', now), &task, &repo, now, qruns::next_queue_position(&tx)?);
    run.cwd = cwd;
    run.executor = executor;
    run.parent_run_id = prev.map(|p| p.id);
    run.prompt = prompt;
    run.extra_instructions = extra;
    run.options = options;
    run.finish = res.finish;
    run.isolation = res.isolation;
    run.review = res.review;
    qruns::insert(&tx, &run)?;
    let next = super::transitions::on_enqueue_work(task.status);
    ops::apply_task_transition(&tx, &task, next, None, manages_source.as_deref(), now)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(run)
}

/// Arma (sin insertar) el run del revisor para la tarea. `parent`: el run de trabajo que
/// revisa (si hay). `reviewer`: override; si no, el configurado.
pub fn prepare_review(
    conn: &Connection,
    env: &Env,
    task: &Task,
    parent: Option<&Run>,
    reviewer: Option<&str>,
    extra: Option<&str>,
    now: i64,
) -> Result<Run, String> {
    let repo = repos::get(conn, &task.repo_id)?;
    let project = projects::get(conn, &task.project_id)?;
    let settings = rows::load_settings(conn)?;
    let name = match reviewer.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => validate::reviewer(r)?,
        None => reviewer_name(&repo, &project, &settings),
    };
    let executor = lookup(env, &repo, &Executor::Agent { name: name.clone(), source: AgentSource::User })?
        .map(|i| i.executor)
        .unwrap_or(Executor::Agent { name, source: AgentSource::User });

    // Dónde y qué revisar.
    let live_wt = task.worktree.as_ref().filter(|w| Path::new(&w.path).is_dir());
    let parent_branch = parent.filter(|p| matches!(p.executor, Executor::Workflow { .. })).and_then(|p| p.branch.clone());
    // Un workflow trabaja en su propio worktree: se revisa su rama (no el worktree de la
    // tarea, que puede tener el trabajo de un paso anterior).
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
    let ctx = task_context(env, &project, task, &repo, &cwd, live_wt)?;
    let work = parent.map(|p| (executor_label(&p.executor), p.summary.clone()));
    let prompt = review_prompt(&ctx, &hint, work.as_ref().map(|(l, s)| (l.as_str(), s.as_deref())), extra);

    let mut run = blank_run(new_id('u', now), task, &repo, now, qruns::next_queue_position(conn)?);
    run.cwd = cwd;
    run.executor = executor;
    run.kind = RunKind::Review;
    run.parent_run_id = parent.map(|p| p.id.clone());
    run.prompt = prompt;
    run.extra_instructions = extra.map(String::from);
    run.finish = parent.map(|p| p.finish).unwrap_or(task.finish.unwrap_or(repo.default_finish));
    // Mismo lock que el trabajo: un `in_place` no arranca mientras se revisa esa carpeta.
    run.isolation = parent.and_then(|p| p.isolation).or(Some(task.isolation.unwrap_or(repo.default_isolation)));
    Ok(run)
}

/// "Review now": encola el revisor sobre el último run de trabajo de la tarea.
pub fn enqueue_review(conn: &mut Connection, env: &Env, task_id: &str, reviewer: Option<&str>, now: i64) -> Result<Run, String> {
    let task = tasks::get(conn, task_id)?;
    check_no_pending(conn, task_id)?;
    let parent = qruns::last_work(conn, task_id)?;
    let run = prepare_review(conn, env, &task, parent.as_ref(), reviewer, None, now)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    check_no_pending(&tx, task_id)?;
    qruns::insert(&tx, &run)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(run)
}

/// Confirma un run migrado que quedó en cola. El viejo queda cancelado como historial.
/// - Con tarea: se encola de nuevo con `enqueue_work` (mismo ejecutor), así el prompt, el
///   worktree y la transición a In Progress salen de la tarea migrada y no del prompt viejo.
///   Si eso falla, el viejo vuelve a quedar en espera de confirmación.
/// - Sin tarea (issue run viejo): se clona tal cual al final de la cola.
pub fn confirm_legacy(conn: &mut Connection, env: &Env, run_id: &str, now: i64) -> Result<Run, String> {
    let old = qruns::get(conn, run_id)?;
    if !super::queue::awaiting_confirmation(&old) {
        return Err("That run isn't waiting for confirmation.".into());
    }
    let mut closed = old.clone();
    closed.status = RunStatus::Canceled;
    closed.finished_at = Some(now);

    if let Some(task_id) = old.task_id.clone() {
        closed.error = Some("Confirmed and re-queued.".into());
        qruns::update(conn, &closed)?;
        let input = LaunchInput { executor: Some(old.executor.clone()), ..Default::default() };
        return match enqueue_work(conn, env, &task_id, &input, false, now) {
            Ok(run) => {
                closed.error = Some(format!("Confirmed and re-queued as {}.", run.id));
                qruns::update(conn, &closed)?;
                Ok(run)
            }
            Err(e) => {
                qruns::update(conn, &old)?;
                Err(e)
            }
        };
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut run = old.clone();
    run.id = new_id('u', now);
    run.legacy_label = None;
    run.queued_at = now;
    run.queue_position = qruns::next_queue_position(&tx)?;
    closed.error = Some(format!("Confirmed and re-queued as {}.", run.id));
    qruns::update(&tx, &closed)?;
    qruns::insert(&tx, &run)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::testutil::{project_of, repo_of, task_of};

    /// Compara con `src/work/snapshots/<name>.txt`. Si no existe (o con
    /// `NODAL_UPDATE_SNAPSHOTS=1`), lo escribe.
    fn snapshot(name: &str, actual: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/work/snapshots").join(format!("{name}.txt"));
        if std::env::var_os("NODAL_UPDATE_SNAPSHOTS").is_some() || !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, actual).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap();
        assert_eq!(actual, expected, "snapshot {name} changed (NODAL_UPDATE_SNAPSHOTS=1 to accept)");
    }

    fn ctx(worktree: bool) -> TaskContext {
        TaskContext {
            key: "PAY-1".into(),
            title: "Logo nuevo en el header".into(),
            source: None,
            repo_path: "/Users/me/Code/web".into(),
            cwd: if worktree { "/Users/me/.nodal/worktrees/web/pay-1-logo".into() } else { "/Users/me/Code/web".into() },
            worktree: worktree.then(|| WorktreeRef {
                path: "/Users/me/.nodal/worktrees/web/pay-1-logo".into(),
                branch: "nodal/pay-1-logo".into(),
                base: "main".into(),
            }),
            plan_path: "/data/tasks/t1/plan.md".into(),
            plan_text: Some("# Logo\nCambiar el logo del header por el nuevo.\n".into()),
            acceptance: vec!["El logo nuevo aparece en el header".into(), "Los tests pasan".into()],
        }
    }

    #[test]
    fn agent_prompt_snapshot() {
        snapshot("agent_worktree_pr", &work_prompt(&ctx(true), Finish::Pr, None, None));
    }

    #[test]
    fn claude_in_place_changes_with_extra_snapshot() {
        let mut c = ctx(false);
        c.source = Some(("ENG-142".into(), "https://linear.app/acme/issue/ENG-142".into()));
        c.acceptance.clear();
        snapshot("claude_in_place_changes", &work_prompt(&c, Finish::Changes, Some("Usá solo CSS."), None));
    }

    #[test]
    fn handoff_prompt_snapshot() {
        let prev = PreviousStep {
            label: "code-reviewer".into(),
            kind: RunKind::Review,
            outcome: Some(RunOutcome::Red),
            summary: Some("Falta el logo en mobile.".into()),
            verdict: Some(Verdict { pass: false, unmet: vec!["El logo nuevo aparece en el header".into()], nits: vec!["Renombrar logo2.svg".into()], summary: None }),
            diff: Some("diff --git a/a.css b/a.css\n+.logo{}\n".into()),
            diff_base: "main".into(),
        };
        snapshot("handoff_commit", &work_prompt(&ctx(true), Finish::Commit, None, Some(&prev)));
    }

    #[test]
    fn review_prompt_snapshot() {
        let hint = "The changes for this task, in the working directory: `git diff main...HEAD` for the branch's commits, `git diff HEAD` for uncommitted work and `git status` for new files.";
        snapshot("review", &review_prompt(&ctx(true), hint, Some(("frontend-developer", Some("Cambié el logo."))), None));
        let mut c = ctx(true);
        c.acceptance.clear();
        let p = review_prompt(&c, hint, Some(("Claude", None)), None);
        assert!(p.contains("infer them from the plan and say so"));
        assert!(p.contains("Claude did the work and left no report."));
    }

    #[test]
    fn workflow_prompts() {
        let c = ctx(false);
        let p = workflow_prompt("plan-task", &c, None, Finish::Commit, None).unwrap();
        assert_eq!(p, r#"/plan-task {"plan":"/data/tasks/t1/plan.md","title":"Logo nuevo en el header","finish":"branch"}"#);
        let p = workflow_prompt("plan-task", &c, None, Finish::Pr, Some("sin tocar el CSS \"global\"")).unwrap();
        let v: serde_json::Value = serde_json::from_str(p.strip_prefix("/plan-task ").unwrap()).unwrap();
        assert_eq!(v["finish"], "pr");
        assert_eq!(v["extraWork"], "sin tocar el CSS \"global\"");
        assert_eq!(workflow_finish(Finish::Changes), "branch");
        assert!(workflow_prompt("linear-issue", &c, None, Finish::Pr, None).unwrap_err().contains("Linear"));
        let mut imported = c.clone();
        imported.source = Some(("ENG-142".into(), "https://linear.app/acme/issue/ENG-142".into()));
        assert_eq!(workflow_prompt("linear-issue", &imported, Some("linear"), Finish::Pr, None).unwrap(), "/linear-issue ENG-142");
        let p = workflow_prompt("linear-issue", &imported, Some("linear"), Finish::Pr, Some("x")).unwrap();
        assert_eq!(p, r#"/linear-issue {"issue":"ENG-142","extraWork":"x"}"#);
        assert!(workflow_prompt("linear-issue", &imported, Some("asana"), Finish::Pr, None).is_err());
    }

    #[test]
    fn resolution_chain() {
        let mut project = project_of("p1", "PAY");
        let mut repo = repo_of("r1", "p1", "/r");
        let mut task = task_of("t1");
        let none = LaunchInput::default();
        let mut settings = Settings::default();
        assert_eq!(pick_executor(&task, &repo, &project, &settings, &none), Executor::Claude);
        let be = Executor::Agent { name: "backend-developer".into(), source: AgentSource::User };
        settings.default_executor = Some(be.clone());
        assert_eq!(pick_executor(&task, &repo, &project, &settings, &none), be, "global como último fallback");
        project.default_executor = Some(Executor::Workflow { name: "plan-task".into() });
        assert_eq!(pick_executor(&task, &repo, &project, &settings, &none), Executor::Workflow { name: "plan-task".into() });
        let fe = Executor::Agent { name: "frontend-developer".into(), source: AgentSource::User };
        repo.default_executor = Some(fe.clone());
        assert_eq!(pick_executor(&task, &repo, &project, &settings, &none), fe);
        task.assignee = Some(Executor::Claude);
        assert_eq!(pick_executor(&task, &repo, &project, &settings, &none), Executor::Claude);
        let input = LaunchInput { executor: Some(fe.clone()), ..Default::default() };
        assert_eq!(pick_executor(&task, &repo, &project, &settings, &input), fe);

        repo.launch.model = Some("opus".into());
        repo.default_finish = Finish::Commit;
        let r = resolve(&task, &repo, &none, &fe, false);
        assert_eq!((r.isolation, r.finish, r.review, r.options.model.as_deref()), (Some(Isolation::Worktree), Finish::Commit, true, Some("opus")));
        task.isolation = Some(Isolation::InPlace);
        task.review = Some(false);
        let input = LaunchInput {
            finish: Some(Finish::Pr),
            options: Some(LaunchOptions { effort: Some("high".into()), ..Default::default() }),
            ..Default::default()
        };
        let r = resolve(&task, &repo, &input, &fe, false);
        assert_eq!((r.isolation, r.finish, r.review), (Some(Isolation::InPlace), Finish::Pr, false));
        assert_eq!((r.options.model.as_deref(), r.options.effort.as_deref()), (Some("opus"), Some("high")));
        // Workflow: sin isolation; si revisa, sin gate.
        task.review = None;
        let wf = Executor::Workflow { name: "plan-task".into() };
        let r = resolve(&task, &repo, &none, &wf, true);
        assert_eq!((r.isolation, r.review), (None, false));
        assert!(resolve(&task, &repo, &none, &wf, false).review);

        let settings = Settings::default();
        assert_eq!(reviewer_name(&repo, &project, &settings), "code-reviewer");
        project.reviewer = Some("proj-reviewer".into());
        assert_eq!(reviewer_name(&repo, &project, &settings), "proj-reviewer");
        repo.reviewer = Some("repo-reviewer".into());
        assert_eq!(reviewer_name(&repo, &project, &settings), "repo-reviewer");
    }

    #[test]
    fn flags_per_executor() {
        use crate::work::testutil::run_of;
        let agent = Executor::Agent { name: "frontend-developer".into(), source: AgentSource::User };
        assert_eq!(extra_flags(&run_of(agent.clone(), RunKind::Work, false)).to_args(), ["--agent", "frontend-developer"]);
        let reviewer = Executor::Agent { name: "code-reviewer".into(), source: AgentSource::User };
        assert_eq!(
            extra_flags(&run_of(reviewer, RunKind::Review, false)).to_args(),
            ["--agent", "code-reviewer", "--disallowedTools", "Edit,Write,NotebookEdit"]
        );
        assert!(extra_flags(&run_of(Executor::Claude, RunKind::Work, false)).to_args().is_empty());
        assert!(extra_flags(&run_of(Executor::Workflow { name: "plan-task".into() }, RunKind::Work, false)).to_args().is_empty());
    }
}
