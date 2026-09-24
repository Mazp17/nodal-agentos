//! Constructores de datos de prueba compartidos por los tests de `work`.

use crate::domain::*;

pub fn task_of(id: &str) -> Task {
    Task {
        id: id.into(),
        project_id: "p1".into(),
        repo_id: "r1".into(),
        number: 1,
        title: "Logo nuevo en el header".into(),
        status: TaskStatus::Todo,
        priority: Priority::None,
        labels: vec![],
        position: 1.0,
        plan: PlanRef::Text,
        plan_overridden: false,
        acceptance: vec!["El logo nuevo aparece en el header".into()],
        assignee: None,
        isolation: None,
        finish: None,
        review: None,
        worktree: None,
        source: None,
        created_at: 1,
        updated_at: 1,
        closed_at: None,
    }
}

pub fn run_of(executor: Executor, kind: RunKind, review: bool) -> Run {
    Run {
        id: "run1".into(),
        task_id: Some("t1".into()),
        repo_id: Some("r1".into()),
        cwd: "/repo".into(),
        executor,
        kind,
        parent_run_id: None,
        prompt: "x".into(),
        extra_instructions: None,
        options: LaunchOptions::default(),
        finish: Finish::Pr,
        isolation: Some(Isolation::Worktree),
        review,
        verdict: None,
        status: RunStatus::Launched,
        queue_position: 1.0,
        claude_run_id: Some("abcd1234".into()),
        session_id: Some("s".into()),
        queued_at: 1,
        launched_at: Some(2),
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

pub fn project_of(id: &str, key: &str) -> Project {
    Project {
        id: id.into(),
        name: format!("Acme {key}"),
        key: key.into(),
        next_task_number: 1,
        color: "#d98c3f".into(),
        default_executor: None,
        reviewer: None,
        created_at: 1,
        archived_at: None,
        description: None,
    }
}

pub fn repo_of(id: &str, project_id: &str, path: &str) -> Repo {
    Repo {
        id: id.into(),
        project_id: project_id.into(),
        path: path.into(),
        name: "web".into(),
        launch: LaunchOptions::default(),
        default_executor: None,
        default_isolation: Isolation::Worktree,
        default_finish: Finish::Pr,
        default_review: true,
        reviewer: None,
        position: 0,
        created_at: 1,
    }
}
