//! System context: legacy data import. Host diagnostics (`claude_version`, `git_version`,
//! `resolve_git_root`, `scan_git_repos`, `repo_trust`, `open_terminal_at`) need no database,
//! so they call `nodal_host` straight from the shell (`SRC/commands/host.rs`) instead of going through this facade.

use std::path::PathBuf;
use std::sync::Arc;

use std::path::Path;

use nodal_domain::diff::{CommitInfo, FileDiff};
use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::{BranchPr, ChatPr, DirEntry, PrState, Repo, RepoFile, RepoWorktree, TaskPr};
use nodal_domain::ports::Git;
use nodal_domain::util::check_id;
use nodal_store::board::{branch_prs, chat_prs, tasks};
use nodal_store::{rows, with_db};

use crate::core::{blocking, AppError, Core};

/// Legacy data import behind one facade method; `core` gives it `env.data_dir`, `clock` and
/// the shared `db`.
pub struct System {
    core: Arc<Core>,
}

impl System {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    async fn repo(&self, repo_id: String) -> Result<Repo, AppError> {
        check_id(&repo_id, "repo")?;
        let repo = with_db(&self.core.db, move |c| rows::get_repo(c, &repo_id)).await?;
        Ok(repo.ok_or("Repo not found.")?)
    }

    /// Branch checked out in the repo's folder; `None` when detached or unknown.
    pub async fn repo_branch(&self, repo_id: String) -> Result<Option<String>, AppError> {
        let repo = self.repo(repo_id).await?;
        let git = self.core.env.git.clone();
        Ok(blocking(move || Ok(git.current_branch(Path::new(&repo.path)))).await?)
    }

    /// The repo's checkouts (main first) for the files panel's picker.
    pub async fn list_repo_worktrees(&self, repo_id: String) -> Result<Vec<RepoWorktree>, AppError> {
        let repo = self.repo(repo_id).await?;
        let git = self.core.env.git.clone();
        Ok(blocking(move || git.list_worktrees(Path::new(&repo.path)).map_err(|e| e.0)).await?)
    }

