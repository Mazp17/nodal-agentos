//! Occupied slots/capacity, queue and what's waiting on the user. Moved from
//! `commands::execution::work_summary`. `claude agents` now goes through `core.claude`
//! instead of the `commands::sessions::list_runs` free function.

use nodal_domain::execution::queue::work_summary as compute_summary;
use nodal_domain::execution::queue::WorkSummary;
use nodal_store::execution::runs as qruns;
use nodal_store::{rows, sources, with_db};

use crate::core::AppError;

use super::{opt_id, Execution};

impl Execution {
    /// Occupied slots/capacity, queue and what's waiting on the user (`project_id` null:
    /// everything, including foreign sessions). If `claude agents` fails, it's computed
    /// without the live sessions.
    pub async fn work_summary(&self, project_id: Option<String>) -> Result<WorkSummary, AppError> {
        let project_id = opt_id(project_id, "project")?;
        let global = project_id.is_none();
        let (runs, blocked, settings) = with_db(&self.core.db, move |c| {
            let p = project_id.as_deref();
            // Tasks that changed project in the provider are waiting on the user. Blocked tasks
            // (a failed run) aren't: they show under "Failed recently", not in "need you".
            let need = sources::moved_ids(c, p)?;
            Ok((qruns::pending_of(c, p)?, need, rows::load_settings(c)?))
        })
        .await?;
        let live = self.core.claude.list_sessions().await.unwrap_or_else(|e| {
            eprintln!("work_summary: {e}");
            Vec::new()
        });
        let now = self.core.clock.now_ms();
        let mut summary = compute_summary(&runs, &blocked, &live, settings.concurrency, global, now);
        summary.pump_error = self.pump_error.lock().unwrap_or_else(|p| p.into_inner()).clone();
        Ok(summary)
    }
}
