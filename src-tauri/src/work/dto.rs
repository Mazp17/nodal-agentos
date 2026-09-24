//! Entradas de los comandos (espejo de los DTOs de `src/domain/api.ts`).
//! En los patches, campo ausente = no tocar y `null` = borrar (`Option<Option<T>>`).

use serde::Deserialize;

use crate::db::queries::double_option;
use crate::domain::{Executor, Finish, Isolation, LaunchOptions, Priority, TaskStatus};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProject {
    pub name: String,
    pub key: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
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

/// Lo que manda la UI para el plan; el backend lo guarda como `PlanRef`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanInput {
    Text { text: String },
    /// Absoluta, o relativa al repo.
    File { path: String },
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

/// `repoId` mueve la tarea a otro repo del mismo proyecto.
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

/// Overrides para este run; lo que falte sale de la tarea → repo → proyecto.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patches_distinguish_missing_from_null() {
        let p: TaskPatch = serde_json::from_value(json!({"assignee": null, "title": "x"})).unwrap();
        assert_eq!(p.assignee, Some(None));
        assert_eq!(p.isolation, None);
        assert_eq!(p.title.as_deref(), Some("x"));
        let p: TaskPatch = serde_json::from_value(json!({"assignee": {"kind": "claude"}, "review": false})).unwrap();
        assert_eq!(p.assignee, Some(Some(Executor::Claude)));
        assert_eq!(p.review, Some(Some(false)));
        let p: RepoPatch = serde_json::from_value(json!({"model": null, "effort": "high"})).unwrap();
        assert_eq!(p.model, Some(None));
        assert_eq!(p.effort, Some(Some("high".into())));
        assert_eq!(p.permission_mode, None);
        assert!(serde_json::from_value::<TaskPatch>(json!({"projectId": "p"})).is_err());
    }

    #[test]
    fn new_repo_flattens_options_and_plan_input_shape() {
        let r: NewRepo = serde_json::from_value(json!({"path": "/x", "model": "opus", "defaultIsolation": "in_place"})).unwrap();
        assert_eq!(r.launch.model.as_deref(), Some("opus"));
        assert_eq!(r.default_isolation, Some(Isolation::InPlace));
        let t: PlanInput = serde_json::from_value(json!({"kind": "text", "text": "# Plan"})).unwrap();
        assert_eq!(t, PlanInput::Text { text: "# Plan".into() });
        assert!(serde_json::from_value::<PlanInput>(json!({"kind": "url", "path": "x"})).is_err());
        let l: LaunchInput = serde_json::from_value(json!({"extraInstructions": "x", "options": {"effort": "high"}})).unwrap();
        assert_eq!(l.options.unwrap().effort.as_deref(), Some("high"));
    }
}
