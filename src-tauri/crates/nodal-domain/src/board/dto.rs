//! Command inputs (mirror of the DTOs in `src/domain/api.ts`).
//! In patches, a missing field = leave as is and `null` = clear (`Option<Option<T>>`).

use serde::Deserialize;

use crate::model::{Executor, Finish, Isolation, LaunchOptions, Priority, TaskStatus};
use crate::serde_util::double_option;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProject {
    pub name: String,
    pub key: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub root_path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub key: Option<String>,
    pub color: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub root_path: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub default_executor: Option<Option<Executor>>,
    #[serde(default, deserialize_with = "double_option")]
    pub reviewer: Option<Option<String>>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRepo {
    pub path: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub launch: LaunchOptions,
    #[serde(default)]
    pub default_executor: Option<Executor>,
    #[serde(default)]
    pub default_isolation: Option<Isolation>,
    #[serde(default)]
    pub default_finish: Option<Finish>,
    #[serde(default)]
    pub default_review: Option<bool>,
    #[serde(default)]
    pub reviewer: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepoPatch {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub default_executor: Option<Option<Executor>>,
    pub default_isolation: Option<Isolation>,
    pub default_finish: Option<Finish>,
    pub default_review: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub reviewer: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub model: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub effort: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub permission_mode: Option<Option<String>>,
    pub position: Option<i64>,
}

/// What the UI sends for the plan; the backend stores it as `PlanRef`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanInput {
    Text {
        text: String,
    },
    /// Absolute, or relative to the repo.
    File {
        path: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTask {
    pub project_id: String,
    pub repo_id: String,
    pub title: String,
    pub plan: PlanInput,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub priority: Option<Priority>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub acceptance: Option<Vec<String>>,
    #[serde(default)]
    pub assignee: Option<Executor>,
    #[serde(default)]
    pub isolation: Option<Isolation>,
    #[serde(default)]
    pub finish: Option<Finish>,
    #[serde(default)]
    pub review: Option<bool>,
}

/// `repoId` moves the task to another repo of the same project.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskPatch {
    pub repo_id: Option<String>,
    pub title: Option<String>,
    pub plan: Option<PlanInput>,
    pub status: Option<TaskStatus>,
    pub priority: Option<Priority>,
    pub labels: Option<Vec<String>>,
    pub acceptance: Option<Vec<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub assignee: Option<Option<Executor>>,
    #[serde(default, deserialize_with = "double_option")]
    pub isolation: Option<Option<Isolation>>,
    #[serde(default, deserialize_with = "double_option")]
    pub finish: Option<Option<Finish>>,
    #[serde(default, deserialize_with = "double_option")]
    pub review: Option<Option<bool>>,
}

/// Overrides for this run; anything missing comes from task → repo → project.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchInput {
    #[serde(default)]
    pub executor: Option<Executor>,
    #[serde(default)]
    pub isolation: Option<Isolation>,
    #[serde(default)]
    pub finish: Option<Finish>,
    #[serde(default)]
    pub review: Option<bool>,
    #[serde(default)]
    pub extra_instructions: Option<String>,
    #[serde(default)]
    pub options: Option<LaunchOptions>,
}

/// "Merge into <base> & done". Squash is on unless it's turned off; pushing is opt-in.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeInput {
    #[serde(default = "yes")]
    pub squash: bool,
    #[serde(default)]
    pub push: bool,
    #[serde(default)]
    pub cleanup: bool,
}

fn yes() -> bool {
    true
}

#[cfg(test)]
mod tests;