    /// Runs `f` on a blocking thread with the checkout root: the repo's folder, or `worktree`
    /// when given. Trust boundary: `worktree` must be one of the repo's own worktrees
    /// (compared canonicalized), never an arbitrary path.
    async fn with_root<T: Send + 'static>(
        &self,
        repo_id: String,
        worktree: Option<String>,
        f: impl FnOnce(&dyn Git, &Path) -> Result<T, String> + Send + 'static,
    ) -> Result<T, AppError> {
        let repo = self.repo(repo_id).await?;
        let env = self.core.env.clone();
        Ok(blocking(move || {
            let root = match worktree {
                None => PathBuf::from(&repo.path),
                Some(wt) => {
                    let not_ours = || "Not a worktree of this repo.".to_string();
                    let want = env.fs.canonicalize(Path::new(&wt)).map_err(|_| not_ours())?;
                    let found = env.git.list_worktrees(Path::new(&repo.path)).map_err(|e| e.0)?.into_iter().find(|w| {
                        env.fs.canonicalize(Path::new(&w.path)).is_ok_and(|p| p == want)
                    });
                    PathBuf::from(found.ok_or_else(not_ours)?.path)
                }
            };
            f(&*env.git, &root)
        })
        .await?)
    }

    /// Every file of the checkout (relative paths) for the quick-open search.
    pub async fn list_repo_files(&self, repo_id: String, worktree: Option<String>) -> Result<Vec<String>, AppError> {
        self.with_root(repo_id, worktree, |git, root| git.list_files(root).map_err(|e| e.0)).await
    }

    /// Direct children of `dir` (relative to the checkout root) for the UI's file picker.
    pub async fn list_repo_dir(&self, repo_id: String, dir: String, worktree: Option<String>) -> Result<Vec<DirEntry>, AppError> {
        self.with_root(repo_id, worktree, move |git, root| git.list_dir(root, &dir).map_err(|e| e.0)).await
    }

    /// Uncommitted changes of the checkout (vs HEAD, untracked files included), per file with its
    /// hunks, for the files panel's "Changes" view and commit box; committed work is in `repo_commits`.
    pub async fn repo_changes(&self, repo_id: String, worktree: Option<String>) -> Result<Vec<FileDiff>, AppError> {
        self.with_root(repo_id, worktree, |git, root| {
            let (patch, _) = git.diff(root, None).map_err(|e| e.0)?;
            Ok(nodal_domain::diff::parse(&patch))
        })
        .await
    }

    /// The checkout's commits, newest first: a worktree's since it branched off, the repo's
    /// own checkout its latest ones.
    pub async fn repo_commits(&self, repo_id: String, worktree: Option<String>) -> Result<Vec<CommitInfo>, AppError> {
        self.with_base(repo_id, worktree, true, |git, root, base| git.log(root, base.as_deref()).map_err(|e| e.0)).await
    }

    /// Commits the checkout's changes with `message`: all of them, or only `paths` when given.
    pub async fn repo_commit(&self, repo_id: String, worktree: Option<String>, message: String, paths: Vec<String>) -> Result<CommitInfo, AppError> {
        let message = message.trim().to_string();
        if message.is_empty() {
            return Err("Write a commit message.".into());
        }
        let repo = self.repo(repo_id.clone()).await?;
        let info = self.with_root(repo_id, worktree, move |git, root| git.commit_all(root, &message, &paths).map_err(|e| e.0)).await?;
        self.core.notifier.notify(ChangeKind::Chats, Some(&repo.project_id));
        Ok(info)
    }

    /// Pushes the checkout's branch and opens a PR against its base (see `with_base`; the repo's
    /// own checkout uses the remote's default branch). Returns the PR URL.
    pub async fn repo_open_pr(
        &self,
        repo_id: String,
        worktree: Option<String>,
        title: String,
        body: String,
        chat_id: Option<String>,
    ) -> Result<String, AppError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err("Write a title for the pull request.".into());
        }
        let own_checkout = worktree.is_none();
        let (rid, wt) = (repo_id.clone(), worktree.clone());
        let url = self.with_base(repo_id, worktree, false, move |git, root, base| {
            let base = if own_checkout { git.default_branch(root) } else { base };
            let base = base.ok_or(if own_checkout {
                "Open a PR from a worktree branch."
            } else {
                "Couldn't tell which branch to open the PR against."
            })?;
            let branch = git.current_branch(root).ok_or("HEAD is detached: check out a branch to open a PR.")?;
            if branch == base {
                return Err(format!("`{branch}` is the base branch: open the PR from another branch."));
            }
            git.open_pr(root, &base, &branch, &title, &body).map_err(|e| e.0)
        })
        .await?;
        if let Some(number) = crate::board::pr::pr_number(&url) {
            self.record_chat_pr(chat_id, rid.clone(), wt.clone(), number, url.clone()).await;
            if let Some(wt) = wt {
                let pr = TaskPr { url: url.clone(), number, state: PrState::Open, head: None };
                if let Err(e) = self.link_task_pr(rid, wt, pr).await {
                    eprintln!("tasks: couldn't link the PR {url}: {e}");
                }
            }
        }
        Ok(url)
    }

    /// Records a PR opened or linked from chat `chat_id` (if any); failures are logged only.
    async fn record_chat_pr(&self, chat_id: Option<String>, repo_id: String, worktree: Option<String>, number: i64, url: String) {
        let Some(chat_id) = chat_id else { return };
        let res: Result<(), AppError> = async {
            check_id(&chat_id, "chat")?;
            let branch = self.with_root(repo_id.clone(), worktree, |git, root| Ok(git.current_branch(root))).await?;
            let pr = ChatPr { repo_id, url, number, branch, created_at: self.core.clock.now_ms() };
            let project = with_db(&self.core.db, move |c| {
                let chat = nodal_store::chats::get(c, &chat_id)?;
                Ok(chat_prs::insert_if_new(c, &chat_id, &pr)?.then_some(chat.project_id))
            })
            .await?;
            if let Some(p) = project {
                self.core.notifier.notify(ChangeKind::Chats, Some(&p));
            }
            Ok(())
        }
        .await;
        if let Err(e) = res {
            eprintln!("chats: couldn't record the PR: {e}");
        }
    }

    /// The PRs a chat opened or linked, newest first.
    pub async fn chat_prs(&self, chat_id: String) -> Result<Vec<ChatPr>, AppError> {
        check_id(&chat_id, "chat")?;
        Ok(with_db(&self.core.db, move |c| chat_prs::list(c, &chat_id)).await?)
    }

    /// Links a PR just opened from `worktree` to the task working there, if any.
    async fn link_task_pr(&self, repo_id: String, worktree: String, pr: TaskPr) -> Result<(), AppError> {
        let (env, now) = (self.core.env.clone(), self.core.clock.now_ms());
        let project = with_db(&self.core.db, move |c| {
            let repo = rows::get_repo(c, &repo_id)?.ok_or_else(|| nodal_store::StoreError::Invalid("Repo not found.".into()))?;
            let canon = |p: &str| env.fs.canonicalize(Path::new(p)).ok();
            let here = canon(&worktree);
            let task = tasks::list(c, Some(&repo.project_id))?
                .into_iter()
                .find(|t| t.repo_id == repo.id && t.worktree.as_ref().is_some_and(|w| here.is_some() && canon(&w.path) == here));
            let Some(task) = task else { return Ok(None) };
            let tx = c.transaction()?;
            crate::board::pr::link_pr(&tx, &task, Some(pr), now).map_err(nodal_store::StoreError::Invalid)?;
            tx.commit()?;
            Ok(Some(task.project_id))
        })
        .await?;
        if let Some(p) = project {
            self.core.notifier.notify(ChangeKind::Tasks, Some(&p));
        }
        Ok(())
    }

    /// The checkout's branch, unless HEAD is detached or it is the default or base branch (no PR
    /// of its own).
    async fn pr_branch(&self, repo_id: String, worktree: Option<String>) -> Result<Option<String>, AppError> {
        self.with_base(repo_id, worktree, false, |git, root, base| {
            let branch = git.current_branch(root);
            Ok(branch.filter(|b| git.default_branch(root).as_ref() != Some(b) && base.as_ref() != Some(b)))
        })
        .await
    }

    /// The checkout's PR: found by branch with `gh`, or linked by hand. Looks `gh` up when `force`
    /// or the stored one is over `CHECK_INTERVAL_MS` old; `gh` failing (missing, logged out) is
    /// logged and the stored PR is returned.
    pub async fn repo_pr(&self, repo_id: String, worktree: Option<String>, force: Option<bool>) -> Result<Option<BranchPr>, AppError> {
        let Some(branch) = self.pr_branch(repo_id.clone(), worktree.clone()).await? else { return Ok(None) };
        let (now, force) = (self.core.clock.now_ms(), force.unwrap_or(false));
        let (rid, b) = (repo_id.clone(), branch.clone());
        let stored = with_db(&self.core.db, move |c| branch_prs::get(c, &rid, &b)).await?;
        let manual = stored.as_ref().is_some_and(|(p, _)| p.manual);
        let fresh = stored.as_ref().is_some_and(|(_, at)| now - at < crate::board::pr::CHECK_INTERVAL_MS);
        if !force && (manual || fresh) {
            return Ok(stored.map(|s| s.0));
        }
        let (b, url) = (branch.clone(), stored.as_ref().filter(|_| manual).map(|(p, _)| p.url.clone()));
        let found = self
            .with_root(repo_id.clone(), worktree, move |git, root| {
                Ok(match url {
                    Some(url) => git.pr_view(root, &url).map(Some),
                    None => git.pr_for_branch(root, &b),
                }
                .map_err(|e| e.0))
            })
            .await?;
        let (rid, b) = (repo_id, branch.clone());
        match found {
            Ok(Some(t)) => {
                let pr = BranchPr { url: t.url, number: t.number, state: t.state, head: t.head, manual, branch };
                let saved = pr.clone();
                with_db(&self.core.db, move |c| branch_prs::upsert(c, &rid, &saved, now)).await?;
                Ok(Some(pr))
            }
            Ok(None) => {
                with_db(&self.core.db, move |c| branch_prs::delete(c, &rid, &b)).await?;
                Ok(None)
            }
            Err(e) => {
                eprintln!("pr: couldn't look up the PR of {branch}: {e}");
                Ok(stored.map(|s| s.0))
            }
        }
    }

    /// Links PR `pr` (number or URL) to the checkout's branch, and to the task working there.
    pub async fn repo_link_pr(
        &self,
        repo_id: String,
        worktree: Option<String>,
        pr: String,
        chat_id: Option<String>,
    ) -> Result<BranchPr, AppError> {
        let Some(branch) = self.pr_branch(repo_id.clone(), worktree.clone()).await? else {
            return Err("Check out a branch to link a PR.".into());
        };
        let pr = pr.trim().to_string();
        let t = self.with_root(repo_id.clone(), worktree.clone(), move |git, root| git.pr_view(root, &pr).map_err(|e| e.0)).await?;
        let linked = BranchPr { url: t.url, number: t.number, state: t.state, head: t.head, manual: true, branch };
        let (rid, saved, now) = (repo_id.clone(), linked.clone(), self.core.clock.now_ms());
        with_db(&self.core.db, move |c| branch_prs::upsert(c, &rid, &saved, now)).await?;
        self.record_chat_pr(chat_id, repo_id.clone(), worktree.clone(), linked.number, linked.url.clone()).await;
        if let Some(wt) = worktree {
            let pr = TaskPr { url: linked.url.clone(), number: linked.number, state: linked.state, head: linked.head.clone() };
            if let Err(e) = self.link_task_pr(repo_id, wt, pr).await {
                eprintln!("tasks: couldn't link the PR {}: {e}", linked.url);
            }
        }
        Ok(linked)
    }

    /// Forgets the PR stored for the checkout's branch (an auto-found one is found again).
    pub async fn repo_unlink_pr(&self, repo_id: String, worktree: Option<String>) -> Result<(), AppError> {
        let Some(branch) = self.pr_branch(repo_id.clone(), worktree).await? else { return Ok(()) };
        Ok(with_db(&self.core.db, move |c| branch_prs::delete(c, &repo_id, &branch)).await?)
    }

    /// A PR's comments, reviews and checks, as `gh` reports them.
    pub async fn repo_pr_details(&self, repo_id: String, worktree: Option<String>, number: i64) -> Result<serde_json::Value, AppError> {
        self.with_root(repo_id, worktree, move |git, root| git.pr_details(root, number).map_err(|e| e.0)).await
    }

    /// The last 200 lines of a GitHub Actions run's failed-step log.
    pub async fn repo_check_log(&self, repo_id: String, worktree: Option<String>, run_id: String) -> Result<String, AppError> {
        self.with_root(repo_id, worktree, move |git, root| git.check_log(root, &run_id).map_err(|e| e.0)).await
    }

    /// Merges PR `number` on GitHub, then re-reads the checkout's PR so a task working there
    /// follows it (merged → Done) on its next refresh.
    pub async fn repo_merge_pr(&self, repo_id: String, worktree: Option<String>, number: i64, method: String) -> Result<(), AppError> {
        self.with_root(repo_id.clone(), worktree.clone(), move |git, root| git.merge_pr(root, number, &method).map_err(|e| e.0)).await?;
        if let Err(e) = self.repo_pr(repo_id, worktree, Some(true)).await {
            eprintln!("pr: couldn't re-read the PR after merging #{number}: {e}");
        }
        Ok(())
    }

    /// One commit's files with their hunks.
    pub async fn repo_commit_diff(&self, repo_id: String, worktree: Option<String>, sha: String) -> Result<Vec<FileDiff>, AppError> {
        self.with_root(repo_id, worktree, move |git, root| {
            let patch = git.show_commit(root, &sha).map_err(|e| e.0)?;
            Ok(nodal_domain::diff::parse(&patch))
        })
        .await
    }

    /// `with_root` plus where the checkout branched off: `None` for the repo's own checkout; for
    /// a worktree, its task's base, else the branch checked out in the repo's folder. With
    /// `for_diff`, a task whose PR merged counts from the PR's head (`Task::diff_base`).
    async fn with_base<T: Send + 'static>(
        &self,
        repo_id: String,
        worktree: Option<String>,
        for_diff: bool,
        f: impl FnOnce(&dyn Git, &Path, Option<String>) -> Result<T, String> + Send + 'static,
    ) -> Result<T, AppError> {
        let repo = self.repo(repo_id.clone()).await?;
        let task_bases = match worktree {
            None => Vec::new(),
            Some(_) => {
                let (rid, pid) = (repo.id.clone(), repo.project_id.clone());
                with_db(&self.core.db, move |c| {
                    Ok(tasks::list(c, Some(&pid))?
                        .into_iter()
                        .filter(|t| t.repo_id == rid)
                        .filter_map(|t| {
                            let base = if for_diff { t.diff_base() } else { t.worktree.as_ref().map(|w| w.base.as_str()) };
                            Some((t.worktree.as_ref()?.path.clone(), base?.to_string()))
                        })
                        .collect::<Vec<_>>())
                })
                .await?
            }
        };
        let (env, main) = (self.core.env.clone(), PathBuf::from(&repo.path));
        let is_wt = worktree.is_some();
        self.with_root(repo_id, worktree, move |git, root| {
            let base = if is_wt {
                let canon = |p: &Path| env.fs.canonicalize(p).ok();
                let here = canon(root);
                task_bases
                    .into_iter()
                    .find(|(p, _)| here.is_some() && canon(Path::new(p)) == here)
                    .map(|(_, b)| b)
                    .or_else(|| git.current_branch(&main))
            } else {
                None
            };
            f(git, root, base)
        })
        .await
    }

    /// A repo file's text for the viewer (`path` relative to the checkout root).
    pub async fn read_repo_file(&self, repo_id: String, path: String, worktree: Option<String>) -> Result<RepoFile, AppError> {
        self.with_root(repo_id, worktree, move |git, root| git.read_file(root, &path).map_err(|e| e.0)).await
    }

    /// Saves an edited repo file (see `Git::write_file`).
    pub async fn write_repo_file(
        &self,
        repo_id: String,
        path: String,
        content: String,
        version: String,
        force: bool,
        worktree: Option<String>,
    ) -> Result<RepoFile, AppError> {
        self.with_root(repo_id, worktree, move |git, root| {
            git.write_file(root, &path, &content, &version, force).map_err(|e| e.0)
        })
        .await
    }

    /// Discards one file's uncommitted changes (see `Git::revert_file`).
    pub async fn revert_repo_file(&self, repo_id: String, path: String, worktree: Option<String>) -> Result<(), AppError> {
        self.with_root(repo_id, worktree, move |git, root| git.revert_file(root, &path).map_err(|e| e.0)).await
    }

    /// "Import data from a previous version…": copies `folder` into a dated backup under the
    /// data dir, then imports it inside one transaction (`nodal_store::legacy`). Same as
    /// today's `import_legacy_data` command, minus the Tauri app handle/`State<Db>` plumbing:
    /// `data_dir` comes from `core.env`, `now` from `core.clock`, the connection from
    /// `core.db`.
    pub async fn import_legacy(&self, folder: String) -> Result<nodal_store::legacy::LegacyImportReport, AppError> {
        let data_dir = self.core.env.data_dir.clone();
        let now = self.core.clock.now_ms();
        let src = PathBuf::from(folder.trim());
        let dd = data_dir.clone();
        // The copy doesn't take the database lock.
        let (backup_dir, skipped) = tokio::task::spawn_blocking(move || nodal_store::legacy::backup(&src, &dd, now))
            .await
            .map_err(|e| format!("Backup task failed: {e}"))??;
        let db = self.core.db.clone();
        let mut report = nodal_store::with_db(&db, move |conn| {
            nodal_store::legacy::import_backup(conn, &backup_dir, &data_dir, now).map_err(nodal_store::DbError::Invalid)
        })
        .await?;
        report.skipped.splice(0..0, skipped);
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
