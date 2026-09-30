//! Sources context: task providers (Linear today), sync and their DB-free hub.

#[cfg(test)]
mod fake;
pub mod import;
pub mod keys;
pub mod links;
pub mod moved;
pub mod registry;
pub mod sync;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::runtime::Handle;

use nodal_domain::model::events::ChangeKind;
use nodal_domain::model::providers::{ExternalItem, ImportQuery, ProviderError};
use nodal_domain::model::{ExtKind, RepoRule, ScopeRef, SourceLink, StateMap, Task};
use nodal_domain::ports::{ChangeNotifier, Clock, PlanFiles, TaskProvider};
use nodal_domain::sources::plan_render::{PLANS_DIR, PLAN_FILE};
use nodal_domain::sources::routing::{
    backfill_keeps, rule_query, BackfillPlan, ImportRequest, ImportResult, ImportableItem, RulePreview, Skipped,
    BACKFILL_MAX_PAGES,
};
use nodal_domain::sources::state_map::{self, SourceStatesReport};
use nodal_domain::sources::{iso_from_ms, key_hint};
use nodal_domain::util::new_id;
use nodal_store::{sources as store, Conn as Connection, Db, StoreError};

use links::{check_link_repos, prepare_rules, NewSourceLink, SourceLinkPatch};
use moved::MovedAction;

use crate::core::{AppError, HubDeps};

/// `<data_dir>/tasks/<id>/plan.md` (same path nodal-host's `plans::plan_path` computes; kept
/// here too since nodal-app can't depend on nodal-host).
pub(crate) fn plan_path(data_dir: &Path, task_id: &str) -> PathBuf {
    data_dir.join(PLANS_DIR).join(task_id).join(PLAN_FILE)
}

/// Was a local `#[allow(async_fn_in_trait)]` trait with static dispatch through a `Provider`
/// enum. Now the frozen, object-safe port; `Provider` is `Arc<dyn TaskProvider>`.
pub type Provider = Arc<dyn TaskProvider>;

/// Cap on the importable listing per call (pages of 25).
const LIST_MAX_PAGES: usize = 4;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub provider: String,
    pub has_key: bool,
    /// User name if the key is valid.
    pub viewer: Option<String>,
    pub error: Option<String>,
    /// Last 4 characters of the stored key (never the key).
    pub key_hint: Option<String>,
    /// Automatic sync is paused until this instant (epoch ms) due to a rate limit or a
    /// rejected key; `pause_reason` says why.
    pub paused_until: Option<i64>,
    pub pause_reason: Option<String>,
}

/// Providers, keys and sync state, usable without a database (today's `ProvidersState` +
/// `Secrets` + `LinearState`): `SourcesHub::new` never fails.
pub struct SourcesHub {
    pub db: Option<Db>,
    pub data_dir: PathBuf,
    rt: Handle,
    clock: Arc<dyn Clock>,
    pub secrets: keys::Secrets,
    pub providers: registry::ProviderRegistry,
    notifier: Arc<dyn ChangeNotifier>,
    plans: Arc<dyn PlanFiles>,
    /// A single sync at a time (worker and `sync_now`), with its memory between passes.
    sync_lock: tokio::sync::Mutex<sync::SyncMemo>,
    /// Providers paused by rate limit or rejected key. Separate from the sync lock so
    /// `provider_status` and key changes don't wait for an ongoing pass.
    pauses: Mutex<HashMap<String, sync::Pause>>,
}

impl SourcesHub {
    pub fn new(d: HubDeps) -> Arc<Self> {
        Arc::new(Self {
            db: d.db,
            data_dir: d.data_dir,
            rt: d.rt,
            clock: d.clock,
            secrets: d.secrets,
            providers: d.providers,
            notifier: d.notifier,
            plans: d.plans,
            sync_lock: tokio::sync::Mutex::new(sync::SyncMemo::default()),
            pauses: Mutex::default(),
        })
    }

    // ---------- Db access ----------

    /// Same message the shell used when `Db` wasn't managed as Tauri state.
    fn require_db(&self) -> Result<Db, AppError> {
        self.db.clone().ok_or_else(|| AppError::Message("The database is not available.".into()))
    }

