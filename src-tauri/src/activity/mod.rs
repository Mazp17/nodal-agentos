//! Claude Code activity in a repo. `assemble`, `external_sessions_of`, `existing_roots` and
//! `list_agents` moved to `nodal_host::claude::activity`; re-exported so current uses don't
//! break. The command stays here for now.

mod claude_sessions;

use tauri::{AppHandle, Manager};

use crate::db::{with_db, Db};

/// Moved to `nodal_domain::model::activity`; re-exported so current uses don't break.
pub use nodal_domain::model::activity::{AppRuns, ExternalSessions};
/// Only the bridge is left; nothing in this crate calls them directly anymore.
#[allow(unused_imports)]
pub use nodal_domain::model::activity::{RepoActivity, RepoSessions, SessionActivity, SubagentActivity};
pub use nodal_host::claude::activity::{existing_roots, external_sessions_of, list_agents};
#[allow(unused_imports)]
pub use nodal_host::claude::activity::assemble;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Runs launched by the app (`runs` table). If the database isn't available, nothing is marked.
async fn app_run_refs(app: &AppHandle) -> AppRuns {
    let mut refs = AppRuns::default();
    let Some(db) = app.try_state::<Db>() else { return refs };
    match with_db(&db, |c| crate::db::queries::runs::launched_refs(c)).await {
        Ok(list) => {
            for (r, s) in list {
                refs.add(r, s);
            }
        }
        Err(e) => eprintln!("activity: {e}"),
    }
    refs
}

/// Claude Code sessions started outside the app in the project's repos (`None`: every
/// project's), with a single `claude agents`.
#[tauri::command]
pub async fn external_sessions(app: AppHandle, project_id: Option<String>) -> Result<ExternalSessions, String> {
    if let Some(id) = &project_id {
        crate::util::check_id(id, "project")?;
    }
    let db = app.try_state::<Db>().ok_or("The database isn't available.")?.inner().clone();
    let repos: Vec<(String, std::path::PathBuf)> = with_db(&db, move |c| {
        if let Some(pid) = &project_id {
            crate::db::queries::projects::get(c, pid)?;
        }
        // Every project: skip archived ones (their runs don't show on Runs either).
        let live: Option<std::collections::HashSet<String>> = match project_id {
            Some(_) => None,
            None => Some(crate::db::queries::projects::list(c, false)?.into_iter().map(|p| p.id).collect()),
        };
        Ok(crate::db::queries::repos::list(c, project_id.as_deref())?
            .into_iter()
            .filter(|r| live.as_ref().is_none_or(|l| l.contains(&r.project_id)))
            .map(|r| (r.id, std::path::PathBuf::from(r.path)))
            .collect())
    })
    .await?;
    if repos.is_empty() {
        return Ok(ExternalSessions { repos: vec![], generated_at: now_ms() });
    }
    let agents = list_agents().await?;
    let refs = app_run_refs(&app).await;
    tauri::async_runtime::spawn_blocking(move || {
        let projects = crate::runs::claude_fs::claude_config_dir()
            .ok_or("Couldn't locate the Claude Code folder ($HOME is not set).")?
            .join("projects");
        Ok(external_sessions_of(&repos, &agents, &projects, &refs, now_ms(), existing_roots))
    })
    .await
    .map_err(|e| format!("Internal error reading activity: {e}"))?
}
