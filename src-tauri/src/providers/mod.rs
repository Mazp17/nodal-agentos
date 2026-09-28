//! Task providers (Linear today; Asana, Azure DevOps later) and their sync with Nodal.
//!
//! - `TaskProvider`: generic provider interface. Static dispatch through the `Provider`
//!   enum (no `dyn`); each adapter lives in its own file (`linear.rs`).
//! - `state_map`: state mapping proposal (pure).
//! - `plan`: `plan.md` materialization, criteria extraction and the closing comment.
//! - `import`: importing items as tasks.
//! - `sync`: worker (pull, outbox push, auto-import) and `sync_now`.
//! - `store`: SQL over `tasks`, `source_links`, `sync_outbox` and `runs` used by all of the
//!   above (lives here so it doesn't collide with `db/queries`).
//! - `commands`: Tauri commands.
//!
//! API for the queue (`work`): `enqueue_status` and `enqueue_comment` add outbox rows in the
//! same transaction as the transition, and `plan::closing_comment` builds the comment.

pub mod commands;
mod import;
pub mod plan;
pub mod state_map;
pub mod store;
pub mod sync;

#[cfg(test)]
mod fake;

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::domain::{ExternalState, ScopeRef};

pub use store::{enqueue_comment, enqueue_status};

/// Tauri commands reject with a display-ready string (in English).
pub type PResult<T> = Result<T, String>;

/// Moved to `nodal_domain::model::providers`; re-exported so current uses don't break.
pub use nodal_domain::model::providers::{
    ChildItem, ErrorKind, ExternalItem, ImportQuery, ItemRef, Page, ProviderError, ProviderResult, OPEN_KINDS,
};

/// Providers Nodal knows about (the rest are rejected with "Unknown provider").
pub const KNOWN_PROVIDERS: &[&str] = &["linear"];

/// Task provider interface. Adapters normalize their states to `ExtKind` so the mapping
/// heuristic (`state_map`) works for all of them.
// Private crate: the `async fn` in public traits lint (due to `Send` bounds) doesn't apply;
// dispatch is through a concrete enum, so futures are `Send` when needed.
#[allow(async_fn_in_trait)]
pub trait TaskProvider {
    /// Provider name (`"linear"`), same as `TaskSource.provider`.
    fn name(&self) -> &'static str;
    /// Validates the key; returns the user's name.
    async fn status(&self) -> ProviderResult<String>;
    /// Linkable scopes (in Linear: teams and active projects).
    async fn scopes(&self) -> ProviderResult<Vec<ScopeRef>>;
    /// Current states of the scope, in provider order.
    async fn states(&self, scope: &ScopeRef) -> ProviderResult<Vec<ExternalState>>;
    /// Provider projects that can be a rule of a link with this scope (in Linear: the team's
    /// active projects; for a link to a project, that project).
    async fn rule_projects(&self, scope: &ScopeRef) -> ProviderResult<Vec<ScopeRef>>;
    async fn list_importable(&self, query: &ImportQuery) -> ProviderResult<Page>;
    /// Full items by id. Those that no longer exist (or the key can't see) are not returned.
    async fn pull(&self, external_ids: &[String]) -> ProviderResult<Vec<ExternalItem>>;
    #[allow(dead_code)] // For the detail view (today `linear_issue_detail`) and future providers.
    async fn fetch(&self, external_id: &str) -> ProviderResult<Option<ExternalItem>> {
        Ok(self.pull(&[external_id.to_string()]).await?.into_iter().next())
    }
    /// Changes the item's state. `state_id` comes from the mapping; if it belongs to a different
    /// state group than the item's (Linear project with several teams), the adapter uses the
    /// equivalent by name/type in the item's group. Returns the resulting state.
    async fn set_state(&self, external_id: &str, state_id: &str) -> ProviderResult<ExternalState>;
    async fn comment(&self, external_id: &str, body: &str) -> ProviderResult<()>;
    /// Whether the item already has a comment containing `marker` (the outbox uses it to avoid
    /// reposting a comment that went out but wasn't marked as sent).
    async fn has_comment_with(&self, external_id: &str, marker: &str) -> ProviderResult<bool>;
}

/// Static provider dispatch.
pub enum Provider {
    Linear(linear::LinearProvider),
    #[cfg(test)]
    Fake(fake::FakeProvider),
}

macro_rules! dispatch {
    ($self:ident, $p:ident => $e:expr) => {
        match $self {
            Provider::Linear($p) => $e,
            #[cfg(test)]
            Provider::Fake($p) => $e,
        }
    };
}