    /// Runs `f` with the connection; `sources`'s own logic already returns display-ready
    /// messages (`Result<_, String>`), converted into `AppError` the same way `Board::db` does.
    async fn db<T, F>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, String> + Send + 'static,
    {
        let db = self.require_db()?;
        Ok(nodal_store::with_db(&db, move |c| f(c).map_err(StoreError::Invalid)).await?)
    }

    async fn load_link(&self, link_id: &str) -> Result<SourceLink, AppError> {
        let link_id = link_id.to_string();
        self.db(move |c| Ok(store::get_link(c, &link_id)?)).await
    }

    // ---------- Providers, keys and pauses ----------

    pub fn check_provider(&self, name: &str) -> Result<(), String> {
        if self.providers.names().contains(&name) {
            Ok(())
        } else {
            Err(format!("Unknown provider \"{name}\"."))
        }
    }

    /// Ready-to-use provider, or `None` if there is no saved key.
    async fn resolve(&self, name: &str) -> Result<Option<Provider>, String> {
        Ok(self.resolve_with_key(name).await?.map(|(p, _)| p))
    }

    /// Like `resolve`, plus the key read (to derive `key_hint` without reading the keychain
    /// twice).
    async fn resolve_with_key(&self, name: &str) -> Result<Option<(Provider, String)>, String> {
        self.check_provider(name)?;
        let Some(key) = self.secrets.get(name).await? else { return Ok(None) };
        let factory = self.providers.get(name).ok_or_else(|| format!("Unknown provider \"{name}\"."))?;
        Ok(Some((factory.build(key.clone()), key)))
    }

    async fn require(&self, name: &str) -> Result<Provider, String> {
        self.resolve(name)
            .await?
            .ok_or_else(|| format!("The {name} API key is missing. Add it in Settings → Providers."))
    }

