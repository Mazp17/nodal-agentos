//! Task chats: a task worked in a Claude chat that runs in the task's own worktree, with the
//! task status following the chat (In Progress on start, Review when a turn leaves changes).

use std::path::Path;

use nodal_domain::execution::worktree::{branch_for, dir_for, task_slug};
use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::{task_key, Chat, PlanRef, Project, Repo, Task, TaskStatus};
use nodal_domain::util::{check_id, clip_chars};
use nodal_store::board::{projects, repos, tasks};
use nodal_store::chats;

use crate::board::ops as board;
use crate::core::{blocking, AppError};
use crate::Env;

use super::{ops, Chats, NewChat};

const PLAN_MAX: usize = 50_000;

/// First message of a task chat: everything the agent needs to start (it has no other context).
fn first_message(env: &Env, project: &Project, task: &Task, repo: &Repo) -> String {
    let mut m = format!("Work on this task: {}: {}\n", task_key(&project.key, task.number), task.title);
    if let Some(s) = &task.source {
        m.push_str(&format!("Source: {} {}\n", s.identifier, s.url));
    }
    let plan = board::plan_path(env, task, repo).and_then(|p| board::read_plan(env, &p)).unwrap_or_default();
    if !plan.trim().is_empty() {
        let label = if matches!(task.plan, PlanRef::Text) { "Plan" } else { "Plan (from the plan file)" };
        m.push_str(&format!("\n{label}:\n{}\n", clip_chars(plan.trim(), PLAN_MAX)));
    }
    if !task.acceptance.is_empty() {
        m.push_str("\nAcceptance criteria:\n");
        for a in &task.acceptance {
            m.push_str(&format!("- {a}\n"));
        }
    }
    m.push_str("\nYou are in the task's own git worktree on its own branch.");
    m
}

/// A task chat ready to open. `prompt`: the task's first message, still unsent (`None` for a
/// chat that already existed): the UI sends it through its own attached channel so it streams live.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskChatStart {
    pub chat: Chat,
    pub prompt: Option<String>,
}

impl Chats {
    /// Opens the task's chat and sends the task as its first message (the MCP path: no UI).
    pub async fn start_task_chat(&self, task_id: String) -> Result<Chat, AppError> {
        let TaskChatStart { chat, prompt } = self.prepare_task_chat(task_id).await?;
        let Some(text) = prompt else { return Ok(chat) };
        match self.send_chat_message(chat.id.clone(), text, vec![]).await {
            Ok(sent) => Ok(sent),
            Err(e) => {
                // Nothing was said yet: drop the chat so a retry starts clean.
                let _ = self.delete_chat(chat.id).await;
                Err(e)
            }
        }
    }

    /// Returns the task's chat, or creates it in the task's worktree and moves the task to In
    /// Progress, without sending: the caller sends `prompt`.
    pub async fn prepare_task_chat(&self, task_id: String) -> Result<TaskChatStart, AppError> {
        check_id(&task_id, "task")?;
        let tid = task_id.clone();
        let existing = self.db(move |c| Ok(chats::for_task(c, &tid)?)).await?;
        if let Some(chat) = existing {
            return Ok(TaskChatStart { chat, prompt: None });
        }

        // Worktree: slug, folder and branch derived from the task, reused if it exists.
        let tid = task_id.clone();
        let (task, repo, project) = self
            .db(move |c| {
                let t = tasks::get(c, &tid)?;
                Ok((t.clone(), repos::get(c, &t.repo_id)?, projects::get(c, &t.project_id)?))
            })
            .await?;
        let (env, t, r, p) = (self.core.env.clone(), task.clone(), repo.clone(), project.clone());
        let wt = blocking(move || {
            let slug = task_slug(&p.key, t.number, &t.title);
            let dir = dir_for(&env.worktrees_root, &r.name, &slug);
            env.git
                .ensure_worktree(Path::new(&r.path), &dir, &branch_for(&slug), t.worktree.as_ref())
                .map_err(|e| e.to_string())
        })
        .await?;

        let now = self.core.clock.now_ms();
        let (tid, wt_new) = (task_id.clone(), wt.clone());
        let input = NewChat { repo_id: Some(repo.id.clone()), title: Some(task.title.clone()), ..Default::default() };
        let (chat, created) = self
            .db(move |c| {
                let tx = c.transaction().map_err(|e| e.to_string())?;
                // Lost a race with another start: that chat wins.
                if let Some(chat) = chats::for_task(&tx, &tid)? {
                    return Ok((chat, false));
                }
                let mut t = tasks::get(&tx, &tid)?;
                if t.worktree.as_ref() != Some(&wt_new) {
                    t.worktree = Some(wt_new);
                    t.updated_at = now;
                    tasks::update(&tx, &t)?;
                }
                let chat = ops::create_for(&tx, &t.project_id, &input, Some(&tid), now)?;
                tx.commit().map_err(|e| e.to_string())?;
                Ok((chat, true))
            })
            .await?;
        if !created {
            return Ok(TaskChatStart { chat, prompt: None });
        }

        let env = self.core.env.clone();
        let task = Task { worktree: Some(wt), ..task };
        let text = blocking(move || Ok(first_message(&env, &project, &task, &repo))).await?;
        let (tid, env) = (task_id, self.core.env.clone());
        let project_id = self
            .db(move |c| {
                let t = tasks::get(c, &tid)?;
                if t.status != TaskStatus::InProgress {
                    let patch = nodal_domain::board::dto::TaskPatch { status: Some(TaskStatus::InProgress), ..Default::default() };
                    board::update_task(c, &env, &tid, &patch, now)?;
                }
                Ok(t.project_id)
            })
            .await?;
        self.core.notifier.notify(ChangeKind::Tasks, Some(&project_id));
        self.core.notifier.notify(ChangeKind::Chats, Some(&project_id));
        Ok(TaskChatStart { chat, prompt: Some(text) })
    }
}
