//! Board context: projects, repos, tasks, relations, executors and settings.

pub mod ops;

use std::path::Path;
use std::sync::Arc;

use nodal_domain::board::dto::{NewProject, NewRepo, NewTask, ProjectPatch, RepoPatch, TaskPatch};
use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::executors::ExecutorInfo;
use nodal_domain::model::{HiddenExecutor, Project, RelationKind, Repo, Settings, Task, TaskRelation, TaskStatus};
use nodal_domain::util::check_id;
use nodal_store::board::{hidden_executors, projects, relations, repos, tasks};
use nodal_store::{rows, Conn as Connection, StoreError};

use crate::core::{AppError, Core};

fn opt_id(id: Option<String>, what: &str) -> Result<Option<String>, String> {
    let id = id.filter(|s| !s.trim().is_empty());
    if let Some(i) = &id {
        check_id(i, what)?;
    }
    Ok(id)
}

/// Runs `f` on a blocking thread, converting the `"Internal error: {e}"` `JoinError` message
/// into `AppError` the same way `Board::db` does for store errors.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, AppError> {
    Ok(crate::core::blocking(f).await?)
}

/// Board context: projects, repos, tasks, relations, executors and settings.
pub struct Board {
    core: Arc<Core>,
}

impl Board {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    /// Runs `f` with the connection; `ops` errors already come ready to display.
    async fn db<T, F>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, String> + Send + 'static,
    {
        Ok(nodal_store::with_db(&self.core.db, move |c| f(c).map_err(StoreError::Invalid)).await?)
    }

    // ---------- Projects ----------

    pub async fn list_projects(&self, include_archived: Option<bool>) -> Result<Vec<Project>, AppError> {
        let all = include_archived.unwrap_or(false);
        self.db(move |c| Ok(projects::list(c, all)?)).await
    }

    pub async fn create_project(&self, input: NewProject) -> Result<Project, AppError> {
        let env = self.core.env.clone();
        let clock = self.core.clock.clone();
        let p = self.db(move |c| ops::create_project(c, &env, &input, clock.now_ms())).await?;
        self.core.notifier.notify(ChangeKind::Projects, None);
        Ok(p)
    }

    pub async fn update_project(&self, id: String, patch: ProjectPatch) -> Result<Project, AppError> {
        check_id(&id, "project")?;
        let env = self.core.env.clone();
        let clock = self.core.clock.clone();
        let p = self.db(move |c| ops::update_project(c, &env, &id, &patch, clock.now_ms())).await?;
        self.core.notifier.notify(ChangeKind::Projects, None);
        Ok(p)
    }

    /// `stop_chats` is called right after `check_id`, exactly where today's command stops the
    /// project's chats before deleting it.
    pub async fn delete_project(&self, id: String, stop_chats: impl FnOnce(&str) + Send) -> Result<(), AppError> {
        check_id(&id, "project")?;
        let env = self.core.env.clone();
        stop_chats(&id);
        let task_ids = self.db(move |c| ops::delete_project(c, &id)).await?;
        self.core.notifier.notify_all(
            &[ChangeKind::Projects, ChangeKind::Tasks, ChangeKind::Runs, ChangeKind::Queue, ChangeKind::Sources, ChangeKind::Chats],
            None,
        );
        blocking(move || {
            for t in task_ids {
                let _ = env.plans.remove_dir_all(&env.plan_dir(&t));
            }
            Ok(())
        })
        .await
    }

    // ---------- Repos ----------

    pub async fn list_repos(&self, project_id: Option<String>) -> Result<Vec<Repo>, AppError> {
        let project_id = opt_id(project_id, "project")?;
        self.db(move |c| Ok(repos::list(c, project_id.as_deref())?)).await
    }

    /// `input.path` can be any folder inside the repo: it's resolved to the git root.
    pub async fn add_repo(&self, project_id: String, input: NewRepo) -> Result<Repo, AppError> {
        check_id(&project_id, "project")?;
        let git = self.core.env.git.clone();
        let path = input.path.clone();
        let root = blocking(move || git.require_git_root(&path).map_err(|e| e.to_string())).await?;
        let clock = self.core.clock.clone();
        let r = self.db(move |c| ops::add_repo(c, &project_id, &input, &root, clock.now_ms())).await?;
        self.core.notifier.notify(ChangeKind::Projects, Some(&r.project_id));
        Ok(r)
    }

    pub async fn update_repo(&self, id: String, patch: RepoPatch) -> Result<Repo, AppError> {
        check_id(&id, "repo")?;
        let r = self.db(move |c| ops::update_repo(c, &id, &patch)).await?;
        self.core.notifier.notify(ChangeKind::Projects, Some(&r.project_id));
        Ok(r)
    }

    pub async fn delete_repo(&self, id: String) -> Result<(), AppError> {
        check_id(&id, "repo")?;
        self.db(move |c| ops::delete_repo(c, &id)).await?;
        self.core.notifier.notify_all(&[ChangeKind::Projects, ChangeKind::Tasks], None);
        Ok(())
    }

    // ---------- Tasks ----------

    pub async fn list_tasks(&self, project_id: Option<String>) -> Result<Vec<Task>, AppError> {
        let project_id = opt_id(project_id, "project")?;
        self.db(move |c| Ok(tasks::list(c, project_id.as_deref())?)).await
    }

    pub async fn get_task(&self, id: String) -> Result<Task, AppError> {
        check_id(&id, "task")?;
        self.db(move |c| Ok(tasks::get(c, &id)?)).await
    }