impl TaskProvider for Provider {
    fn name(&self) -> &'static str {
        dispatch!(self, p => p.name())
    }
    async fn status(&self) -> ProviderResult<String> {
        dispatch!(self, p => p.status().await)
    }
    async fn scopes(&self) -> ProviderResult<Vec<ScopeRef>> {
        dispatch!(self, p => p.scopes().await)
    }
    async fn states(&self, scope: &ScopeRef) -> ProviderResult<Vec<ExternalState>> {
        dispatch!(self, p => p.states(scope).await)
    }
    async fn rule_projects(&self, scope: &ScopeRef) -> ProviderResult<Vec<ScopeRef>> {
        dispatch!(self, p => p.rule_projects(scope).await)
    }
    async fn list_importable(&self, query: &ImportQuery) -> ProviderResult<Page> {
        dispatch!(self, p => p.list_importable(query).await)
    }
    async fn pull(&self, external_ids: &[String]) -> ProviderResult<Vec<ExternalItem>> {
        dispatch!(self, p => p.pull(external_ids).await)
    }
    async fn fetch(&self, external_id: &str) -> ProviderResult<Option<ExternalItem>> {
        dispatch!(self, p => p.fetch(external_id).await)
    }
    async fn set_state(&self, external_id: &str, state_id: &str) -> ProviderResult<ExternalState> {
        dispatch!(self, p => p.set_state(external_id, state_id).await)
    }
    async fn comment(&self, external_id: &str, body: &str) -> ProviderResult<()> {
        dispatch!(self, p => p.comment(external_id, body).await)
    }
    async fn has_comment_with(&self, external_id: &str, marker: &str) -> ProviderResult<bool> {
        dispatch!(self, p => p.has_comment_with(external_id, marker).await)
    }
}

pub fn check_provider(name: &str) -> PResult<()> {
    if KNOWN_PROVIDERS.contains(&name) {
        Ok(())
    } else {
        Err(format!("Unknown provider \"{name}\"."))
    }
}

/// Ready-to-use provider, or `None` if there is no saved key.
pub async fn resolve(app: &AppHandle, name: &str) -> PResult<Option<Provider>> {
    check_provider(name)?;
    Ok(resolve_with_key(app, name).await?.map(|(p, _)| p))
}

/// Like `resolve`, plus the key read (to derive `key_hint` without reading the keychain twice).
pub async fn resolve_with_key(app: &AppHandle, name: &str) -> PResult<Option<(Provider, String)>> {
    check_provider(name)?;
    let Some(key) = app.state::<crate::secrets::Secrets>().get(name).await? else { return Ok(None) };
    match name {
        "linear" => {
            let http = app.state::<crate::linear::LinearState>().http().clone();
            Ok(Some((Provider::Linear(linear::LinearProvider::new(http, key.clone())), key)))
        }
        other => Err(format!("Unknown provider \"{other}\".")),
    }
}

pub async fn require(app: &AppHandle, name: &str) -> PResult<Provider> {
    resolve(app, name)
        .await?
        .ok_or_else(|| format!("The {name} API key is missing. Add it in Settings → Providers."))
}

// ---------- State and startup ----------

pub struct ProvidersState {
    /// `app_data_dir`: plans go in `tasks/<id>/plan.md`.
    pub data_dir: PathBuf,
    /// A single sync at a time (worker and `sync_now`), with its memory between passes.
    pub sync_lock: tokio::sync::Mutex<sync::SyncMemo>,
    /// Providers paused by rate limit or rejected key. Separate from the sync lock so
    /// `provider_status` and key changes don't wait for an ongoing pass.
    pauses: std::sync::Mutex<std::collections::HashMap<String, sync::Pause>>,
}

impl ProvidersState {
    pub fn paused(&self) -> std::collections::HashMap<String, sync::Pause> {
        self.pauses.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// Applies only the pauses a pass set, changed or lifted (`before` → `after`).
    pub fn merge_paused(
        &self,
        before: &std::collections::HashMap<String, sync::Pause>,
        after: &std::collections::HashMap<String, sync::Pause>,
    ) {
        let Ok(mut m) = self.pauses.lock() else { return };
        for (k, v) in after {
            if before.get(k) != Some(v) {
                m.insert(k.clone(), v.clone());
            }
        }
        for k in before.keys() {
            if !after.contains_key(k) {
                m.remove(k);
            }
        }
    }

    /// Active pause of a provider.
    pub fn pause_of(&self, provider: &str, now: i64) -> Option<sync::Pause> {
        self.paused().remove(provider).filter(|p| p.until > now)
    }

    /// A rate limit or a rejected key outside the sync (a rule backfill) pauses the provider
    /// just like in the sync; other errors don't.
    pub fn pause_on(&self, provider: &str, err: &ProviderError, now: i64) {
        let ms = match err.kind {
            ErrorKind::RateLimited => sync::RATE_LIMIT_PAUSE_MS,
            ErrorKind::Auth => sync::AUTH_PAUSE_MS,
            _ => return,
        };
        if let Ok(mut m) = self.pauses.lock() {
            m.insert(provider.to_string(), sync::Pause { until: now + ms, reason: err.message.clone() });
        }
    }

    /// A new (or deleted) key lifts the pause.
    pub fn clear_pause(&self, provider: &str) {
        if let Ok(mut m) = self.pauses.lock() {
            m.remove(provider);
        }
    }
}

/// Registers the state and starts the sync worker. Needs `LinearState` and the database
/// (`db::Db`) already registered; if the database didn't open, the worker does nothing.
pub fn init(app: &AppHandle) -> Result<(), String> {
    let data_dir = crate::util::paths::data_dir(app)?;
    app.manage(ProvidersState {
        data_dir,
        sync_lock: tokio::sync::Mutex::new(sync::SyncMemo::default()),
        pauses: Default::default(),
    });
    sync::spawn_worker(app.clone());
    Ok(())
}

// ---------- Utilities ----------

/// Moved to `nodal_domain::sources`; re-exported so current uses don't break.
pub use nodal_domain::sources::{iso_from_ms, key_hint};

#[cfg(test)]
mod tests;
