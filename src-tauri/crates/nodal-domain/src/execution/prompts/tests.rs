use std::path::Path;

use super::*;
use crate::model::AgentSource;
use crate::testutil::{project_of, repo_of, task_of};

/// Compares against `src/execution/snapshots/<name>.txt`. If it doesn't exist (or with
/// `NODAL_UPDATE_SNAPSHOTS=1`), writes it.
fn snapshot(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/execution/snapshots")
        .join(format!("{name}.txt"));
    if std::env::var_os("NODAL_UPDATE_SNAPSHOTS").is_some() || !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        actual, expected,
        "snapshot {name} changed (NODAL_UPDATE_SNAPSHOTS=1 to accept)"
    );
}

fn ctx(worktree: bool) -> TaskContext {
    TaskContext {
        key: "PAY-1".into(),
        title: "New logo in the header".into(),
        source: None,
        repo_path: "/Users/me/Code/web".into(),
        cwd: if worktree {
            "/Users/me/.nodal/worktrees/web/pay-1-logo".into()
        } else {
            "/Users/me/Code/web".into()
        },
        worktree: worktree.then(|| WorktreeRef {
            path: "/Users/me/.nodal/worktrees/web/pay-1-logo".into(),
            branch: "nodal/pay-1-logo".into(),
            base: "main".into(),
        }),
        plan_path: "/data/tasks/t1/plan.md".into(),
        plan_text: Some("# Logo\nReplace the header logo with the new one.\n".into()),
        acceptance: vec![
            "The new logo shows up in the header".into(),
            "The tests pass".into(),
        ],
    }
}

#[test]
fn agent_prompt_snapshot() {
    snapshot(
        "agent_worktree_pr",
        &work_prompt(&ctx(true), Finish::Pr, None, None),
    );
}

#[test]
fn claude_in_place_changes_with_extra_snapshot() {
    let mut c = ctx(false);
    c.source = Some((
        "ENG-142".into(),
        "https://linear.app/acme/issue/ENG-142".into(),
    ));
    c.acceptance.clear();
    snapshot(
        "claude_in_place_changes",
        &work_prompt(&c, Finish::Changes, Some("Only use CSS."), None),
    );
}

#[test]
fn handoff_prompt_snapshot() {
    let prev = PreviousStep {
        label: "code-reviewer".into(),
        kind: RunKind::Review,
        outcome: Some(RunOutcome::Red),
        summary: Some("The logo is missing on mobile.".into()),
        verdict: Some(Verdict {
            pass: false,
            unmet: vec!["The new logo shows up in the header".into()],
            nits: vec!["Rename logo2.svg".into()],
            summary: None,
        }),
        diff: Some("diff --git a/a.css b/a.css\n+.logo{}\n".into()),
        diff_base: "main".into(),
    };
    snapshot(
        "handoff_commit",
        &work_prompt(&ctx(true), Finish::Commit, None, Some(&prev)),
    );
}