    pub async fn create_task(&self, input: NewTask) -> Result<Task, AppError> {
        check_id(&input.project_id, "project")?;
        check_id(&input.repo_id, "repo")?;
        let env = self.core.env.clone();
        let clock = self.core.clock.clone();
        let t = self.db(move |c| ops::create_task(c, &env, &input, clock.now_ms())).await?;
        self.core.notifier.notify(ChangeKind::Tasks, Some(&t.project_id));
        Ok(t)
    }

    pub async fn update_task(&self, id: String, patch: TaskPatch) -> Result<Task, AppError> {
        check_id(&id, "task")?;
        if let Some(r) = &patch.repo_id {
            check_id(r, "repo")?;
        }
        let env = self.core.env.clone();
        let clock = self.core.clock.clone();
        let t = self.db(move |c| ops::update_task(c, &env, &id, &patch, clock.now_ms())).await?;
        self.core.notifier.notify(ChangeKind::Tasks, Some(&t.project_id));
        Ok(t)
    }

    pub async fn delete_task(&self, id: String) -> Result<(), AppError> {
        check_id(&id, "task")?;
        let env = self.core.env.clone();
        self.db(move |c| ops::delete_task(c, &env, &id)).await?;
        self.core.notifier.notify_all(&[ChangeKind::Tasks, ChangeKind::Runs, ChangeKind::Queue], None);
        Ok(())
    }

    pub async fn move_task(&self, id: String, status: TaskStatus, position: f64) -> Result<Task, AppError> {
        check_id(&id, "task")?;
        let clock = self.core.clock.clone();
        let t = self.db(move |c| ops::move_task(c, &id, status, position, clock.now_ms())).await?;
        self.core.notifier.notify(ChangeKind::Tasks, Some(&t.project_id));
        Ok(t)
    }

    /// New order of a board column (ids from a single project, all with `status`).
    pub async fn reorder_tasks(&self, status: TaskStatus, ordered_ids: Vec<String>) -> Result<(), AppError> {
        for id in &ordered_ids {
            check_id(id, "task")?;
        }
        let clock = self.core.clock.clone();
        let project = self.db(move |c| ops::reorder_tasks(c, status, &ordered_ids, clock.now_ms())).await?;
        if let Some(p) = project {
            self.core.notifier.notify(ChangeKind::Tasks, Some(&p));
        }
        Ok(())
    }

    pub async fn read_task_plan(&self, id: String) -> Result<String, AppError> {
        check_id(&id, "task")?;
        let env = self.core.env.clone();
        self.db(move |c| ops::read_task_plan(c, &env, &id)).await
    }

    pub async fn list_task_relations(&self, task_id: String) -> Result<Vec<TaskRelation>, AppError> {
        check_id(&task_id, "task")?;
        self.db(move |c| Ok(relations::list(c, &task_id)?)).await
    }

    pub async fn add_task_relation(&self, task_id: String, other_id: String, kind: RelationKind) -> Result<(), AppError> {
        check_id(&task_id, "task")?;
        check_id(&other_id, "task")?;
        self.db(move |c| ops::add_relation(c, &task_id, &other_id, kind)).await?;
        self.core.notifier.notify(ChangeKind::Tasks, None);
        Ok(())
    }

    pub async fn remove_task_relation(&self, task_id: String, other_id: String, kind: RelationKind) -> Result<(), AppError> {
        check_id(&task_id, "task")?;
        check_id(&other_id, "task")?;
        self.db(move |c| ops::remove_relation(c, &task_id, &other_id, kind)).await?;
        self.core.notifier.notify(ChangeKind::Tasks, None);
        Ok(())
    }

    // ---------- Executors ----------

    pub async fn list_executors(&self, repo_id: Option<String>) -> Result<Vec<ExecutorInfo>, AppError> {
        let repo_id = opt_id(repo_id, "repo")?;
        let repo_path = match repo_id {
            Some(id) => Some(self.db(move |c| Ok(repos::get(c, &id)?.path)).await?),
            None => None,
        };
        let claude_dir = self.core.env.claude_dir.clone();
        let claude_config = self.core.env.claude_config.clone();
        blocking(move || Ok(claude_config.catalog(claude_dir.as_deref(), repo_path.as_deref().map(Path::new)))).await
    }

    /// Agents and workflows the project hides from its pickers. `list_executors` stays unfiltered.
    pub async fn list_hidden_executors(&self, project_id: String) -> Result<Vec<HiddenExecutor>, AppError> {
        check_id(&project_id, "project")?;
        self.db(move |c| Ok(hidden_executors::list(c, &project_id)?)).await
    }

    /// Hides or shows one agent or workflow in the project; returns the project's new set.
    pub async fn set_executor_hidden(&self, project_id: String, key: HiddenExecutor, hidden: bool) -> Result<Vec<HiddenExecutor>, AppError> {
        check_id(&project_id, "project")?;
        if let Some(r) = &key.repo_id {
            check_id(r, "repo")?;
        }
        let pid = project_id.clone();
        let out = self
            .db(move |c| {
                hidden_executors::set(c, &pid, &key, hidden)?;
                Ok(hidden_executors::list(c, &pid)?)
            })
            .await?;
        self.core.notifier.notify(ChangeKind::Projects, Some(&project_id));
        Ok(out)
    }

    // ---------- Settings ----------

    pub async fn get_settings(&self) -> Result<Settings, AppError> {
        self.db(|c| Ok(rows::load_settings(c)?)).await
    }

    /// Doesn't kick the queue itself: the command does that right after this returns (today's
    /// order), since concurrency changes may free up room for queued runs.
    pub async fn set_settings(&self, settings: Settings) -> Result<Settings, AppError> {
        let s = self.db(move |c| ops::set_settings(c, &settings)).await?;
        self.core.notifier.notify(ChangeKind::Projects, None);
        Ok(s)
    }
}

#[cfg(test)]
mod tests;