    fn paused(&self) -> HashMap<String, sync::Pause> {
        self.pauses.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// Applies only the pauses a pass set, changed or lifted (`before` → `after`).
    fn merge_paused(&self, before: &HashMap<String, sync::Pause>, after: &HashMap<String, sync::Pause>) {
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
    fn pause_of(&self, provider: &str, now: i64) -> Option<sync::Pause> {
        self.paused().remove(provider).filter(|p| p.until > now)
    }

    /// A rate limit or a rejected key outside the sync (a rule backfill) pauses the provider
    /// just like in the sync; other errors don't.
    fn pause_on(&self, provider: &str, err: &ProviderError, now: i64) {
        let ms = match err.kind {
            nodal_domain::model::providers::ErrorKind::RateLimited => sync::RATE_LIMIT_PAUSE_MS,
            nodal_domain::model::providers::ErrorKind::Auth => sync::AUTH_PAUSE_MS,
            _ => return,
        };
        if let Ok(mut m) = self.pauses.lock() {
            m.insert(provider.to_string(), sync::Pause { until: now + ms, reason: err.message.clone() });
        }
    }

    /// A new (or deleted) key lifts the pause.
    fn clear_pause(&self, provider: &str) {
        if let Ok(mut m) = self.pauses.lock() {
            m.remove(provider);
        }
    }

    pub async fn provider_status(&self, provider: String) -> Result<ProviderStatus, AppError> {
        let p = self.resolve_with_key(&provider).await?;
        let mut out = ProviderStatus {
            key_hint: p.as_ref().and_then(|(_, k)| key_hint(k)),
            has_key: p.is_some(),
            viewer: None,
            error: None,
            paused_until: None,
            pause_reason: None,
            provider,
        };
        if let Some(pause) = self.pause_of(&out.provider, self.clock.now_ms()) {
            out.paused_until = Some(pause.until);
            out.pause_reason = Some(pause.reason);
        }
        if let Some((p, _)) = p {
            match p.status().await {
                Ok(v) => out.viewer = Some(v),
                Err(err) => out.error = Some(err.message),
            }
        }
        Ok(out)
    }

    /// Validates the key against the provider and stores it only if valid. `None` deletes it.
    pub async fn provider_set_key(&self, provider: String, key: Option<String>) -> Result<ProviderStatus, AppError> {
        self.check_provider(&provider)?;
        let Some(key) = key.map(|k| k.trim().to_string()) else {
            return self.provider_clear_key(provider).await;
        };
        if key.is_empty() {
            return Err("The API key is empty.".into());
        }
        let factory = self.providers.get(&provider).ok_or_else(|| format!("Unknown provider \"{provider}\"."))?;
        let p = factory.build(key.clone());
        let viewer = p.status().await?;
        self.secrets.set(&provider, &key).await?;
        self.clear_pause(&provider);
        let key_hint = key_hint(&key);
        Ok(ProviderStatus { provider, has_key: true, viewer: Some(viewer), error: None, key_hint, paused_until: None, pause_reason: None })
    }

    pub async fn provider_clear_key(&self, provider: String) -> Result<ProviderStatus, AppError> {
        self.check_provider(&provider)?;
        self.secrets.delete(&provider).await?;
        self.clear_pause(&provider);
        Ok(ProviderStatus { provider, has_key: false, viewer: None, error: None, key_hint: None, paused_until: None, pause_reason: None })
    }

    pub async fn provider_scopes(&self, provider: String) -> Result<Vec<ScopeRef>, AppError> {
        Ok(self.require(&provider).await?.scopes().await?)
    }

    // ---------- Source links ----------

    pub async fn list_source_links(&self, project_id: Option<String>) -> Result<Vec<SourceLink>, AppError> {
        self.db(move |c| Ok(store::list_links(c, project_id.as_deref())?)).await
    }

    /// Creates the link with the proposed mapping (pending until `save_state_map`). If the
    /// provider does not respond, the link is created anyway with an empty mapping and the
    /// proposal is built later in `source_states`.
    pub async fn create_source_link(&self, input: NewSourceLink) -> Result<SourceLink, AppError> {
        self.check_provider(&input.provider)?;
        let states = match self.resolve(&input.provider).await? {
            Some(p) => p.states(&input.scope).await.unwrap_or_else(|err| {
                eprintln!("create_source_link: states: {err}");
                Vec::new()
            }),
            None => Vec::new(),
        };
        let now = self.clock.now_ms();
        let link = SourceLink {
            id: new_id('s', now),
            project_id: input.project_id,
            provider: input.provider,
            scope: input.scope,
            default_repo_id: input.default_repo_id,
            repo_rules: prepare_rules(&[], input.repo_rules, now)?,
            state_map: if states.is_empty() { StateMap::default() } else { state_map::propose(&states) },
            auto_import: input.auto_import,
            created_at: now,
            last_synced_at: None,
            last_sync_error: None,
            pending_state_changes: None,
        };
        let out = self
            .db(move |c| {
                if !store::project_exists(c, &link.project_id)? {
                    return Err("Project not found.".to_string());
                }
                check_link_repos(c, &link)?;
                store::insert_link(c, &link)?;
                Ok(link)
            })
            .await?;
        self.notifier.notify(ChangeKind::Sources, Some(&out.project_id));
        Ok(out)
    }

    pub async fn update_source_link(&self, id: String, patch: SourceLinkPatch) -> Result<SourceLink, AppError> {
        let now = self.clock.now_ms();
        let out = self
            .db(move |c| {
                let mut link = store::get_link(c, &id)?;
                if let Some(r) = patch.default_repo_id {
                    link.default_repo_id = r;
                }
                if let Some(r) = patch.repo_rules {
                    link.repo_rules = prepare_rules(&link.repo_rules, r, now)?;
                }
                if let Some(a) = patch.auto_import {
                    link.auto_import = a;
                }
                check_link_repos(c, &link)?;
                store::save_link(c, &link)?;
                Ok(link)
            })
            .await?;
        self.notifier.notify(ChangeKind::Sources, Some(&out.project_id));
        Ok(out)
    }

    /// Disconnect: the link's tasks become local and the link is deleted (one transaction).
    pub async fn delete_source_link(&self, id: String) -> Result<(), AppError> {
        let now = self.clock.now_ms();
        self.db(move |c| Ok(store::disconnect_link(c, &id, now).map(|_| ())?)).await?;
        self.notifier.notify_all(&[ChangeKind::Sources, ChangeKind::Tasks], None);
        Ok(())
    }

    /// Unlink: the task becomes local.
    pub async fn unlink_task(&self, task_id: String) -> Result<Task, AppError> {
        let now = self.clock.now_ms();
        let out = self
            .db(move |c| {
                let tx = c.transaction().map_err(|e| e.to_string())?;
                let t = store::unlink_task(&tx, &task_id, now)?;
                tx.commit().map_err(|e| e.to_string())?;
                Ok(t)
            })
            .await?;
        self.notifier.notify(ChangeKind::Tasks, Some(&out.project_id));
        Ok(out)
    }

    // ---------- State mapping ----------

    pub async fn source_states(&self, link_id: String) -> Result<SourceStatesReport, AppError> {
        let link = self.load_link(&link_id).await?;
        let states = self.require(&link.provider).await?.states(&link.scope).await?;
        Ok(state_map::report(&link.state_map, states))
    }

    /// Saves and confirms the mapping: sets `confirmed_at` and `known_states` to the current
    /// states.
    pub async fn save_state_map(&self, link_id: String, map: StateMap) -> Result<SourceLink, AppError> {
        let mut link = self.load_link(&link_id).await?;
        let states = self.require(&link.provider).await?.states(&link.scope).await?;
        state_map::validate(&map, &states)?;
        link.state_map = StateMap { confirmed_at: Some(self.clock.now_ms()), known_states: states, ..map };
        let out = self
            .db(move |c| {
                // Only the mapping: an `update_source_link` made while on the network is not
                // overwritten.
                store::save_state_map(c, &link.id, &link.state_map)?;
                Ok(store::get_link(c, &link.id)?)
            })
            .await?;
        self.notifier.notify(ChangeKind::Sources, Some(&out.project_id));
        Ok(out)
    }

    // ---------- Import ----------

    /// Items in the link's scope (up to 100), with suggested repo and an already-imported flag.
    /// Empty or missing `state_kinds` = open (triage, backlog, unstarted, started).
    pub async fn provider_list_importable(
        &self,
        link_id: String,
        query: Option<String>,
        state_kinds: Option<Vec<ExtKind>>,
    ) -> Result<Vec<ImportableItem>, AppError> {
        let link = self.load_link(&link_id).await?;
        let p = self.require(&link.provider).await?;
        let kinds = state_kinds.filter(|k| !k.is_empty()).unwrap_or_else(|| nodal_domain::model::providers::OPEN_KINDS.to_vec());
        let mut items = Vec::new();
        let mut cursor = None;
        for _ in 0..LIST_MAX_PAGES {
            let page = p
                .list_importable(&ImportQuery {
                    scope: link.scope.clone(),
                    text: query.clone(),
                    state_kinds: kinds.clone(),
                    created_after: None,
                    project_id: None,
                    closed_within_days: None,
                    cursor: cursor.take(),
                })
                .await?;
            items.extend(page.items);
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        self.db(move |c| import::importable_rows(c, &link, items)).await
    }

    pub async fn import_tasks(
        &self,
        project_id: String,
        link_id: String,
        items: Vec<ImportRequest>,
    ) -> Result<ImportResult, AppError> {
        let link = self.load_link(&link_id).await?;
        if link.project_id != project_id {
            return Err("That source belongs to another project.".into());
        }
        let p = self.require(&link.provider).await?;
        let ids: Vec<String> = items.iter().map(|i| i.external_id.clone()).collect();
        let mut full: HashMap<String, ExternalItem> = p.pull(&ids).await?.into_iter().map(|i| (i.external_id.clone(), i)).collect();
        let mut skipped = Vec::new();
        let mut pairs = Vec::new();
        for req in items {
            match full.remove(&req.external_id) {
                Some(item) => pairs.push((item, req.repo_id)),
                None => skipped.push(Skipped {
                    reason: format!("Not found in {} (deleted, archived, no access or listed twice).", link.provider),
                    external_id: req.external_id,
                }),
            }
        }
        let data_dir = self.data_dir.clone();
        let plans = self.plans.clone();
        let now = self.clock.now_ms();
        let mut result = self.db(move |c| import::import_items(c, &data_dir, &plans, &link, pairs, now)).await?;
        result.skipped.extend(skipped);
        self.notifier.notify(ChangeKind::Tasks, Some(&project_id));
        Ok(result)
    }

    // ---------- Project rules ----------

    /// Provider projects eligible as a link rule (in Linear: the active ones of its team).
    pub async fn source_rule_projects(&self, link_id: String) -> Result<Vec<ScopeRef>, AppError> {
        let link = self.load_link(&link_id).await?;
        Ok(self.require(&link.provider).await?.rule_projects(&link.scope).await?)
    }

    fn find_rule(link: &SourceLink, rule_id: &str) -> Result<RepoRule, String> {
        link.repo_rules
            .iter()
            .find(|r| r.id == rule_id && r.is_project())
            .cloned()
            .ok_or_else(|| "Project rule not found. Reload the source and try again.".to_string())
    }

    /// Items of the rule's backfill (all pages up to `BACKFILL_MAX_PAGES`), honoring the
    /// provider pause: if paused it does not hit the network, and a rate limit or a rejected
    /// key pauses it as in the sync.
    async fn backfill_items(&self, link: &SourceLink, rule: &RepoRule) -> Result<(Provider, Vec<ExternalItem>), AppError> {
        let now = self.clock.now_ms();
        if let Some(pause) = self.pause_of(&link.provider, now) {
            return Err(AppError::Message(format!(
                "{} sync is paused until {} ({}). Try again later or run a manual sync.",
                link.provider,
                iso_from_ms(pause.until),
                pause.reason
            )));
        }
        let p = self.require(&link.provider).await?;
        let base = rule_query(link, rule, true);
        let mut items = Vec::new();
        let mut cursor = None;
        for page_no in 0..BACKFILL_MAX_PAGES {
            let page = match p.list_importable(&ImportQuery { cursor: cursor.take(), ..base.clone() }).await {
                Ok(page) => page,
                Err(err) => {
                    self.pause_on(&link.provider, &err, now);
                    return Err(err.into());
                }
            };
            items.extend(page.items.into_iter().filter(|i| backfill_keeps(i, now)));
            match page.next_cursor {
                Some(c) if page_no + 1 < BACKFILL_MAX_PAGES => cursor = Some(c),
                Some(_) => {
                    eprintln!("import_rule: {} has more than {BACKFILL_MAX_PAGES} pages; the rest waits", rule.name);
                    break;
                }
                None => break,
            }
        }
        Ok((p, items))
    }

    async fn backfill_plan(&self, link_id: &str, rule_id: &str) -> Result<(SourceLink, RepoRule, Provider, BackfillPlan), AppError> {
        let link = self.load_link(link_id).await?;
        let rule = Self::find_rule(&link, rule_id)?;
        let (repo, project) = (rule.repo_id.clone(), link.project_id.clone());
        self.db(move |c| Ok(store::check_repo_in_project(c, &repo, &project)?)).await?;
        let (p, items) = self.backfill_items(&link, &rule).await?;
        let (l, r) = (link.clone(), rule.clone());
        let plan = self.db(move |c| import::plan_backfill(c, &l, &r, &items)).await?;
        Ok((link, rule, p, plan))
    }

    /// How many items the rule's backfill would bring (open + closed in the last 14 days), how
    /// many are already in its repo and how many in another one (those are not moved).
    pub async fn preview_rule_import(&self, link_id: String, rule_id: String) -> Result<RulePreview, AppError> {
        Ok(self.backfill_plan(&link_id, &rule_id).await?.3.preview())
    }

    /// Backfill of a project rule: imports into the rule's repo the project's items (within the
    /// link's scope) that are open or were closed in the last 14 days. Those already imported
    /// in another repo are not moved: they come back in `skipped`. Runs under the sync lock so
    /// it does not race the auto-import.
    pub async fn import_rule(&self, link_id: String, rule_id: String) -> Result<ImportResult, AppError> {
        let _sync = self.sync_lock.lock().await;
        let (link, rule, p, plan) = self.backfill_plan(&link_id, &rule_id).await?;
        let mut skipped = plan.elsewhere;
        let full = if plan.to_import.is_empty() {
            Vec::new()
        } else {
            match p.pull(&plan.to_import).await {
                Ok(v) => v,
                Err(err) => {
                    self.pause_on(&link.provider, &err, self.clock.now_ms());
                    return Err(err.into());
                }
            }
        };
        let found: std::collections::HashSet<&str> = full.iter().map(|i| i.external_id.as_str()).collect();
        skipped.extend(plan.to_import.iter().filter(|id| !found.contains(id.as_str())).map(|id| Skipped {
            external_id: id.clone(),
            reason: format!("Not found in {} (deleted, archived or no access).", link.provider),
        }));
        let pairs = full.into_iter().map(|i| (i, rule.repo_id.clone())).collect();
        let data_dir = self.data_dir.clone();
        let plans = self.plans.clone();
        let (here, rule_id, repo_id) = (plan.here, rule.id.clone(), rule.repo_id.clone());
        let now = self.clock.now_ms();
        let mut result = self
            .db(move |c| {
                // `update_source_link` does not take the sync lock: if the rule changed while
                // on the network, nothing is imported into the old repo.
                let l = store::get_link(c, &link_id)?;
                if !l.repo_rules.iter().any(|r| r.id == rule_id && r.repo_id == repo_id) {
                    return Err("The rule changed while importing. Try again.".to_string());
                }
                store::tag_rule(c, &l.id, &rule_id, &repo_id, &here)?;
                import::import_items(c, &data_dir, &plans, &l, pairs, now)
            })
            .await?;
        result.skipped.extend(skipped);
        self.notifier.notify(ChangeKind::Tasks, Some(&link.project_id));
        Ok(result)
    }

    /// Resolves the "moved to another project in the provider" notice: `move` moves it to the
    /// suggested repo (rejects with an active run or a worktree), `keep` leaves it where it is.
    pub async fn resolve_moved_task(&self, task_id: String, action: MovedAction) -> Result<Task, AppError> {
        let plans = self.plans.clone();
        let now = self.clock.now_ms();
        let out = self.db(move |c| moved::resolve_moved(c, &plans, &task_id, action, now)).await?;
        self.notifier.notify(ChangeKind::Tasks, Some(&out.project_id));
        Ok(out)
    }

    // ---------- Sync ----------

    /// One sync pass now (all sources, or only `link_id`).
    pub async fn sync_now(&self, link_id: Option<String>) -> Result<sync::SyncReport, AppError> {
        Ok(self.pass(link_id.as_deref(), true).await?)
    }

    /// A full pass, under `sync_lock`. The literal is today's message when the database isn't
    /// open (`sync_now`'s and the worker's).
    async fn pass(&self, link_filter: Option<&str>, force: bool) -> Result<sync::SyncReport, String> {
        let db = self.db.clone().ok_or("The database is not available.")?;
        let mut memo = self.sync_lock.lock().await;
        // Pauses are read from `self.pauses`: a new key may have lifted them.
        memo.paused = self.paused();
        let before = memo.paused.clone();
        let mut providers = Vec::new();
        for name in self.providers.names() {
            match self.resolve(name).await {
                Ok(Some(p)) => providers.push(p),
                Ok(None) => {}
                Err(e) if link_filter.is_some() => return Err(e),
                Err(e) => eprintln!("sync: {name}: {e}"),
            }
        }
        let report =
            sync::sync_run(&db, &self.data_dir, &self.plans, &providers, link_filter, self.clock.now_ms(), &mut memo, force).await;
        // Only what changed in this pass: a new key saved while it ran (which lifted an
        // earlier pause) does not get it back through the copy the pass took.
        self.merge_paused(&before, &memo.paused);
        // P15: `last_synced_at` moves on every pass (`set_link_sync`, above), but that alone
        // isn't worth a `Sources` event every tick (60 s) when nothing else did: only pulls,
        // pushes, imports, errors/notices, a pause change or a link's `last_sync_error`
        // changing (a cleared error has no `errors` entry, yet the UI must drop "Sync failed").
        let link_errors_changed = std::mem::take(&mut memo.link_errors_changed);
        let changed = report.pulled + report.pushed + report.imported > 0
            || !report.errors.is_empty()
            || !report.notices.is_empty()
            || memo.paused != before
            || link_errors_changed;
        if !providers.is_empty() && changed {
            // Tasks only if something moved.
            let kinds: &[ChangeKind] = if report.pulled + report.pushed + report.imported > 0 {
                &[ChangeKind::Sources, ChangeKind::Tasks]
            } else {
                &[ChangeKind::Sources]
            };
            self.notifier.notify_all(kinds, None);
        }
        Ok(report)
    }

    /// Every 60 s (15 s the first time), a full pass; errors are only logged, like today's
    /// worker.
    pub fn spawn_sync_worker(self: &Arc<Self>) {
        let hub = self.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(sync::FIRST_TICK).await;
            loop {
                match hub.pass(None, false).await {
                    Ok(r) => {
                        for e in &r.errors {
                            eprintln!("sync: {e}");
                        }
                    }
                    Err(e) => eprintln!("sync: {e}"),
                }
                tokio::time::sleep(sync::TICK).await;
            }
        });
    }
}

#[cfg(test)]
mod tests;
