//! Comandos de Tauri de proveedores. Firmas en `src/domain/api.ts` (sección Proveedores).
//! Todos rechazan con un string listo para mostrar.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::db::{with_db, Db, DbError};
use crate::domain::{ExtKind, RepoRule, RuleKind, ScopeRef, SourceLink, StateMap, Task};
use crate::linear::LinearState;

use super::import::{
    backfill_keeps, import_items, importable_rows, plan_backfill, rule_query, BackfillPlan, ImportRequest, ImportResult,
    ImportableItem, RulePreview, Skipped, BACKFILL_MAX_PAGES,
};
use super::state_map::{self, SourceStatesReport};
use super::sync::{run_for_app, SyncReport};
use crate::events::{notify, Kind};
use crate::secrets::Secrets;
use crate::util::{new_id, now_ms};
use super::{check_provider, require, resolve, store, ImportQuery, PResult, Provider, ProvidersState, TaskProvider};

/// Tope del listado de importables por llamada (páginas de 25).
const LIST_MAX_PAGES: usize = 4;

fn e(err: DbError) -> String {
    err.to_string()
}

async fn load_link(db: &Db, id: &str) -> PResult<SourceLink> {
    let id = id.to_string();
    with_db(db, move |c| store::get_link(c, &id)).await.map_err(e)
}

// ---------- Keys y estado ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub provider: String,
    pub has_key: bool,
    /// Nombre del usuario si la key es válida.
    pub viewer: Option<String>,
    pub error: Option<String>,
    /// Últimos 4 caracteres de la key guardada (nunca la key).
    pub key_hint: Option<String>,
    /// El sync automático está en pausa hasta este instante (epoch ms) por rate limit o key
    /// rechazada; `pause_reason` dice por qué.
    pub paused_until: Option<i64>,
    pub pause_reason: Option<String>,
}

fn pause_of(app: &AppHandle, provider: &str) -> Option<super::sync::Pause> {
    app.try_state::<ProvidersState>().and_then(|s| s.pause_of(provider, now_ms()))
}

fn clear_pause(app: &AppHandle, provider: &str) {
    if let Some(s) = app.try_state::<ProvidersState>() {
        s.clear_pause(provider);
    }
}

