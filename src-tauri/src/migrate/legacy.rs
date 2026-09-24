//! Formatos de la versión anterior, copiados acá para que la migración no dependa de los
//! módulos viejos (`config`, `tasks`, `issue_runs`), que se borran en F1-B.
//! Todos los campos que se pueden faltar son opcionales: se parsea registro por registro y
//! uno inválido se saltea sin tirar el archivo entero.

use serde::Deserialize;

use crate::domain::LaunchOptions;

pub const CONFIG_FILE: &str = "config.json";
pub const TASKS_FILE: &str = "tasks.json";
pub const TASK_RUNS_FILE: &str = "task-runs.json";
pub const ISSUE_RUNS_FILE: &str = "issue-runs.json";
pub const PLANS_DIR: &str = "tasks";
pub const PLAN_FILE: &str = "plan.md";

// ---------- config.json ----------

/// `{repos: [...], concurrency}`.
#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub repos: Vec<serde_json::Value>,
    #[serde(default)]
    pub concurrency: Option<u32>,
}

/// Una entrada de `repos`. Acepta el formato legacy (un mapeo `teamId`/`projectId` → path)
/// y el "actual" (un repo con `links` a varios scopes, con nombres).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoEntry {
    pub path: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub team_name: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// `"pr"` | `"branch"`.
    #[serde(default)]
    pub finish: Option<String>,
    #[serde(default)]
    pub links: Vec<LinkEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkEntry {
    #[serde(default = "linear")]
    pub provider: String,
    /// `"team"` | `"project"`.
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

fn linear() -> String {
    "linear".into()
}

fn clean(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

impl RepoEntry {
    /// Scopes vinculados. En el formato legacy, el mapeo team+proyecto es el proyecto (el
    /// más específico); solo team es el team.
    pub fn scopes(&self) -> Vec<LinkEntry> {
        if !self.links.is_empty() {
            return self
                .links
                .iter()
                .filter(|l| !l.id.trim().is_empty() && !l.kind.trim().is_empty())
                .cloned()
                .collect();
        }
        if let Some(id) = clean(&self.project_id) {
            return vec![LinkEntry { provider: linear(), kind: "project".into(), id, name: clean(&self.project_name) }];
        }
        if let Some(id) = clean(&self.team_id) {
            return vec![LinkEntry { provider: linear(), kind: "team".into(), id, name: clean(&self.team_name) }];
        }
        Vec::new()
    }

    pub fn launch(&self) -> LaunchOptions {
        LaunchOptions {
            model: clean(&self.model),
            effort: clean(&self.effort),
            permission_mode: clean(&self.permission_mode),
        }
    }
}

// ---------- tasks.json ----------

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PlanRef {
    Text,
    File { path: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub repo_path: String,
    pub title: String,
    pub plan: PlanRef,
    /// `"todo"` | `"done"`.
    pub status: String,
    pub created_at: i64,
    #[serde(default)]
    pub done_at: Option<i64>,
}

// ---------- task-runs.json / issue-runs.json ----------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub task_id: String,
    pub prompt: String,
    /// `"pr"` | `"branch"`.
    #[serde(default)]
    pub finish: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub cwd: String,
    pub queued_at: i64,
    #[serde(default)]
    pub launched_at: Option<i64>,
    /// `queued` | `launching` | `launched` | `failed`.
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRun {
    pub issue_id: String,
    pub identifier: String,
    pub workflow: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub cwd: String,
    pub queued_at: i64,
    #[serde(default)]
    pub launched_at: Option<i64>,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub options: LaunchOptions,
}
