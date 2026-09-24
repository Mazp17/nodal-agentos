//! Comandos de Tauri de proveedores. Firmas en `src/domain/api.ts` (sección Proveedores).
//! Todos rechazan con un string listo para mostrar.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::db::{with_db, Db, DbError};
use crate::domain::{ExtKind, RepoRule, ScopeRef, SourceLink, StateMap, Task};
use crate::linear::LinearState;

use super::import::{import_items, importable_rows, ImportRequest, ImportResult, ImportableItem, Skipped};
use super::state_map::{self, SourceStatesReport};
use super::sync::{run_for_app, SyncReport};
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
}

async fn status_of(app: &AppHandle, provider: &str) -> PResult<ProviderStatus> {
    let p = resolve(app, provider).await?;
    let mut out = ProviderStatus { provider: provider.into(), has_key: p.is_some(), viewer: None, error: None };
    if let Some(p) = p {
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
    Ok(ProviderStatus { provider, has_key: true, viewer: Some(viewer), error: None })
}

#[tauri::command]
pub async fn provider_clear_key(app: AppHandle, provider: String) -> PResult<ProviderStatus> {
    check_provider(&provider)?;
    app.state::<Secrets>().delete(&provider).await?;
    Ok(ProviderStatus { provider, has_key: false, viewer: None, error: None })
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
    if link.repo_rules.iter().any(|r| r.label.trim().is_empty()) {
        return Err(DbError::Invalid("Routing rules need a label.".into()));
    }
    Ok(())
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
        repo_rules: input.repo_rules,
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
}

#[tauri::command]
pub async fn update_source_link(db: State<'_, Db>, id: String, patch: SourceLinkPatch) -> PResult<SourceLink> {
    with_db(&db, move |c| {
        let mut link = store::get_link(c, &id)?;
        if let Some(r) = patch.default_repo_id {
            link.default_repo_id = r;
        }
        if let Some(r) = patch.repo_rules {
            link.repo_rules = r;
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
}

/// Disconnect: las tareas del link quedan locales y el link se borra (una transacción).
#[tauri::command]
pub async fn delete_source_link(db: State<'_, Db>, id: String) -> PResult<()> {
    with_db(&db, move |c| store::disconnect_link(c, &id, now_ms()).map(|_| ())).await.map_err(e)
}

/// Unlink: la tarea pasa a ser local.
#[tauri::command]
pub async fn unlink_task(db: State<'_, Db>, task_id: String) -> PResult<Task> {
    with_db(&db, move |c| {
        let tx = c.transaction()?;
        let t = store::unlink_task(&tx, &task_id, now_ms())?;
        tx.commit()?;
        Ok(t)
    })
    .await
    .map_err(e)
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
    Ok(result)
}

// ---------- Sync ----------

/// Una pasada de sync ahora (todas las fuentes, o solo `link_id`).
#[tauri::command]
pub async fn sync_now(app: AppHandle, link_id: Option<String>) -> PResult<SyncReport> {
    run_for_app(&app, link_id.as_deref()).await
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
        let s = ProviderStatus { provider: "linear".into(), has_key: true, viewer: Some("Ana".into()), error: None };
        let v = serde_json::to_value(s).unwrap();
        assert_eq!(v["hasKey"], true);
        assert_eq!(v["viewer"], "Ana");
        assert!(v["error"].is_null());
    }
}
