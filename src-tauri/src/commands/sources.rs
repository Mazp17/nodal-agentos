//! Sources Tauri commands (signatures in `src/domain/api.ts`): providers, source links, state
//! mapping, import and sync.
//!
//! `provider_status`, `provider_set_key`, `provider_clear_key`, `provider_scopes` and
//! `sync_now` work without a database, through the always-registered `SourcesHub`. The rest
//! need one, through `Arc<App>` (`state.sources` is the same hub either way).

use std::sync::Arc;

use tauri::{AppHandle, State};

use nodal_app::sources::links::{NewSourceLink, SourceLinkPatch};
use nodal_app::sources::moved::MovedAction;
use nodal_app::sources::sync::SyncReport;
use nodal_app::sources::{ProviderStatus, SourcesHub};
use nodal_app::App;
use nodal_domain::model::{ExtKind, ScopeRef, SourceLink, StateMap, Task};
use nodal_domain::sources::routing::{ImportRequest, ImportResult, ImportableItem, RulePreview};
use nodal_domain::sources::state_map::SourceStatesReport;

use super::CommandError;

/// `hub.spawn_sync_worker()` needs the database only through `hub.db`, so it's started
/// regardless of whether the app opened one (today's `providers::init`, minus the state: the
/// hub is always registered by the time this runs).
pub fn setup(h: &AppHandle, hub: &Arc<SourcesHub>) -> Result<(), String> {
    let _ = h;
    hub.spawn_sync_worker();
    Ok(())
}

// ---------- Keys and status ----------

#[tauri::command]
pub async fn provider_status(state: State<'_, Arc<SourcesHub>>, provider: String) -> Result<ProviderStatus, CommandError> {
    Ok(state.provider_status(provider).await?)
}

/// Validates the key against the provider and stores it only if valid. `None` deletes it.
#[tauri::command]
pub async fn provider_set_key(
    state: State<'_, Arc<SourcesHub>>,
    provider: String,
    key: Option<String>,
) -> Result<ProviderStatus, CommandError> {
    Ok(state.provider_set_key(provider, key).await?)
}

#[tauri::command]
pub async fn provider_clear_key(state: State<'_, Arc<SourcesHub>>, provider: String) -> Result<ProviderStatus, CommandError> {
    Ok(state.provider_clear_key(provider).await?)
}

#[tauri::command]
pub async fn provider_scopes(state: State<'_, Arc<SourcesHub>>, provider: String) -> Result<Vec<ScopeRef>, CommandError> {
    Ok(state.provider_scopes(provider).await?)
}

// ---------- Source links ----------

#[tauri::command]
pub async fn list_source_links(state: State<'_, Arc<App>>, project_id: Option<String>) -> Result<Vec<SourceLink>, CommandError> {
    Ok(state.sources.list_source_links(project_id).await?)
}

/// Creates the link with the proposed mapping (pending until `save_state_map`). If the
/// provider does not respond, the link is created anyway with an empty mapping and the
/// proposal is built later in `source_states`.
#[tauri::command]
pub async fn create_source_link(state: State<'_, Arc<App>>, input: NewSourceLink) -> Result<SourceLink, CommandError> {
    Ok(state.sources.create_source_link(input).await?)
}

#[tauri::command]
pub async fn update_source_link(
    state: State<'_, Arc<App>>,
    id: String,
    patch: SourceLinkPatch,
) -> Result<SourceLink, CommandError> {
    Ok(state.sources.update_source_link(id, patch).await?)
}

/// Disconnect: the link's tasks become local and the link is deleted (one transaction).
#[tauri::command]
pub async fn delete_source_link(state: State<'_, Arc<App>>, id: String) -> Result<(), CommandError> {
    Ok(state.sources.delete_source_link(id).await?)
}

/// Unlink: the task becomes local.
#[tauri::command]
pub async fn unlink_task(state: State<'_, Arc<App>>, task_id: String) -> Result<Task, CommandError> {
    Ok(state.sources.unlink_task(task_id).await?)
}

// ---------- State mapping ----------

#[tauri::command]
pub async fn source_states(state: State<'_, Arc<App>>, link_id: String) -> Result<SourceStatesReport, CommandError> {
    Ok(state.sources.source_states(link_id).await?)
}

/// Saves and confirms the mapping: sets `confirmed_at` and `known_states` to the current states.
#[tauri::command]
pub async fn save_state_map(
    state: State<'_, Arc<App>>,
    link_id: String,
    map: StateMap,
) -> Result<SourceLink, CommandError> {
    Ok(state.sources.save_state_map(link_id, map).await?)
}

// ---------- Import ----------

/// Items in the link's scope (up to 100), with suggested repo and an already-imported flag.
/// Empty or missing `state_kinds` = open (triage, backlog, unstarted, started).
#[tauri::command]
pub async fn provider_list_importable(
    state: State<'_, Arc<App>>,
    link_id: String,
    query: Option<String>,
    state_kinds: Option<Vec<ExtKind>>,
) -> Result<Vec<ImportableItem>, CommandError> {
    Ok(state.sources.provider_list_importable(link_id, query, state_kinds).await?)
}

#[tauri::command]
pub async fn import_tasks(
    state: State<'_, Arc<App>>,
    project_id: String,
    link_id: String,
    items: Vec<ImportRequest>,
) -> Result<ImportResult, CommandError> {
    Ok(state.sources.import_tasks(project_id, link_id, items).await?)
}

// ---------- Project rules ----------

/// Provider projects eligible as a link rule (in Linear: the active ones of its team).
#[tauri::command]
pub async fn source_rule_projects(state: State<'_, Arc<App>>, link_id: String) -> Result<Vec<ScopeRef>, CommandError> {
    Ok(state.sources.source_rule_projects(link_id).await?)
}

/// How many items the rule's backfill would bring (open + closed in the last 14 days), how
/// many are already in its repo and how many in another one (those are not moved).
#[tauri::command]
pub async fn preview_rule_import(state: State<'_, Arc<App>>, link_id: String, rule_id: String) -> Result<RulePreview, CommandError> {
    Ok(state.sources.preview_rule_import(link_id, rule_id).await?)
}

/// Backfill of a project rule: imports into the rule's repo the project's items (within the
/// link's scope) that are open or were closed in the last 14 days. Those already imported
/// in another repo are not moved: they come back in `skipped`. Runs under the sync lock so
/// it does not race the auto-import.
#[tauri::command]
pub async fn import_rule(state: State<'_, Arc<App>>, link_id: String, rule_id: String) -> Result<ImportResult, CommandError> {
    Ok(state.sources.import_rule(link_id, rule_id).await?)
}

/// Resolves the "moved to another project in the provider" notice: `move` moves it to the
/// suggested repo (rejects with an active run or a worktree), `keep` leaves it where it is.
#[tauri::command]
pub async fn resolve_moved_task(state: State<'_, Arc<App>>, task_id: String, action: MovedAction) -> Result<Task, CommandError> {
    Ok(state.sources.resolve_moved_task(task_id, action).await?)
}

// ---------- Sync ----------

/// One sync pass now (all sources, or only `link_id`).
#[tauri::command]
pub async fn sync_now(state: State<'_, Arc<SourcesHub>>, link_id: Option<String>) -> Result<SyncReport, CommandError> {
    Ok(state.sync_now(link_id).await?)
}
