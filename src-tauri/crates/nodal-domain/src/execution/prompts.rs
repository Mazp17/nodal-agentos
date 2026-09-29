//! Run building: executor and option resolution (run → task → repo → project), prompt per
//! executor and launch flags. Pure functions (with snapshots in the tests).
//!
//! Worktree creation, plan reading, executor lookup and everything else that touches disk,
//! git or the database lives in nodal-host / nodal-app (`enqueue_work`, `build_review`…).
//! Prompts don't start with `#` or `/` (Claude Code would take them as a shortcut or command).

use serde::Serialize;

use crate::board::dto::LaunchInput;
use crate::model::claude::ExtraFlags;
use crate::model::{
    Executor, Finish, Isolation, LaunchOptions, Project, Repo, Run, RunKind, RunOutcome, Settings,
    Task, Verdict, WorktreeRef,
};

/// Goes on every run (`--append-system-prompt`): a session that ends by asking a question
/// stays `idle` + `working` and the task never leaves In Progress.
pub const UNATTENDED_SYSTEM_PROMPT: &str = "You are running unattended, launched by Nodal. When done, report the result in one short message and stop. Don't ask questions or offer next steps: Nodal decides what happens next.";
/// Appended for workflow runs: their details (findings, nits) already live in the workflow's
/// result, so the closing message is just a status line.
pub const WORKFLOW_CLOSING_PROMPT: &str = "Close with a single status line: the real outcome (green, yellow or red) and PR or branch, e.g. `green · PR #39`. Don't list pending items or nits: they stay in the workflow's result. If the workflow didn't finish, say why in that line.";
/// Tools the reviewer can't use (`--disallowedTools`).
pub const REVIEW_DISALLOWED: [&str; 3] = ["Edit", "Write", "NotebookEdit"];
/// The reviewer's permission mode, whatever the repo's is: `dontAsk` denies without asking
/// anything not in `--allowedTools` (a background session isn't left waiting for a
/// permission). With `plan`, non-read-only Bash commands (the tests) ask for approval and the
/// session ends up in "Needs input".
pub const REVIEW_PERMISSION_MODE: &str = "dontAsk";
/// The only things the reviewer can use (`--allowedTools`), plus the repo's test commands
/// (`test_commands`): reading and query-only git. It isn't strictly read-only: the tests run
/// repo code and `git diff/log/show --output=<f>` writes a file.
pub const REVIEW_ALLOWED: [&str; 7] = [
    "Read",
    "Grep",
    "Glob",
    "Bash(git diff:*)",
    "Bash(git status:*)",
    "Bash(git log:*)",
    "Bash(git show:*)",
];

/// Options the run is launched with: the stored ones, except for the reviewer, which always
/// uses `REVIEW_PERMISSION_MODE` (even if the repo asks for `bypassPermissions`/`acceptEdits`).
pub fn launch_options(run: &Run) -> LaunchOptions {
    let mut o = run.options.clone();
    if run.kind == RunKind::Review {
        o.permission_mode = Some(REVIEW_PERMISSION_MODE.into());
    }
    o
}
/// Cap on the diff pasted into a handoff prompt.
pub const DIFF_PROMPT_MAX: usize = 30 * 1024;
/// Cap on the plan pasted into the prompt.
pub const PLAN_INLINE_MAX: usize = 16 * 1024;

// ---------- Resolution ----------