#[test]
fn review_prompt_snapshot() {
    let hint = "The changes for this task, in the working directory: `git diff main...HEAD` for the branch's commits, `git diff HEAD` for uncommitted work and `git status` for new files.";
    snapshot(
        "review",
        &review_prompt(
            &ctx(true),
            hint,
            Some(("frontend-developer", Some("I changed the logo."))),
            None,
        ),
    );
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
    assert_eq!(
        p,
        r#"/plan-task {"plan":"/data/tasks/t1/plan.md","title":"New logo in the header","finish":"branch"}"#
    );
    let p = workflow_prompt(
        "plan-task",
        &c,
        None,
        Finish::Pr,
        Some("without touching the \"global\" CSS"),
    )
    .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(p.strip_prefix("/plan-task ").unwrap()).unwrap();
    assert_eq!(v["finish"], "pr");
    assert_eq!(v["extraWork"], "without touching the \"global\" CSS");
    assert_eq!(workflow_finish(Finish::Changes), "branch");
    assert!(workflow_prompt("linear-issue", &c, None, Finish::Pr, None)
        .unwrap_err()
        .contains("Linear"));
    let mut imported = c.clone();
    imported.source = Some((
        "ENG-142".into(),
        "https://linear.app/acme/issue/ENG-142".into(),
    ));
    assert_eq!(
        workflow_prompt("linear-issue", &imported, Some("linear"), Finish::Pr, None).unwrap(),
        "/linear-issue ENG-142"
    );
    let p = workflow_prompt(
        "linear-issue",
        &imported,
        Some("linear"),
        Finish::Pr,
        Some("x"),
    )
    .unwrap();
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
    assert_eq!(
        pick_executor(&task, &repo, &project, &settings, &none),
        Executor::Claude
    );
    let be = Executor::Agent {
        name: "backend-developer".into(),
        source: AgentSource::User,
    };
    settings.default_executor = Some(be.clone());
    assert_eq!(
        pick_executor(&task, &repo, &project, &settings, &none),
        be,
        "global as the last fallback"
    );
    project.default_executor = Some(Executor::Workflow {
        name: "plan-task".into(),
    });
    assert_eq!(
        pick_executor(&task, &repo, &project, &settings, &none),
        Executor::Workflow {
            name: "plan-task".into()
        }
    );
    let fe = Executor::Agent {
        name: "frontend-developer".into(),
        source: AgentSource::User,
    };
    repo.default_executor = Some(fe.clone());
    assert_eq!(pick_executor(&task, &repo, &project, &settings, &none), fe);
    task.assignee = Some(Executor::Claude);
    assert_eq!(
        pick_executor(&task, &repo, &project, &settings, &none),
        Executor::Claude
    );
    let input = LaunchInput {
        executor: Some(fe.clone()),
        ..Default::default()
    };
    assert_eq!(pick_executor(&task, &repo, &project, &settings, &input), fe);

    repo.launch.model = Some("opus".into());
    repo.default_finish = Finish::Commit;
    let r = resolve(&task, &repo, &none, &fe, false);
    assert_eq!(
        (r.isolation, r.finish, r.review, r.options.model.as_deref()),
        (
            Some(Isolation::Worktree),
            Finish::Commit,
            true,
            Some("opus")
        )
    );
    task.isolation = Some(Isolation::InPlace);
    task.review = Some(false);
    let input = LaunchInput {
        finish: Some(Finish::Pr),
        options: Some(LaunchOptions {
            effort: Some("high".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let r = resolve(&task, &repo, &input, &fe, false);
    assert_eq!(
        (r.isolation, r.finish, r.review),
        (Some(Isolation::InPlace), Finish::Pr, false)
    );
    assert_eq!(
        (r.options.model.as_deref(), r.options.effort.as_deref()),
        (Some("opus"), Some("high"))
    );
    // Workflow: no isolation; if it reviews, no gate.
    task.review = None;
    let wf = Executor::Workflow {
        name: "plan-task".into(),
    };
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
    use crate::testutil::run_of;
    let agent = Executor::Agent {
        name: "frontend-developer".into(),
        source: AgentSource::User,
    };
    let tests = vec!["npm test".to_string()];
    let sp = "--append-system-prompt";
    assert_eq!(
        extra_flags(&run_of(agent.clone(), RunKind::Work, false), &tests).to_args(),
        [
            "--agent",
            "frontend-developer",
            sp,
            UNATTENDED_SYSTEM_PROMPT
        ]
    );
    let reviewer = Executor::Agent {
        name: "code-reviewer".into(),
        source: AgentSource::User,
    };
    assert_eq!(
        extra_flags(&run_of(reviewer, RunKind::Review, false), &tests).to_args(),
        [
            "--agent",
            "code-reviewer",
            "--allowedTools",
            "Read",
            "Grep",
            "Glob",
            "Bash(git diff:*)",
            "Bash(git status:*)",
            "Bash(git log:*)",
            "Bash(git show:*)",
            "Bash(npm test:*)",
            "--disallowedTools",
            "Edit,Write,NotebookEdit",
            sp,
            UNATTENDED_SYSTEM_PROMPT
        ]
    );
    assert_eq!(
        extra_flags(&run_of(Executor::Claude, RunKind::Work, false), &tests).to_args(),
        [sp, UNATTENDED_SYSTEM_PROMPT]
    );
    assert_eq!(
        extra_flags(
            &run_of(
                Executor::Workflow {
                    name: "plan-task".into()
                },
                RunKind::Work,
                false
            ),
            &[]
        )
        .to_args(),
        [
            sp.to_string(),
            format!("{UNATTENDED_SYSTEM_PROMPT} {WORKFLOW_CLOSING_PROMPT}")
        ]
    );
}

#[test]
fn reviewer_ignores_the_repo_permission_mode() {
    use crate::testutil::run_of;
    let mut review = run_of(Executor::Claude, RunKind::Review, false);
    review.options.permission_mode = Some("bypassPermissions".into());
    review.options.model = Some("opus".into());
    let o = launch_options(&review);
    assert_eq!(
        (o.permission_mode.as_deref(), o.model.as_deref()),
        (Some("dontAsk"), Some("opus"))
    );
    let mut work = run_of(Executor::Claude, RunKind::Work, false);
    work.options.permission_mode = Some("acceptEdits".into());
    assert_eq!(
        launch_options(&work).permission_mode.as_deref(),
        Some("acceptEdits")
    );
}
