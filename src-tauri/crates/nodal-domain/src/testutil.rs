//! Test data builders shared across the domain crate's own tests, and by other crates'
//! tests through the `test-support` feature.

use crate::model::*;

pub fn task_of(id: &str) -> Task {
    Task {
        id: id.into(),
        project_id: "p1".into(),
        repo_id: "r1".into(),
        number: 1,
        title: "New logo in the header".into(),
        status: TaskStatus::Todo,
        priority: Priority::None,
        labels: vec![],
        position: 1.0,
        plan: PlanRef::Text,
        plan_overridden: false,
        acceptance: vec!["The new logo shows up in the header".into()],
        worktree: None,
        source: None,
        created_at: 1,
        updated_at: 1,
        closed_at: None,
    }
}

pub fn project_of(id: &str, key: &str) -> Project {
    Project {
        id: id.into(),
        name: format!("Acme {key}"),
        key: key.into(),
        next_task_number: 1,
        color: "#d98c3f".into(),
        created_at: 1,
        archived_at: None,
        root_path: None,
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
        position: 0,
        created_at: 1,
    }
}