/// Effective executor: the run's → the task's → the repo's → the project's → Settings →
/// Claude.
pub fn pick_executor(
    task: &Task,
    repo: &Repo,
    project: &Project,
    settings: &Settings,
    input: &LaunchInput,
) -> Executor {
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

/// Effective options. Workflows manage their own worktree (`isolation: None`) and the ones
/// that review (`reviews: true`) skip the gate.
pub fn resolve(
    task: &Task,
    repo: &Repo,
    input: &LaunchInput,
    executor: &Executor,
    executor_reviews: bool,
) -> Resolved {
    let is_workflow = matches!(executor, Executor::Workflow { .. });
    let isolation = if is_workflow {
        None
    } else {
        Some(
            input
                .isolation
                .or(task.isolation)
                .unwrap_or(repo.default_isolation),
        )
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
    Resolved {
        isolation,
        finish: input.finish.or(task.finish).unwrap_or(repo.default_finish),
        review,
        options,
    }
}

/// Reviewer: the repo's → the project's → the one in Settings.
pub fn reviewer_name(repo: &Repo, project: &Project, settings: &Settings) -> String {
    repo.reviewer
        .clone()
        .or_else(|| project.reviewer.clone())
        .unwrap_or_else(|| settings.reviewer.clone())
}

/// `claude --bg` flags according to the executor and run type. `tests`: the cwd's test
/// commands (`test_commands`), which the reviewer may run.
pub fn extra_flags(run: &Run, tests: &[String]) -> ExtraFlags {
    let agent = match &run.executor {
        Executor::Agent { name, .. } => Some(name.clone()),
        _ => None,
    };
    let append_system_prompt = Some(match (&run.kind, &run.executor) {
        (RunKind::Work, Executor::Workflow { .. }) => {
            format!("{UNATTENDED_SYSTEM_PROMPT} {WORKFLOW_CLOSING_PROMPT}")
        }
        _ => UNATTENDED_SYSTEM_PROMPT.to_string(),
    });
    match run.kind {
        RunKind::Work => ExtraFlags {
            agent,
            append_system_prompt,
            ..Default::default()
        },
        RunKind::Review => ExtraFlags {
            agent,
            append_system_prompt,
            allowed_tools: REVIEW_ALLOWED
                .iter()
                .map(|s| s.to_string())
                .chain(tests.iter().map(|t| format!("Bash({t}:*)")))
                .collect(),
            disallowed_tools: REVIEW_DISALLOWED.iter().map(|s| s.to_string()).collect(),
        },
    }
}

// ---------- Prompts ----------

/// Task data that goes into the prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskContext {
    /// `PAY-1`.
    pub key: String,
    pub title: String,
    /// `(identifier, url)` if imported.
    pub source: Option<(String, String)>,
    pub repo_path: String,
    pub cwd: String,
    pub worktree: Option<WorktreeRef>,
    /// Where to read the plan (inside the cwd if the plan is a repo file).
    pub plan_path: String,
    /// The pasted plan (truncated), so it doesn't depend on reading outside the cwd.
    pub plan_text: Option<String>,
    pub acceptance: Vec<String>,
}

/// Previous step in the chain, for a handoff.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviousStep {
    pub label: String,
    pub kind: RunKind,
    pub outcome: Option<RunOutcome>,
    pub summary: Option<String>,
    pub verdict: Option<Verdict>,
    /// Diff of the branch against the base (truncated), or uncommitted work in `in_place`.
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

/// Prompt for an agent or for Claude.
pub fn work_prompt(
    ctx: &TaskContext,
    finish: Finish,
    extra: Option<&str>,
    prev: Option<&PreviousStep>,
) -> String {
    let mut out = vec![format!("Task {}: {}", ctx.key, ctx.title)];
    header(ctx, &mut out);
    plan_section(ctx, &mut out);
    criteria_section(ctx, &mut out, false);
    if let Some(p) = prev {
        out.push(String::new());
        out.push("## Previous step".into());
        let what = if p.kind == RunKind::Review {
            "reviewed this work"
        } else {
            "worked on this task"
        };
        out.push(format!(
            "{} {what} before you: {}.",
            p.label,
            outcome_label(p.outcome)
        ));
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

/// Reviewer prompt (read-only).
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
    out.push(
        r#"{"verdict": "pass", "unmet": [], "nits": [], "summary": "<one paragraph>"}"#.into(),
    );
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

/// `finish` as `plan-task` understands it: `pr` → `"pr"`; `changes`/`commit` → `"branch"`.
pub fn workflow_finish(f: Finish) -> &'static str {
    match f {
        Finish::Pr => "pr",
        Finish::Changes | Finish::Commit => "branch",
    }
}

/// `/<wf> <args>`. `linear-issue` gets the imported task's identifier; the rest, the
/// `plan-task` JSON `{plan, title, finish}` (built by serde: never by hand).
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
                serde_json::to_string(&IssueArgs {
                    issue: ident,
                    extra_work: e
                })
                .map_err(|e| e.to_string())?
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

// ---------- Preparation ----------

/// Clips a byte string, appending a note about how much was truncated. `pub`: used by both
/// this module's prompt builders and by `enqueue_work`/`previous_step` in nodal-app.
pub fn clip_bytes(s: &str, max: usize, what: &str) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n… ({what} truncated: {} KB in total)",
        &s[..cut],
        s.len() / 1024
    )
}

/// The reviewer's isolation based on where it runs: in the repo folder it's `in_place` (takes
/// the repo lock); in the task's worktree, `worktree`.
pub fn review_isolation(cwd: &str, repo_path: &str) -> Isolation {
    if std::path::Path::new(cwd.trim_end_matches('/'))
        == std::path::Path::new(repo_path.trim_end_matches('/'))
    {
        Isolation::InPlace
    } else {
        Isolation::Worktree
    }
}

#[cfg(test)]
mod tests;
