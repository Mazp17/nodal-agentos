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
mod linear;
pub mod plan;
pub mod state_map;
pub mod store;
pub mod sync;

#[cfg(test)]
mod fake;

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::domain::{ExtKind, ExtProject, ExternalState, Priority, ScopeRef};

pub use store::{enqueue_comment, enqueue_status};

/// Tauri commands reject with a display-ready string (in English).
pub type PResult<T> = Result<T, String>;

/// Provider error class: decides what the outbox does with the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Network, 5xx: retry with backoff (up to `sync::MAX_ATTEMPTS`).
    Transient,
    /// Retry later and stop draining this provider for this pass.
    RateLimited,
    /// Missing or invalid key: same as `RateLimited`.
    Auth,
    /// The provider rejected the write (deleted item, no permission, invalid input): retrying
    /// makes no sense.
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub kind: ErrorKind,
    pub message: String,
}

impl ProviderError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<ProviderError> for String {
    fn from(e: ProviderError) -> Self {
        e.message
    }
}

impl From<crate::linear::LinearError> for ProviderError {
    fn from(e: crate::linear::LinearError) -> Self {
        use crate::linear::LinearError as L;
        let kind = match &e {
            L::MissingKey | L::InvalidKey => ErrorKind::Auth,
            L::RateLimited => ErrorKind::RateLimited,
            // The classification comes from the HTTP status / `extensions.code` (`linear::model`).
            L::Network(_) | L::Keychain(_) | L::Unavailable(_) => ErrorKind::Transient,
            L::Api(_) => ErrorKind::Permanent,
        };
        ProviderError::new(kind, e.to_string())
    }
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// Providers Nodal knows about (the rest are rejected with "Unknown provider").
pub const KNOWN_PROVIDERS: &[&str] = &["linear"];

/// "Open" state kinds: the default for the importable listing and for auto-import.
pub const OPEN_KINDS: &[ExtKind] = &[ExtKind::Triage, ExtKind::Backlog, ExtKind::Unstarted, ExtKind::Started];

// ---------- Generic model ----------

/// Lightweight reference to another item (the parent).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemRef {
    pub external_id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
}

/// Sub-item (sub-issue), with its state to mark the ones already done.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildItem {
    pub external_id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state: ExternalState,
}

/// A provider item (Linear issue, Asana task, ADO work item).
/// In the importable listing `description_md`, `parent`, `children` and `assignee` may
/// be empty: importing fetches the full item again with `pull`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalItem {
    /// Stable provider id (UUID in Linear). Unique per provider.
    pub external_id: String,
    /// Human-readable id (`ENG-142`).
    pub identifier: String,
    pub url: String,
    pub title: String,
    pub description_md: Option<String>,
    pub state: ExternalState,
    /// Scopes it belongs to (team and, if any, project).
    pub scopes: Vec<ScopeRef>,
    pub parent: Option<ItemRef>,
    pub children: Vec<ChildItem>,
    pub labels: Vec<String>,
    /// Assignee's display name in the provider (informational).
    pub assignee: Option<String>,
    pub priority: Priority,
    /// ISO 8601, as given by the provider.
    pub updated_at: String,
    /// ISO 8601. `None` if the provider doesn't report it.
    pub created_at: Option<String>,
    /// ISO 8601: when it was completed or canceled (`None` if open or unknown).
    pub closed_at: Option<String>,
}

impl ExternalItem {
    /// Provider project it belongs to (`project` scope), if any.
    pub fn project(&self) -> Option<ExtProject> {
        self.scopes
            .iter()
            .find(|s| s.kind == "project")
            .map(|s| ExtProject { id: s.id.clone(), name: s.name.clone() })
    }
}

/// Query for the importable listing.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportQuery {
    pub scope: ScopeRef,
    /// Free text: title or identifier.
    pub text: Option<String>,
    /// Empty = all kinds.
    pub state_kinds: Vec<ExtKind>,
    /// Only items created after this instant (epoch ms). Used by auto-import.
    pub created_after: Option<i64>,
    /// Only items from this provider project (within `scope`): project rules.
    pub project_id: Option<String>,
    /// Besides `state_kinds`, the ones completed or canceled less than this many days ago
    /// (backfill of a project rule).
    pub closed_within_days: Option<u32>,
    pub cursor: Option<String>,
}

impl ImportQuery {
    pub fn new(scope: ScopeRef) -> Self {
        ImportQuery {
            scope,
            text: None,
            state_kinds: OPEN_KINDS.to_vec(),
            created_after: None,
            project_id: None,
            closed_within_days: None,
            cursor: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Page {
    pub items: Vec<ExternalItem>,
    pub next_cursor: Option<String>,
}

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

/// Shorter keys show no hint: the last 4 would be too much of the key.
const KEY_HINT_MIN_CHARS: usize = 12;

/// Last 4 characters of the key to recognize it in the UI; never the whole key.
pub fn key_hint(key: &str) -> Option<String> {
    let chars: Vec<char> = key.trim().chars().collect();
    (chars.len() >= KEY_HINT_MIN_CHARS).then(|| chars[chars.len() - 4..].iter().collect())
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

/// Epoch ms → `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC), without depending on chrono.
pub fn iso_from_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Howard Hinnant's "civil from days" algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z", tod / 3600, tod % 3600 / 60, tod % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_from_ms_matches_known_dates() {
        assert_eq!(iso_from_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_ms(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(iso_from_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn linear_errors_classify_by_status_and_code() {
        use crate::linear::model::interpret_response;
        let kind = |status: u16, body: &str| {
            ProviderError::from(interpret_response::<serde_json::Value>(status, body).unwrap_err()).kind
        };
        assert_eq!(kind(502, "<html>"), ErrorKind::Transient);
        assert_eq!(kind(200, r#"{"data":null}"#), ErrorKind::Transient);
        assert_eq!(kind(429, ""), ErrorKind::RateLimited);
        assert_eq!(kind(401, ""), ErrorKind::Auth);
        let gql = |code: &str, msg: &str| {
            format!(r#"{{"errors":[{{"message":"{msg}","extensions":{{"code":"{code}"}}}}]}}"#)
        };
        assert_eq!(kind(400, &gql("AUTHENTICATION_ERROR", "x")), ErrorKind::Auth);
        assert_eq!(kind(400, &gql("RATELIMITED", "x")), ErrorKind::RateLimited);
        assert_eq!(kind(500, &gql("INTERNAL_SERVER_ERROR", "boom")), ErrorKind::Transient);
        // The text doesn't decide: a rejection that mentions "unavailable" is still permanent.
        assert_eq!(kind(400, &gql("INVALID_INPUT", "state unavailable for this team")), ErrorKind::Permanent);
        assert_eq!(kind(400, &gql("FORBIDDEN", "no")), ErrorKind::Permanent);
    }

    #[test]
    fn unknown_provider_is_rejected() {
        assert!(check_provider("linear").is_ok());
        assert!(check_provider("jira").unwrap_err().contains("Unknown provider"));
    }
}