async fn status_of(app: &AppHandle, provider: &str) -> PResult<ProviderStatus> {
    let p = super::resolve_with_key(app, provider).await?;
    let mut out = ProviderStatus {
        provider: provider.into(),
        has_key: p.is_some(),
        viewer: None,
        error: None,
        key_hint: p.as_ref().and_then(|(_, k)| super::key_hint(k)),
        paused_until: None,
        pause_reason: None,
    };
    if let Some(pause) = pause_of(app, provider) {
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

#[tauri::command]
pub async fn provider_status(app: AppHandle, provider: String) -> PResult<ProviderStatus> {
    status_of(&app, &provider).await
}

/// Valida la key contra el proveedor y solo si es válida la guarda. `None` la borra.
#[tauri::command]
pub async fn provider_set_key(app: AppHandle, provider: String, key: Option<String>) -> PResult<ProviderStatus> {
    check_provider(&provider)?;
    let Some(key) = key.map(|k| k.trim().to_string()) else {
        return provider_clear_key(app, provider).await;
    };
    if key.is_empty() {
        return Err("The API key is empty.".into());
    }
    let linear = app.state::<LinearState>();
    let p = Provider::Linear(super::linear::LinearProvider::new(linear.http().clone(), key.clone()));
    let viewer = p.status().await?;
    app.state::<Secrets>().set(&provider, &key).await?;
    clear_pause(&app, &provider);
    let key_hint = super::key_hint(&key);
    Ok(ProviderStatus {
        provider,
        has_key: true,
        viewer: Some(viewer),
        error: None,
        key_hint,
        paused_until: None,
        pause_reason: None,
    })
}

#[tauri::command]
pub async fn provider_clear_key(app: AppHandle, provider: String) -> PResult<ProviderStatus> {
    check_provider(&provider)?;
    app.state::<Secrets>().delete(&provider).await?;
    clear_pause(&app, &provider);
    Ok(ProviderStatus {
        provider,
        has_key: false,
        viewer: None,
        error: None,
        key_hint: None,
        paused_until: None,
        pause_reason: None,
    })
}

#[tauri::command]
pub async fn provider_scopes(app: AppHandle, provider: String) -> PResult<Vec<ScopeRef>> {
    Ok(require(&app, &provider).await?.scopes().await?)
}

// ---------- Source links ----------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSourceLink {
    pub project_id: String,
    pub provider: String,
    pub scope: ScopeRef,
    #[serde(default)]
    pub default_repo_id: Option<String>,
    #[serde(default)]
    pub repo_rules: Vec<RepoRule>,
    #[serde(default)]
    pub auto_import: bool,
}

/// Distingue "falta" (`None`) de `null` (`Some(None)`).
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Ok(Some(Option::deserialize(d)?))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLinkPatch {
    #[serde(default, deserialize_with = "nullable")]
    pub default_repo_id: Option<Option<String>>,
    pub repo_rules: Option<Vec<RepoRule>>,
    pub auto_import: Option<bool>,
}

fn check_link_repos(conn: &rusqlite::Connection, link: &SourceLink) -> Result<(), DbError> {
    let repos = link.default_repo_id.iter().chain(link.repo_rules.iter().map(|r| &r.repo_id));
    for repo in repos {
        store::check_repo_in_project(conn, repo, &link.project_id)?;
    }
    Ok(())
}

/// Reglas que manda la UI → reglas guardadas. Una regla nueva (sin id, o con otro tipo o
/// valor que la guardada con ese id) recibe id y `created_at = now`: desde ahí cuenta su
/// auto-import. Las que no cambiaron conservan id y `created_at` (no se confía en el
/// cliente). Rechaza valores vacíos y dos reglas para el mismo proyecto.
pub fn prepare_rules(old: &[RepoRule], incoming: Vec<RepoRule>, now: i64) -> Result<Vec<RepoRule>, DbError> {
    let mut out: Vec<RepoRule> = Vec::with_capacity(incoming.len());
    for mut r in incoming {
        r.value = r.value.trim().to_string();
        r.name = r.name.trim().to_string();
        if r.value.is_empty() || r.kind == RuleKind::Unknown {
            return Err(DbError::Invalid(match r.kind {
                RuleKind::Label => "Routing rules need a label.".into(),
                RuleKind::Project => "Project rules need a project.".into(),
                RuleKind::Unknown => "Unknown routing rule type.".into(),
            }));
        }
        if r.name.is_empty() {
            r.name = r.value.clone();
        }
        let same = old.iter().find(|o| !r.id.is_empty() && o.id == r.id && o.kind == r.kind && o.value == r.value);
        match same {
            Some(o) => r.created_at = o.created_at,
            None => {
                r.id = new_id('r', now);
                r.created_at = now;
            }
        }
        if out.iter().any(|o| o.id == r.id) {
            r.id = new_id('r', now);
        }
        if r.is_project() && out.iter().any(|o| o.is_project() && o.value == r.value) {
            return Err(DbError::Invalid(format!("The project \"{}\" already has a rule.", r.name)));
        }
        out.push(r);
    }
    Ok(out)
}

#[tauri::command]
pub async fn list_source_links(db: State<'_, Db>, project_id: Option<String>) -> PResult<Vec<SourceLink>> {
    with_db(&db, move |c| store::list_links(c, project_id.as_deref())).await.map_err(e)
}

/// Crea el link con la propuesta de mapeo (pendiente hasta `save_state_map`). Si el
/// proveedor no responde, el link se crea igual con el mapeo vacío y la propuesta se arma
/// después en `source_states`.
#[tauri::command]
pub async fn create_source_link(app: AppHandle, db: State<'_, Db>, input: NewSourceLink) -> PResult<SourceLink> {
    check_provider(&input.provider)?;
    let states = match resolve(&app, &input.provider).await? {
        Some(p) => p.states(&input.scope).await.unwrap_or_else(|err| {
            eprintln!("create_source_link: states: {err}");
            Vec::new()
        }),
        None => Vec::new(),
    };
    let now = now_ms();
    let link = SourceLink {
        id: new_id('s', now),
        project_id: input.project_id,
        provider: input.provider,
        scope: input.scope,
        default_repo_id: input.default_repo_id,
        repo_rules: prepare_rules(&[], input.repo_rules, now).map_err(e)?,
        state_map: if states.is_empty() { StateMap::default() } else { state_map::propose(&states) },
        auto_import: input.auto_import,
        created_at: now,
        last_synced_at: None,
        last_sync_error: None,
        pending_state_changes: None,
    };
    with_db(&db, move |c| {
        if !store::project_exists(c, &link.project_id)? {
            return Err(DbError::Invalid("Project not found.".into()));
        }
        check_link_repos(c, &link)?;
        store::insert_link(c, &link)?;
        Ok(link)
    })
    .await
    .map_err(e)
    .inspect(|l| notify(&app, &[Kind::Sources], Some(&l.project_id)))
}

#[tauri::command]
pub async fn update_source_link(app: AppHandle, db: State<'_, Db>, id: String, patch: SourceLinkPatch) -> PResult<SourceLink> {
    with_db(&db, move |c| {
        let mut link = store::get_link(c, &id)?;
        if let Some(r) = patch.default_repo_id {
            link.default_repo_id = r;
        }
        if let Some(r) = patch.repo_rules {
            link.repo_rules = prepare_rules(&link.repo_rules, r, now_ms())?;
        }
        if let Some(a) = patch.auto_import {
            link.auto_import = a;
        }
        check_link_repos(c, &link)?;
        store::save_link(c, &link)?;
        Ok(link)
    })
    .await
    .map_err(e)
    .inspect(|l| notify(&app, &[Kind::Sources], Some(&l.project_id)))
}

/// Disconnect: las tareas del link quedan locales y el link se borra (una transacción).
#[tauri::command]
pub async fn delete_source_link(app: AppHandle, db: State<'_, Db>, id: String) -> PResult<()> {
    with_db(&db, move |c| store::disconnect_link(c, &id, now_ms()).map(|_| ())).await.map_err(e)?;
    notify(&app, &[Kind::Sources, Kind::Tasks], None);
    Ok(())
}

/// Unlink: la tarea pasa a ser local.
#[tauri::command]
pub async fn unlink_task(app: AppHandle, db: State<'_, Db>, task_id: String) -> PResult<Task> {
    with_db(&db, move |c| {
        let tx = c.transaction()?;
        let t = store::unlink_task(&tx, &task_id, now_ms())?;
        tx.commit()?;
        Ok(t)
    })
    .await
    .map_err(e)
    .inspect(|t| notify(&app, &[Kind::Tasks], Some(&t.project_id)))
}

// ---------- Mapeo de estados ----------

#[tauri::command]
pub async fn source_states(app: AppHandle, db: State<'_, Db>, link_id: String) -> PResult<SourceStatesReport> {
    let link = load_link(&db, &link_id).await?;
    let states = require(&app, &link.provider).await?.states(&link.scope).await?;
    Ok(state_map::report(&link.state_map, states))
}

/// Guarda y confirma el mapeo: fija `confirmed_at` y `known_states` con los estados actuales.
#[tauri::command]
pub async fn save_state_map(app: AppHandle, db: State<'_, Db>, link_id: String, map: StateMap) -> PResult<SourceLink> {
    let mut link = load_link(&db, &link_id).await?;
    let states = require(&app, &link.provider).await?.states(&link.scope).await?;
    state_map::validate(&map, &states)?;
    link.state_map = StateMap { confirmed_at: Some(now_ms()), known_states: states, ..map };
    with_db(&db, move |c| {
        // Solo el mapeo: un `update_source_link` hecho mientras se iba a la red no se pisa.
        store::save_state_map(c, &link.id, &link.state_map)?;
        store::get_link(c, &link.id)
    })
    .await
    .map_err(e)
    .inspect(|l| notify(&app, &[Kind::Sources], Some(&l.project_id)))
}

// ---------- Importación ----------

/// Ítems del scope del link (hasta 100), con repo sugerido y marca de ya importados.
/// `state_kinds` vacío o ausente = abiertos (triage, backlog, unstarted, started).
#[tauri::command]
pub async fn provider_list_importable(
    app: AppHandle,
    db: State<'_, Db>,
    link_id: String,
    query: Option<String>,
    state_kinds: Option<Vec<ExtKind>>,
) -> PResult<Vec<ImportableItem>> {
    let link = load_link(&db, &link_id).await?;
    let p = require(&app, &link.provider).await?;
    let kinds = state_kinds.filter(|k| !k.is_empty()).unwrap_or_else(|| super::OPEN_KINDS.to_vec());
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
    with_db(&db, move |c| importable_rows(c, &link, items)).await.map_err(e)
}

#[tauri::command]
pub async fn import_tasks(
    app: AppHandle,
    db: State<'_, Db>,
    project_id: String,
    link_id: String,
    items: Vec<ImportRequest>,
) -> PResult<ImportResult> {
    let link = load_link(&db, &link_id).await?;
    if link.project_id != project_id {
        return Err("That source belongs to another project.".into());
    }
    let p = require(&app, &link.provider).await?;
    let ids: Vec<String> = items.iter().map(|i| i.external_id.clone()).collect();
    let mut full: HashMap<String, _> = p.pull(&ids).await?.into_iter().map(|i| (i.external_id.clone(), i)).collect();
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
    let data_dir = app.state::<ProvidersState>().data_dir.clone();
    let mut result = with_db(&db, move |c| import_items(c, &data_dir, &link, pairs, now_ms())).await.map_err(e)?;
    result.skipped.extend(skipped);
    notify(&app, &[Kind::Tasks], Some(&project_id));
    Ok(result)
}

// ---------- Reglas de proyecto ----------

/// Proyectos del proveedor elegibles como regla del link (en Linear: los activos de su team).
#[tauri::command]
pub async fn source_rule_projects(app: AppHandle, db: State<'_, Db>, link_id: String) -> PResult<Vec<ScopeRef>> {
    let link = load_link(&db, &link_id).await?;
    Ok(require(&app, &link.provider).await?.rule_projects(&link.scope).await?)
}

fn find_rule(link: &SourceLink, rule_id: &str) -> PResult<RepoRule> {
    link.repo_rules
        .iter()
        .find(|r| r.id == rule_id && r.is_project())
        .cloned()
        .ok_or_else(|| "Project rule not found. Reload the source and try again.".to_string())
}

/// Ítems del backfill de la regla (todas las páginas hasta `BACKFILL_MAX_PAGES`), con la
/// pausa del proveedor: si está en pausa no va a la red, y un rate limit o una key rechazada
/// lo pausan como en el sync.
async fn backfill_items(app: &AppHandle, link: &SourceLink, rule: &RepoRule) -> PResult<(Provider, Vec<super::ExternalItem>)> {
    let state = app.state::<ProvidersState>();
    let now = now_ms();
    if let Some(pause) = state.pause_of(&link.provider, now) {
        return Err(format!(
            "{} sync is paused until {} ({}). Try again later or run a manual sync.",
            link.provider,
            super::iso_from_ms(pause.until),
            pause.reason
        ));
    }
    let p = require(app, &link.provider).await?;
    let base = rule_query(link, rule, true);
    let mut items = Vec::new();
    let mut cursor = None;
    for page_no in 0..BACKFILL_MAX_PAGES {
        let page = match p.list_importable(&super::ImportQuery { cursor: cursor.take(), ..base.clone() }).await {
            Ok(page) => page,
            Err(err) => {
                state.pause_on(&link.provider, &err, now);
                return Err(err.message);
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

async fn backfill_plan(app: &AppHandle, db: &Db, link_id: &str, rule_id: &str) -> PResult<(SourceLink, RepoRule, Provider, BackfillPlan)> {
    let link = load_link(db, link_id).await?;
    let rule = find_rule(&link, rule_id)?;
    let (repo, project) = (rule.repo_id.clone(), link.project_id.clone());
    with_db(db, move |c| store::check_repo_in_project(c, &repo, &project)).await.map_err(e)?;
    let (p, items) = backfill_items(app, &link, &rule).await?;
    let (l, r) = (link.clone(), rule.clone());
    let plan = with_db(db, move |c| plan_backfill(c, &l, &r, &items)).await.map_err(e)?;
    Ok((link, rule, p, plan))
}

/// Cuántos ítems traería el backfill de la regla (abiertos + cerrados en los últimos 14
/// días), cuántos ya están en su repo y cuántos en otro (esos no se mueven).
#[tauri::command]
pub async fn preview_rule_import(app: AppHandle, db: State<'_, Db>, link_id: String, rule_id: String) -> PResult<RulePreview> {
    Ok(backfill_plan(&app, &db, &link_id, &rule_id).await?.3.preview())
}

/// Backfill de una regla de proyecto: importa al repo de la regla los ítems del proyecto
/// (dentro del scope del link) abiertos o cerrados en los últimos 14 días. Los ya
/// importados en otro repo no se mueven: vuelven en `skipped`. Corre bajo el lock del sync
/// para no competir con el auto-import.
#[tauri::command]
pub async fn import_rule(app: AppHandle, db: State<'_, Db>, link_id: String, rule_id: String) -> PResult<ImportResult> {
    let state = app.state::<ProvidersState>();
    let _sync = state.sync_lock.lock().await;
    let (link, rule, p, plan) = backfill_plan(&app, &db, &link_id, &rule_id).await?;
    let mut skipped = plan.elsewhere;
    let full = if plan.to_import.is_empty() {
        Vec::new()
    } else {
        match p.pull(&plan.to_import).await {
            Ok(v) => v,
            Err(err) => {
                state.pause_on(&link.provider, &err, now_ms());
                return Err(err.message);
            }
        }
    };
    let found: std::collections::HashSet<&str> = full.iter().map(|i| i.external_id.as_str()).collect();
    skipped.extend(plan.to_import.iter().filter(|id| !found.contains(id.as_str())).map(|id| Skipped {
        external_id: id.clone(),
        reason: format!("Not found in {} (deleted, archived or no access).", link.provider),
    }));
    let pairs = full.into_iter().map(|i| (i, rule.repo_id.clone())).collect();
    let data_dir = state.data_dir.clone();
    let (here, rule_id, repo_id) = (plan.here, rule.id.clone(), rule.repo_id.clone());
    let mut result = with_db(&db, move |c| {
        // `update_source_link` no toma el lock del sync: si la regla cambió mientras se iba
        // a la red, no se importa al repo viejo.
        let l = store::get_link(c, &link_id)?;
        if !l.repo_rules.iter().any(|r| r.id == rule_id && r.repo_id == repo_id) {
            return Err(DbError::Invalid("The rule changed while importing. Try again.".into()));
        }
        store::tag_rule(c, &l.id, &rule_id, &repo_id, &here)?;
        import_items(c, &data_dir, &l, pairs, now_ms())
    })
    .await
    .map_err(e)?;
    result.skipped.extend(skipped);
    notify(&app, &[Kind::Tasks], Some(&link.project_id));
    Ok(result)
}

/// Resuelve el aviso "cambió de proyecto en el proveedor": `move` la pasa al repo
/// sugerido (rechaza con un run activo o un worktree), `keep` la deja donde está.
#[tauri::command]
pub async fn resolve_moved_task(app: AppHandle, db: State<'_, Db>, task_id: String, action: store::MovedAction) -> PResult<Task> {
    with_db(&db, move |c| store::resolve_moved(c, &task_id, action, now_ms()))
        .await
        .map_err(e)
        .inspect(|t| notify(&app, &[Kind::Tasks], Some(&t.project_id)))
}

// ---------- Sync ----------

/// Una pasada de sync ahora (todas las fuentes, o solo `link_id`).
#[tauri::command]
pub async fn sync_now(app: AppHandle, link_id: Option<String>) -> PResult<SyncReport> {
    run_for_app(&app, link_id.as_deref(), true).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_distinguishes_missing_from_null() {
        let p: SourceLinkPatch = serde_json::from_str(r#"{"autoImport": true}"#).unwrap();
        assert_eq!(p.default_repo_id, None);
        assert_eq!(p.auto_import, Some(true));
        let p: SourceLinkPatch = serde_json::from_str(r#"{"defaultRepoId": null}"#).unwrap();
        assert_eq!(p.default_repo_id, Some(None));
        let p: SourceLinkPatch = serde_json::from_str(r#"{"defaultRepoId": "r1"}"#).unwrap();
        assert_eq!(p.default_repo_id, Some(Some("r1".into())));
    }

    #[test]
    fn provider_status_shape() {
        let s = ProviderStatus {
            provider: "linear".into(),
            has_key: true,
            viewer: Some("Ana".into()),
            error: None,
            key_hint: super::super::key_hint("lin_api_0000000000abcd"),
            paused_until: Some(5),
            pause_reason: None,
        };
        let v = serde_json::to_value(s).unwrap();
        assert_eq!(v["hasKey"], true);
        assert_eq!(v["keyHint"], "abcd");
        assert_eq!(v["viewer"], "Ana");
        assert!(v["error"].is_null());
        assert_eq!(v["pausedUntil"], 5);
    }

    #[test]
    fn prepare_rules_assigns_ids_and_keeps_created_at() {
        let old = vec![RepoRule::project("proj-a", "A", "r1", 5), RepoRule::label("docs", "r2")];
        // Sin cambios: conserva id y created_at aunque el cliente mande otro.
        let mut same = old.clone();
        same[0].created_at = 999;
        let out = prepare_rules(&old, same, 50).unwrap();
        assert_eq!((out[0].id.as_str(), out[0].created_at), ("rule-proj-a", 5));
        // Regla nueva (sin id) y regla con el mismo id pero otro proyecto: id y created_at nuevos.
        let incoming: Vec<RepoRule> = serde_json::from_value(serde_json::json!([
            {"id": "rule-proj-a", "kind": "project", "value": "proj-b", "name": "B", "repoId": "r1"},
            {"kind": "project", "value": " proj-c ", "repoId": "r2"},
            {"label": "legacy", "repoId": "r2"}
        ]))
        .unwrap();
        let out = prepare_rules(&old, incoming, 50).unwrap();
        assert!(out.iter().all(|r| r.id.starts_with('r') && r.id != "rule-proj-a" && r.created_at == 50), "{out:?}");
        assert_eq!((out[1].value.as_str(), out[1].name.as_str()), ("proj-c", "proj-c"));
        assert_eq!((out[2].kind, out[2].value.as_str()), (RuleKind::Label, "legacy"));
        // Validaciones.
        let dup = vec![RepoRule::project("p", "P", "r1", 1), RepoRule::project("p", "P", "r2", 1)];
        assert!(prepare_rules(&[], dup, 1).unwrap_err().to_string().contains("already has a rule"));
        let empty = vec![RepoRule::label(" ", "r1")];
        assert!(prepare_rules(&[], empty, 1).unwrap_err().to_string().contains("need a label"));
    }

    #[test]
    fn key_hint_never_reveals_the_key() {
        use super::super::key_hint;
        assert_eq!(key_hint("  lin_api_0123456789wxyz \n").as_deref(), Some("wxyz"));
        assert_eq!(key_hint("short-key"), None, "keys cortas no dan pista");
        assert_eq!(key_hint(""), None);
    }
}
