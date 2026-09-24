//! Sync con los proveedores: worker cada 60 s y `sync_now`.
//!
//! Orden por proveedor: estados actuales de cada link → push (drena el outbox) → pull →
//! auto-import. El push va primero para que el pull ya vea lo que Nodal empujó.
//!
//! - Pull: título y plan (si no está overridden) siempre; el estado Nodal solo si cambió el
//!   `external_state.id` y la tarea no tiene un run activo (gana Nodal). Un estado sin mapear
//!   no cambia el estado Nodal y queda en `sync_error`.
//! - Push: backoff exponencial por fila; los `set_state` con mapeo pendiente, "No
//!   sincronizar", sin mapeo o hacia un estado desaparecido se descartan (los comentarios
//!   se mandan igual). Errores permanentes o `MAX_ATTEMPTS` intentos descartan la fila; rate
//!   limit o key inválida no gastan intentos y cortan el drenado del proveedor en esa pasada.
//!   Una fila fallida frena las siguientes de su tarea (orden estado → comentario).
//! - Pull solo de tareas abiertas o cerradas hace menos de 7 días; un ítem desvinculado
//!   (unlink) no vuelve por auto-import.
//! - Auto-import: ítems abiertos del scope creados después del link, con repo por regla o
//!   default; sin repo resoluble no se importan y se reporta. El pull completo solo se pide
//!   para candidatos nuevos con repo.
//! - Rate limit o key rechazada en cualquier paso cortan la pasada del proveedor y lo pausan
//!   en memoria (`SyncMemo`); los estados de cada scope se releen cada `STATES_TTL_MS`. Un
//!   sync manual (`force`) ignora la pausa y relee los estados.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::{rows, with_db, Db, DbError};
use crate::util::now_ms;
use crate::domain::{ExternalState, OutboxPayload, PlanRef, SourceLink, StateChanges, StateMap, Task, TaskStatus};

use super::import::{import_items, suggest_repo};
use super::plan::{plan_path, render_plan, write_plan};
use super::state_map::{diff_known, literal_target, pull_status, push_target, PushTarget, SkipReason};
use super::{
    resolve, store, ErrorKind, ExternalItem, ImportQuery, PResult, Provider, ProviderError, ProviderResult,
    ProvidersState, TaskProvider,
};

pub const TICK: Duration = Duration::from_secs(60);
/// Primera pasada un rato después de arrancar, para no competir con el resto del arranque.
const FIRST_TICK: Duration = Duration::from_secs(15);
pub const BACKOFF_BASE_MS: i64 = 30_000;
pub const BACKOFF_MAX_MS: i64 = 60 * 60_000;
/// Intentos de una fila del outbox con errores transitorios antes de descartarla.
pub const MAX_ATTEMPTS: i64 = 10;
const AUTO_IMPORT_MAX_PAGES: usize = 4;
/// Los estados de cada scope se releen cada tanto (o en un sync manual), no en cada pasada.
pub const STATES_TTL_MS: i64 = 10 * 60_000;
/// Pausa del proveedor entero tras un rate limit o una key rechazada (el worker no lo toca
/// hasta entonces; un sync manual o una key nueva la levantan).
pub const RATE_LIMIT_PAUSE_MS: i64 = 5 * 60_000;
pub const AUTH_PAUSE_MS: i64 = 30 * 60_000;

/// Espera antes del intento `attempts + 1` (30 s, 1 min, 2 min… hasta 1 h).
pub fn backoff_ms(attempts: i64) -> i64 {
    let exp = (attempts - 1).clamp(0, 20) as u32;
    BACKOFF_BASE_MS.saturating_mul(1 << exp).min(BACKOFF_MAX_MS)
}

/// Proveedor en pausa (ver `RATE_LIMIT_PAUSE_MS`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pause {
    pub until: i64,
    pub reason: String,
}

/// Memoria del sync entre pasadas (en memoria; vive bajo `ProvidersState::sync_lock`).
#[derive(Debug, Default)]
pub struct SyncMemo {
    /// Proveedor → pausa.
    pub paused: HashMap<String, Pause>,
    /// Link → (leídos en, mapeo con el que se leyeron, estados del scope). Un mapeo distinto
    /// (el usuario lo guardó) invalida la entrada: un destino recién mapeado no se toma como
    /// desaparecido.
    states: HashMap<String, (i64, StateMap, Vec<ExternalState>)>,
}

/// Errores que no son de un ítem sino del proveedor entero: cortan la pasada y lo pausan.
fn halts(e: &ProviderError) -> bool {
    matches!(e.kind, ErrorKind::RateLimited | ErrorKind::Auth)
}

/// Espejo de `SyncReport` en `api.ts`.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    /// Tareas refrescadas desde el proveedor.
    pub pulled: usize,
    /// Escrituras hechas en el proveedor (estados y comentarios).
    pub pushed: usize,
    /// Tareas nuevas por auto-import.
    pub imported: usize,
    pub errors: Vec<String>,
    /// Avisos que no son errores: estados nuevos o desaparecidos, ítems sin repo para el
    /// auto-import, pushes descartados porque el destino ya no existe.
    pub notices: Vec<String>,
}

// ---------- Decisiones puras ----------

#[derive(Debug, Clone, PartialEq)]
pub struct PullDecision {
    pub title: Option<String>,
    pub status: Option<TaskStatus>,
    /// Guardar el estado externo como visto. No se guarda uno sin mapear: cuando el usuario
    /// lo mapee, el pull siguiente lo verá como cambio y lo aplicará. Tampoco mientras hay un
    /// push de estado pendiente: si ese push se descarta, el pull siguiente aplica el externo.
    pub record_state: bool,
    /// El estado externo no está en el mapeo pull.
    pub unmapped: bool,
    pub sync_error: Option<String>,
}

/// `pending_push`: la tarea tiene un `set_state` en el outbox (cambio local sin empujar), que
/// gana sobre el estado externo igual que un run activo.
pub fn decide_pull(task: &Task, item: &ExternalItem, map: &StateMap, active_run: bool, pending_push: bool) -> PullDecision {
    let prev = task.source.as_ref().and_then(|s| s.external_state.as_ref()).map(|s| s.id.as_str());
    let changed = prev != Some(item.state.id.as_str());
    let mapped = pull_status(map, &item.state);
    let apply = changed && !active_run && !pending_push;
    PullDecision {
        title: (item.title != task.title).then(|| item.title.clone()),
        status: if apply { mapped.filter(|s| *s != task.status) } else { None },
        record_state: mapped.is_some() && !pending_push,
        unmapped: mapped.is_none(),
        sync_error: mapped.is_none().then(|| {
            format!("External state \"{}\" is not mapped to a Nodal status. Review the mapping.", item.state.name)
        }),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PushAction {
    Comment(String),
    SetState(String),
    /// Se descarta la fila; con mensaje si hay que avisar.
    Drop(Option<String>),
}

/// Qué hacer con una fila del outbox. `map`: el del link de la tarea (`None` = tarea sin
/// link, equivale a mapeo pendiente). `current`: estados actuales del scope si se conocen.
pub fn decide_push(
    payload: &OutboxPayload,
    map: Option<&StateMap>,
    current: Option<&[ExternalState]>,
    external_state: Option<&ExternalState>,
) -> PushAction {
    let state_id = match payload {
        OutboxPayload::Comment { body } => return PushAction::Comment(body.clone()),
        OutboxPayload::SetState { state_id } => state_id,
    };
    let Some(map) = map else { return PushAction::Drop(None) };
    let target = match TaskStatus::parse(state_id) {
        Some(status) => push_target(map, status, current),
        None => literal_target(map, state_id, current),
    };
    match target {
        PushTarget::Push(id) if external_state.is_some_and(|s| s.id == id) => PushAction::Drop(None),
        PushTarget::Push(id) => PushAction::SetState(id),
        PushTarget::Skip(SkipReason::TargetGone(id)) => {
            let name = map.known_states.iter().find(|s| s.id == id).map_or(id.as_str(), |s| s.name.as_str());
            PushAction::Drop(Some(format!(
                "The mapped state \"{name}\" no longer exists in the provider; the status was not pushed. Review the mapping."
            )))
        }
        PushTarget::Skip(_) => PushAction::Drop(None),
    }
}

/// Marca invisible (comentario HTML) que identifica la fila del outbox en el cuerpo del
/// comentario. Los ids del outbox son `AUTOINCREMENT`: nunca se reusan.
pub fn outbox_marker(id: i64) -> String {
    format!("<!-- nodal:outbox:{id} -->")
}

/// Cuerpo enviado: el texto más la marca.
pub fn marked_body(body: &str, id: i64) -> String {
    format!("{}\n\n{}", body.trim_end(), outbox_marker(id))
}

/// Manda el comentario de la fila `id` salvo que ya esté en el proveedor: si un intento
/// anterior llegó a Linear pero no a `outbox_done` (timeout, cierre de la app), no se repite.
async fn send_comment(p: &Provider, external_id: &str, body: &str, id: i64) -> ProviderResult<()> {
    if p.has_comment_with(external_id, &outbox_marker(id)).await? {
        return Ok(());
    }
    p.comment(external_id, &marked_body(body, id)).await
}

// ---------- Pasada de sync ----------

fn err(e: DbError) -> String {
    e.to_string()
}

/// Una pasada completa (o solo de `link_filter`). `providers`: los que tienen key.
/// `force` (sync manual): ignora la pausa de los proveedores y relee los estados.
pub async fn sync_run(
    db: &Db,
    data_dir: &Path,
    providers: &[Provider],
    link_filter: Option<&str>,
    now: i64,
    memo: &mut SyncMemo,
    force: bool,
) -> SyncReport {
    let mut report = SyncReport::default();
    let links = match with_db(db, |c| store::list_links(c, None)).await {
        Ok(l) => l,
        Err(e) => {
            report.errors.push(err(e));
            return report;
        }
    };
    if let Some(id) = link_filter {
        match links.iter().find(|l| l.id == id) {
            None => {
                report.errors.push("Source not found.".into());
                return report;
            }
            Some(l) if !providers.iter().any(|p| p.name() == l.provider) => {
                report.errors.push(format!("The {} API key is missing. Add it in Settings → Providers.", l.provider));
                return report;
            }
            _ => {}
        }
    }

    for p in providers {
        let name = p.name();
        match memo.paused.get(name) {
            Some(pause) if !force && pause.until > now => {
                report.notices.push(format!(
                    "{name}: sync paused until {} ({})",
                    super::iso_from_ms(pause.until),
                    pause.reason
                ));
                continue;
            }
            Some(_) => {
                memo.paused.remove(name);
            }
            None => {}
        }
        let targets: Vec<&SourceLink> = links
            .iter()
            .filter(|l| l.provider == name && link_filter.is_none_or(|id| id == l.id))
            .collect();

        let mut current: HashMap<String, Vec<ExternalState>> = HashMap::new();
        // Errores atribuibles a cada link en esta pasada (estados, pull, auto-import).
        let mut link_errors: HashMap<String, Vec<String>> = HashMap::new();
        // Rate limit o key rechazada: el resto de la pasada fallaría igual.
        let mut halt: Option<ProviderError> = None;
        for link in &targets {
            if halt.is_some() {
                break;
            }
            let cached = memo
                .states
                .get(&link.id)
                .filter(|(at, map, _)| !force && now - at < STATES_TTL_MS && *map == link.state_map);
            if let Some((_, _, states)) = cached {
                current.insert(link.id.clone(), states.clone());
                continue;
            }
            match p.states(&link.scope).await {
                Ok(states) => {
                    memo.states.insert(link.id.clone(), (now, link.state_map.clone(), states.clone()));
                    if link.state_map.confirmed_at.is_some() {
                        let (added, removed) = diff_known(&link.state_map.known_states, &states);
                        let names = |v: &[ExternalState]| v.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ");
                        if !added.is_empty() {
                            report.notices.push(format!("{}: new states to map: {}.", link.scope.name, names(&added)));
                        }
                        if !removed.is_empty() {
                            report.notices.push(format!("{}: states removed: {}.", link.scope.name, names(&removed)));
                        }
                        let (id, changes) = (link.id.clone(), StateChanges { added, removed });
                        if let Err(e) = with_db(db, move |c| store::set_pending_changes(c, &id, &changes)).await {
                            report.errors.push(err(e));
                        }
                    }
                    current.insert(link.id.clone(), states);
                }
                Err(e) => {
                    let msg = format!("{}: {e}", link.scope.name);
                    link_errors.entry(link.id.clone()).or_default().push(msg.clone());
                    report.errors.push(msg);
                    if halts(&e) {
                        halt = Some(e);
                    }
                }
            }
        }

        if halt.is_none() {
            let (flagged, h) = drain_outbox(db, p, &links, &current, link_filter, now, &mut report).await;
            halt = h;
            for link in &targets {
                if halt.is_some() {
                    break;
                }
                let before = report.errors.len();
                halt = pull_link(db, data_dir, p, link, &flagged, now, &mut report).await;
                if link.auto_import && halt.is_none() {
                    halt = auto_import_link(db, data_dir, p, link, now, &mut report).await;
                }
                link_errors.entry(link.id.clone()).or_default().extend(report.errors[before..].iter().cloned());
            }
        }
        if let Some(e) = &halt {
            let until = now + if e.kind == ErrorKind::Auth { AUTH_PAUSE_MS } else { RATE_LIMIT_PAUSE_MS };
            report.notices.push(format!("{name}: sync paused until {} ({e})", super::iso_from_ms(until)));
            memo.paused.insert(name.to_string(), Pause { until, reason: e.message.clone() });
        }
        for link in &targets {
            let mut errors = link_errors.remove(&link.id).unwrap_or_default();
            if let Some(e) = &halt {
                // Los links que no llegaron a sincronizarse también muestran por qué.
                if errors.is_empty() {
                    errors.push(format!("{}: {e}", link.scope.name));
                }
            }
            let msg = (!errors.is_empty()).then(|| errors.join("\n"));
            let id = link.id.clone();
            if let Err(e) = with_db(db, move |c| store::set_link_sync(c, &id, now, msg.as_deref())).await {
                report.errors.push(err(e));
            }
        }
    }
    report
}

async fn drain_outbox(
    db: &Db,
    p: &Provider,
    links: &[SourceLink],
    current: &HashMap<String, Vec<ExternalState>>,
    link_filter: Option<&str>,
    now: i64,
    report: &mut SyncReport,
) -> (HashSet<String>, Option<ProviderError>) {
    // Tareas a las que el push les dejó un error o aviso: el pull de esta pasada no lo borra.
    let mut flagged = HashSet::new();
    let mut halt = None;
    let name = p.name();
    let items = match with_db(db, move |c| store::due_outbox(c, name, now)).await {
        Ok(i) => i,
        Err(e) => {
            report.errors.push(err(e));
            return (flagged, None);
        }
    };
    // Orden por tarea: si una fila falla, las siguientes de esa tarea esperan (el comentario
    // de cierre no sale antes que el cambio de estado que lo acompaña).
    let mut failed: HashSet<String> = HashSet::new();
    for item in items {
        if failed.contains(&item.task_id) {
            continue;
        }
        let task_id = item.task_id.clone();
        let task = match with_db(db, move |c| rows::get_task(c, &task_id)).await {
            Ok(t) => t,
            Err(e) => {
                report.errors.push(err(e));
                continue;
            }
        };
        let Some((task, src)) = task.and_then(|t| t.source.clone().map(|s| (t, s))) else {
            // Tarea desvinculada entre medio: la fila ya no tiene destino.
            let id = item.id;
            if let Err(e) = with_db(db, move |c| store::outbox_done(c, id)).await {
                report.errors.push(err(e));
            }
            continue;
        };
        if link_filter.is_some() && src.link_id.as_deref() != link_filter {
            continue;
        }
        let link = src.link_id.as_deref().and_then(|id| links.iter().find(|l| l.id == id));
        let states = link.and_then(|l| current.get(&l.id)).map(Vec::as_slice);
        let action = decide_push(&item.payload, link.map(|l| &l.state_map), states, src.external_state.as_ref());

        let sent: ProviderResult<Option<ExternalState>> = match &action {
            PushAction::Comment(body) => send_comment(p, &src.external_id, body, item.id).await.map(|()| None),
            PushAction::SetState(state_id) => p.set_state(&src.external_id, state_id).await.map(Some),
            PushAction::Drop(_) => Ok(None),
        };
        let (id, task_id) = (item.id, task.id.clone());
        let mut stop = false;
        let result = match (sent, action) {
            (Ok(_), PushAction::Drop(notice)) => {
                if let Some(n) = &notice {
                    report.notices.push(format!("{}: {n}", src.identifier));
                    flagged.insert(task.id.clone());
                }
                with_db(db, move |c| {
                    store::outbox_done(c, id)?;
                    match notice {
                        Some(n) => store::set_sync_error(c, &task_id, Some(&n)),
                        None => Ok(()),
                    }
                })
                .await
            }
            (Ok(state), _) => {
                report.pushed += 1;
                with_db(db, move |c| {
                    store::outbox_done(c, id)?;
                    match state {
                        Some(s) => store::set_external_state(c, &task_id, &s, now),
                        None => store::set_sync_error(c, &task_id, None),
                    }
                })
                .await
            }
            (Err(e), _) => {
                flagged.insert(task.id.clone());
                failed.insert(task.id.clone());
                // Sin key o con rate limit, el resto de las filas fallaría igual. Esos errores
                // no son de la fila: no gastan intentos (una key revocada no descarta cambios),
                // solo corren el próximo intento (la espera larga es la pausa del proveedor).
                stop = halts(&e);
                if stop {
                    halt = Some(e.clone());
                }
                let attempts = if stop { item.attempts } else { item.attempts + 1 };
                let give_up = e.kind == ErrorKind::Permanent || (!stop && attempts >= MAX_ATTEMPTS);
                let msg = if give_up {
                    format!("{e} (not retried after {attempts} attempt(s); the change was not pushed)")
                } else {
                    e.message.clone()
                };
                report.errors.push(format!("{}: {msg}", src.identifier));
                let next = now + if stop { BACKOFF_BASE_MS } else { backoff_ms(attempts) };
                with_db(db, move |c| {
                    if give_up {
                        store::outbox_done(c, id)?;
                    } else {
                        store::outbox_retry(c, id, attempts, next, &msg)?;
                    }
                    store::set_sync_error(c, &task_id, Some(&msg))
                })
                .await
            }
        };
        if let Err(e) = result {
            report.errors.push(err(e));
        }
        if stop {
            break;
        }
    }
    (flagged, halt)
}

/// Lo común a todas las tareas de un link en el pull.
struct PullCtx<'a> {
    data_dir: &'a Path,
    provider: &'a str,
    map: &'a StateMap,
    now: i64,
}

/// Aplica el pull a una tarea. Devuelve si el ítem existía.
fn apply_pull_task(
    conn: &Connection,
    ctx: &PullCtx,
    task: &Task,
    item: Option<&ExternalItem>,
    keep_error: bool,
) -> Result<bool, DbError> {
    let PullCtx { data_dir, provider, map, now } = *ctx;
    // La tarea se leyó antes de ir a la red: si entre medio la desvincularon, la movieron de
    // link o le sobrescribieron el plan, manda lo que hay ahora.
    let before = task.source.as_ref().map(|s| (s.link_id.clone(), s.external_id.clone()));
    let Some(task) = rows::get_task(conn, &task.id)?
        .filter(|t| t.source.as_ref().map(|s| (s.link_id.clone(), s.external_id.clone())) == before)
    else {
        return Ok(false);
    };
    let task = &task;
    let Some(item) = item else {
        let msg = format!("Not found in {provider} (deleted or no access).");
        store::set_sync_error(conn, &task.id, Some(&msg))?;
        return Ok(false);
    };
    let active = store::has_active_run(conn, &task.id)?;
    let pending = store::has_pending_state_push(conn, &task.id)?;
    let d = decide_pull(task, item, map, active, pending);
    store::apply_pull(
        conn,
        &store::PullUpdate {
            task_id: &task.id,
            title: d.title.as_deref(),
            status: d.status,
            external_state: d.record_state.then_some(&item.state),
            sync_error: d.sync_error.as_deref(),
            keep_error,
            unmapped: d.unmapped,
            now,
        },
    )?;
    if !task.plan_overridden && task.plan == PlanRef::Text {
        write_plan(&plan_path(data_dir, &task.id), &render_plan(item))
            .map_err(|e| DbError::Invalid(format!("Could not write the plan of {}: {e}", item.identifier)))?;
    }
    Ok(true)
}

async fn pull_link(
    db: &Db,
    data_dir: &Path,
    p: &Provider,
    link: &SourceLink,
    flagged: &HashSet<String>,
    now: i64,
    report: &mut SyncReport,
) -> Option<ProviderError> {
    let link_id = link.id.clone();
    let tasks = match with_db(db, move |c| store::linked_tasks(c, Some(&link_id), now)).await {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(err(e));
            return None;
        }
    };
    if tasks.is_empty() {
        return None;
    }
    let ids: Vec<String> = tasks.iter().filter_map(|t| t.source.as_ref().map(|s| s.external_id.clone())).collect();
    let items: HashMap<String, ExternalItem> = match p.pull(&ids).await {
        Ok(v) => v.into_iter().map(|i| (i.external_id.clone(), i)).collect(),
        Err(e) => {
            report.errors.push(format!("{}: {e}", link.scope.name));
            return halts(&e).then_some(e);
        }
    };
    let (data_dir, provider, map, flagged) = (data_dir.to_path_buf(), p.name(), link.state_map.clone(), flagged.clone());
    let res = with_db(db, move |c| {
        let mut pulled = 0;
        let mut errors = Vec::new();
        let ctx = PullCtx { data_dir: &data_dir, provider, map: &map, now };
        for t in &tasks {
            let item = t.source.as_ref().and_then(|s| items.get(&s.external_id));
            match apply_pull_task(c, &ctx, t, item, flagged.contains(&t.id)) {
                Ok(true) => pulled += 1,
                Ok(false) => {}
                Err(e) => errors.push(e.to_string()),
            }
        }
        Ok((pulled, errors))
    })
    .await;
    match res {
        Ok((n, errors)) => {
            report.pulled += n;
            report.errors.extend(errors);
        }
        Err(e) => report.errors.push(err(e)),
    }
    None
}

async fn auto_import_link(
    db: &Db,
    data_dir: &Path,
    p: &Provider,
    link: &SourceLink,
    now: i64,
    report: &mut SyncReport,
) -> Option<ProviderError> {
    let mut candidates = Vec::new();
    let mut cursor = None;
    for _ in 0..AUTO_IMPORT_MAX_PAGES {
        let q = ImportQuery {
            scope: link.scope.clone(),
            text: None,
            state_kinds: super::OPEN_KINDS.to_vec(),
            created_after: Some(link.created_at),
            cursor: cursor.take(),
        };
        match p.list_importable(&q).await {
            Ok(page) => {
                candidates.extend(page.items);
                match page.next_cursor {
                    Some(c) => cursor = Some(c),
                    None => break,
                }
            }
            Err(e) => {
                report.errors.push(format!("{}: {e}", link.scope.name));
                return halts(&e).then_some(e);
            }
        }
    }
    let l = link.clone();
    let fresh = with_db(db, move |c| {
        let mut out = Vec::new();
        for i in candidates {
            if store::task_by_external(c, &l.provider, &i.external_id)?.is_none()
                && !store::is_unlinked(c, &l.provider, &i.external_id)?
            {
                // Un repo de regla o por defecto que ya no es del proyecto (borrado) no se
                // reintenta: el ítem espera a que se corrija la fuente.
                let repo = suggest_repo(&l, &i.labels);
                let usable = match &repo {
                    Some(r) => store::repo_project(c, r)?.as_deref() == Some(l.project_id.as_str()),
                    None => false,
                };
                out.push((repo, usable, i));
            }
        }
        Ok(out)
    })
    .await;
    let fresh = match fresh {
        Ok(f) => f,
        Err(e) => {
            report.errors.push(err(e));
            return None;
        }
    };
    let mut routed: Vec<(String, String)> = Vec::new();
    let mut stale: Vec<String> = Vec::new();
    for (repo, usable, item) in fresh {
        match repo {
            Some(r) if usable => routed.push((item.external_id, r)),
            Some(_) => stale.push(item.identifier),
            None => report.notices.push(format!(
                "{}: not auto-imported (no repo rule matches its labels and the source has no default repo).",
                item.identifier
            )),
        }
    }
    if !stale.is_empty() {
        // Error del link (queda en `last_sync_error` mientras dure): no se reintenta cada minuto.
        report.errors.push(format!(
            "{}: auto-import is waiting for {} ({}): the repo of its rule or the default repo no longer exists or belongs to another project. Review the source's repos.",
            link.scope.name,
            if stale.len() == 1 { "1 item".to_string() } else { format!("{} items", stale.len()) },
            stale.join(", ")
        ));
    }
    if routed.is_empty() {
        return None;
    }
    let ids: Vec<String> = routed.iter().map(|(id, _)| id.clone()).collect();
    let full: HashMap<String, ExternalItem> = match p.pull(&ids).await {
        Ok(v) => v.into_iter().map(|i| (i.external_id.clone(), i)).collect(),
        Err(e) => {
            report.errors.push(format!("{}: {e}", link.scope.name));
            return halts(&e).then_some(e);
        }
    };
    let pairs: Vec<(ExternalItem, String)> =
        routed.into_iter().filter_map(|(id, repo)| full.get(&id).cloned().map(|i| (i, repo))).collect();
    let (l, dir) = (link.clone(), data_dir.to_path_buf());
    match with_db(db, move |c| import_items(c, &dir, &l, pairs, now)).await {
        Ok(r) => {
            report.imported += r.imported.len();
            report.errors.extend(r.skipped.into_iter().map(|s| format!("Auto-import {}: {}", s.external_id, s.reason)));
        }
        Err(e) => report.errors.push(err(e)),
    }
    None
}

// ---------- Worker y comando ----------

/// Proveedores con key, una pasada, bajo el lock de sync. `force`: sync manual (ver
/// `sync_run`).
pub async fn run_for_app(app: &AppHandle, link_filter: Option<&str>, force: bool) -> PResult<SyncReport> {
    let db: Db = app.try_state::<Db>().map(|s| s.inner().clone()).ok_or("The database is not available.")?;
    let state = app.state::<ProvidersState>();
    let mut memo = state.sync_lock.lock().await;
    // Las pausas se leen de `ProvidersState::pauses`: una key nueva pudo levantarlas.
    memo.paused = state.paused();
    let before = memo.paused.clone();
    let mut providers = Vec::new();
    for name in super::KNOWN_PROVIDERS {
        match resolve(app, name).await {
            Ok(Some(p)) => providers.push(p),
            Ok(None) => {}
            Err(e) if link_filter.is_some() => return Err(e),
            Err(e) => eprintln!("sync: {name}: {e}"),
        }
    }
    let data_dir: PathBuf = state.data_dir.clone();
    let report = sync_run(&db, &data_dir, &providers, link_filter, now_ms(), &mut memo, force).await;
    // Solo lo que cambió esta pasada: una key nueva guardada mientras corría (que levantó una
    // pausa previa) no la recupera por la copia que tomó la pasada.
    state.merge_paused(&before, &memo.paused);
    if !providers.is_empty() {
        use crate::events::Kind;
        // El estado de los links cambia en cada pasada; las tareas solo si hubo movimiento.
        let kinds: &[Kind] =
            if report.pulled + report.pushed + report.imported > 0 { &[Kind::Sources, Kind::Tasks] } else { &[Kind::Sources] };
        crate::events::notify(app, kinds, None);
    }
    Ok(report)
}

pub fn spawn_worker(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_TICK).await;
        loop {
            match run_for_app(&app, None, false).await {
                Ok(r) => {
                    for e in &r.errors {
                        eprintln!("sync: {e}");
                    }
                }
                Err(e) => eprintln!("sync: {e}"),
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::domain::*;
    use crate::providers::fake::FakeProvider;
    use crate::providers::{ErrorKind, ProviderError};
    use crate::providers::import::tests::{seed, tmp_dir};
    use crate::providers::plan::tests::item;
    use crate::providers::state_map::tests::team_states;

    fn state(id: &str) -> ExternalState {
        team_states().into_iter().find(|s| s.id == id).unwrap()
    }

    struct Env {
        db: Db,
        dir: PathBuf,
        fake: FakeProvider,
        providers: Vec<Provider>,
        link: SourceLink,
        memo: std::cell::RefCell<SyncMemo>,
    }

    impl Drop for Env {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    impl Env {
        fn new() -> Self {
            let db = open_in_memory().unwrap();
            let link = seed(&db.lock().unwrap());
            let fake = FakeProvider::default();
            fake.data().states = team_states();
            Env {
                db,
                dir: tmp_dir(),
                providers: vec![Provider::Fake(fake.clone())],
                fake,
                link,
                memo: Default::default(),
            }
        }

        /// Importa el ítem `n` (en estado `state_id`) al repo web y lo deja en el fake.
        fn import(&self, n: u32, state_id: &str) -> Task {
            let mut it = item(n, Some("Descripción original."));
            it.state = state(state_id);
            self.fake.data().items.insert(it.external_id.clone(), it.clone());
            let mut conn = self.db.lock().unwrap();
            let r = import_items(&mut conn, &self.dir, &self.link, vec![(it, "r-web".into())], 1).unwrap();
            r.imported.into_iter().next().unwrap()
        }

        fn set_link(&mut self, f: impl FnOnce(&mut SourceLink)) {
            f(&mut self.link);
            store::save_link(&self.db.lock().unwrap(), &self.link).unwrap();
        }

        fn run(&self, now: i64) -> SyncReport {
            self.run_with(now, false)
        }

        /// Sync manual: sin pausa y releyendo estados.
        fn run_forced(&self, now: i64) -> SyncReport {
            self.run_with(now, true)
        }

        fn run_with(&self, now: i64, force: bool) -> SyncReport {
            let mut memo = self.memo.borrow_mut();
            tauri::async_runtime::block_on(sync_run(&self.db, &self.dir, &self.providers, None, now, &mut memo, force))
        }

        fn task(&self, id: &str) -> Task {
            rows::get_task(&self.db.lock().unwrap(), id).unwrap().unwrap()
        }

        fn outbox(&self) -> Vec<OutboxItem> {
            store::due_outbox(&self.db.lock().unwrap(), "fake", i64::MAX).unwrap()
        }

        fn enqueue_status(&self, task_id: &str, s: TaskStatus, now: i64) {
            assert!(store::enqueue_status(&self.db.lock().unwrap(), task_id, s, now).unwrap());
        }

        fn enqueue_comment(&self, task_id: &str, body: &str, now: i64) {
            assert!(store::enqueue_comment(&self.db.lock().unwrap(), task_id, body, now).unwrap());
        }
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff_ms(1), 30_000);
        assert_eq!(backoff_ms(2), 60_000);
        assert_eq!(backoff_ms(3), 120_000);
        assert_eq!(backoff_ms(50), BACKOFF_MAX_MS);
    }

    #[test]
    fn push_retries_with_backoff_then_succeeds() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        env.enqueue_status(&t.id, TaskStatus::InReview, 100);
        env.enqueue_comment(&t.id, "cierre", 100);
        env.fake.data().fail_writes =
            Some(ProviderError::new(ErrorKind::Transient, "Linear is unavailable (HTTP 502)"));

        let r = env.run(100);
        assert_eq!(r.pushed, 0);
        // El comentario espera al cambio de estado que falló.
        assert_eq!(r.errors, vec!["ENG-1: Linear is unavailable (HTTP 502)"]);
        assert_eq!(env.outbox()[1].attempts, 0);
        let row = &env.outbox()[0];
        assert_eq!((row.attempts, row.next_attempt_at), (1, 100 + 30_000));
        assert_eq!(row.last_error.as_deref(), Some("Linear is unavailable (HTTP 502)"));
        assert!(env.task(&t.id).source.unwrap().sync_error.unwrap().contains("502"));

        // Antes de vencer el backoff no se reintenta.
        env.run(100 + 29_999);
        assert_eq!(env.outbox()[0].attempts, 1);
        env.run(100 + 30_000);
        let row = &env.outbox()[0];
        assert_eq!((row.attempts, row.next_attempt_at), (2, 100 + 30_000 + 60_000));

        env.fake.data().fail_writes = None;
        let r = env.run(200_000);
        assert_eq!(r.pushed, 2, "{r:?}");
        assert!(env.outbox().is_empty());
        assert_eq!(env.fake.data().set_states, vec![("uuid-1".to_string(), "s-review".to_string())]);
        let src = env.task(&t.id).source.unwrap();
        assert_eq!(src.external_state.unwrap().id, "s-review");
        assert_eq!(src.sync_error, None);
    }

    #[test]
    fn comment_waits_for_earlier_state_in_backoff() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        env.enqueue_status(&t.id, TaskStatus::InReview, 100);
        env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::Transient, "HTTP 502"));
        env.run(100);
        env.fake.data().fail_writes = None;
        // El comentario llega mientras el cambio de estado espera su backoff: no se adelanta.
        env.enqueue_comment(&t.id, "cierre", 200);
        let r = env.run(1_000);
        assert_eq!(r.pushed, 0, "{r:?}");
        assert!(env.fake.data().comments.is_empty());
        let r = env.run(100 + 30_000);
        assert_eq!(r.pushed, 2, "{r:?}");
        assert_eq!(env.fake.data().set_states.len(), 1);
        assert_eq!(env.fake.data().comments.len(), 1);
    }

    #[test]
    fn comment_already_posted_is_not_reposted() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        env.enqueue_comment(&t.id, "cierre", 1);
        let id = env.outbox()[0].id;
        // Un intento anterior llegó a Linear pero la fila no se marcó (timeout, crash).
        env.fake.data().comments.push(("uuid-1".into(), marked_body("cierre", id)));
        let r = env.run(10);
        assert_eq!(r.pushed, 1, "{r:?}");
        assert!(env.outbox().is_empty());
        assert_eq!(env.fake.data().comments.len(), 1, "not reposted");
    }

    #[test]
    fn permanent_errors_and_attempt_cap_drop_the_row() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        env.enqueue_comment(&t.id, "hola", 1);
        env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::Permanent, "Entity not found"));
        let r = env.run(10);
        assert!(r.errors[0].contains("not retried"), "{r:?}");
        assert!(env.outbox().is_empty());
        assert!(env.task(&t.id).source.unwrap().sync_error.unwrap().contains("Entity not found"));

        env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::Transient, "timeout"));
        env.enqueue_comment(&t.id, "hola", 20);
        env.db.lock().unwrap().execute("UPDATE sync_outbox SET attempts = ?1", [MAX_ATTEMPTS - 1]).unwrap();
        env.run(30);
        assert!(env.outbox().is_empty(), "gave up after MAX_ATTEMPTS");
    }

    #[test]
    fn rate_limit_stops_draining_the_provider() {
        let env = Env::new();
        let a = env.import(1, "s-todo");
        let b = env.import(2, "s-todo");
        env.enqueue_comment(&a.id, "uno", 1);
        env.enqueue_comment(&b.id, "dos", 1);
        env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::RateLimited, "rate limited"));
        let r = env.run(10);
        assert_eq!(r.errors.len(), 1, "{r:?}");
        let rows = env.outbox();
        // El rate limit no gasta intentos: solo corre el próximo.
        assert_eq!((rows[0].attempts, rows[1].attempts), (0, 0));
        assert_eq!(rows[0].next_attempt_at, 10 + BACKOFF_BASE_MS);
    }

    #[test]
    fn rate_limit_pauses_the_provider_between_passes() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        env.fake.data().fail_states = Some(ProviderError::new(ErrorKind::RateLimited, "rate limited"));
        let r = env.run(10);
        assert!(r.notices.iter().any(|n| n.contains("fake: sync paused until")), "{r:?}");
        env.fake.data().fail_states = None;
        env.fake.data().items.get_mut("uuid-1").unwrap().title = "Nuevo".into();
        let calls = env.fake.data().states_calls;
        // En pausa: el worker no toca el proveedor.
        let r = env.run(20);
        assert_eq!((r.pulled, env.fake.data().states_calls), (0, calls), "{r:?}");
        assert_eq!(env.task(&t.id).title, t.title);
        let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
        assert!(l.last_sync_error.unwrap().contains("rate limited"), "the link keeps the reason");
        // Vencida la pausa, vuelve a sincronizar.
        let r = env.run(10 + RATE_LIMIT_PAUSE_MS);
        assert_eq!(r.pulled, 1, "{r:?}");
        assert_eq!(env.task(&t.id).title, "Nuevo");
        assert!(env.memo.borrow().paused.is_empty());
    }

    #[test]
    fn manual_sync_ignores_the_pause() {
        let env = Env::new();
        env.import(1, "s-todo");
        env.fake.data().fail_states = Some(ProviderError::new(ErrorKind::Auth, "key revoked"));
        env.run(10);
        assert_eq!(env.memo.borrow().paused["fake"].until, 10 + AUTH_PAUSE_MS);
        env.fake.data().fail_states = None;
        assert_eq!(env.run_forced(20).pulled, 1);
        assert!(env.memo.borrow().paused.is_empty());
    }

    #[test]
    fn saving_the_mapping_invalidates_cached_states() {
        let mut env = Env::new();
        let t = env.import(1, "s-todo");
        env.run(10);
        // Estado nuevo en el proveedor, mapeado por el usuario dentro del TTL.
        let qa = ExternalState { id: "s-qa".into(), name: "QA".into(), kind: ExtKind::Started, color: None };
        env.fake.data().states.push(qa);
        env.set_link(|l| {
            l.state_map.push.insert(TaskStatus::InReview, Some("s-qa".into()));
        });
        env.enqueue_status(&t.id, TaskStatus::InReview, 20);
        let r = env.run(30);
        assert_eq!(r.pushed, 1, "{r:?}");
        assert_eq!(env.fake.data().set_states, vec![("uuid-1".to_string(), "s-qa".to_string())]);
    }

    #[test]
    fn states_are_cached_between_passes() {
        let env = Env::new();
        env.import(1, "s-todo");
        env.run(10);
        env.run(20);
        assert_eq!(env.fake.data().states_calls, 1);
        env.run(10 + STATES_TTL_MS);
        assert_eq!(env.fake.data().states_calls, 2);
        env.run_forced(10 + STATES_TTL_MS + 1);
        assert_eq!(env.fake.data().states_calls, 3);
    }

    #[test]
    fn auth_errors_never_drop_changes() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        env.enqueue_comment(&t.id, "hola", 1);
        env.db.lock().unwrap().execute("UPDATE sync_outbox SET attempts = ?1", [MAX_ATTEMPTS - 1]).unwrap();
        env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::Auth, "key revoked"));
        let mut now = 10;
        for _ in 0..20 {
            env.run(now);
            now += BACKOFF_MAX_MS;
        }
        let rows = env.outbox();
        assert_eq!(rows.len(), 1, "a revoked key keeps the change queued");
        assert_eq!(rows[0].attempts, MAX_ATTEMPTS - 1);
        env.fake.data().fail_writes = None;
        assert_eq!(env.run(now).pushed, 1);
    }

    #[test]
    fn unlinked_items_are_not_auto_imported_again() {
        let mut env = Env::new();
        env.set_link(|l| {
            l.auto_import = true;
            l.default_repo_id = Some("r-web".into());
        });
        let t = env.import(1, "s-todo");
        store::unlink_task(&env.db.lock().unwrap(), &t.id, 5).unwrap();
        assert_eq!(env.run(10).imported, 0);
        assert!(store::task_by_external(&env.db.lock().unwrap(), "fake", "uuid-1").unwrap().is_none());
        // Los settings conocidos no se ven afectados por la lápida.
        assert_eq!(rows::load_settings(&env.db.lock().unwrap()).unwrap(), Settings::default());
    }

    #[test]
    fn pending_map_skips_state_but_sends_comment() {
        let mut env = Env::new();
        let t = env.import(1, "s-todo");
        env.set_link(|l| l.state_map.confirmed_at = None);
        env.enqueue_status(&t.id, TaskStatus::InReview, 1);
        env.enqueue_comment(&t.id, "**Nodal** · In Review", 1);
        let r = env.run(10);
        assert_eq!(r.pushed, 1);
        assert!(env.outbox().is_empty());
        let d = env.fake.data();
        assert!(d.set_states.is_empty());
        assert_eq!(d.comments.len(), 1);
        assert_eq!(d.comments[0].0, "uuid-1");
        assert!(d.comments[0].1.starts_with("**Nodal** · In Review\n\n<!-- nodal:outbox:"), "{:?}", d.comments);
    }

    #[test]
    fn no_sync_and_vanished_targets_are_dropped() {
        let mut env = Env::new();
        let t = env.import(1, "s-todo");
        env.set_link(|l| {
            l.state_map.push.insert(TaskStatus::Blocked, None);
        });
        env.enqueue_status(&t.id, TaskStatus::Blocked, 1);
        let r = env.run(10);
        assert_eq!((r.pushed, env.outbox().len()), (0, 0));
        assert!(env.fake.data().set_states.is_empty());

        // "In Review" desaparece del proveedor: no se empuja y se avisa.
        env.fake.data().states.retain(|s| s.id != "s-review");
        env.enqueue_status(&t.id, TaskStatus::InReview, 20);
        let r = env.run_forced(30);
        assert!(env.fake.data().set_states.is_empty());
        assert!(r.notices.iter().any(|n| n.contains("\"In Review\" no longer exists")), "{r:?}");
        assert!(env.task(&t.id).source.unwrap().sync_error.unwrap().contains("no longer exists"));
    }

    #[test]
    fn todo_is_pushed_when_mapped_and_silently_skipped_otherwise() {
        let mut env = Env::new();
        let t = env.import(1, "s-progress");
        env.enqueue_status(&t.id, TaskStatus::Todo, 1);
        let r = env.run(10);
        assert_eq!((r.pushed, r.errors.len(), r.notices.len()), (1, 0, 0), "{r:?}");
        assert_eq!(env.fake.data().set_states, vec![("uuid-1".to_string(), "s-todo".to_string())]);

        // Mapeo confirmado antes de que existiera la fila de Todo: no se sincroniza, sin aviso.
        env.set_link(|l| {
            l.state_map.push.remove(&TaskStatus::Todo);
        });
        env.fake.data().items.get_mut("uuid-1").unwrap().state = state("s-progress");
        env.run_forced(20);
        env.enqueue_status(&t.id, TaskStatus::Todo, 30);
        let r = env.run(40);
        assert_eq!((r.pushed, r.errors.len(), r.notices.len()), (0, 0, 0), "{r:?}");
        assert!(env.outbox().is_empty());
        assert_eq!(env.fake.data().set_states.len(), 1);
    }

    #[test]
    fn enqueue_status_keeps_only_latest_and_ignores_local_tasks() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        env.enqueue_status(&t.id, TaskStatus::InProgress, 1);
        env.enqueue_status(&t.id, TaskStatus::InReview, 2);
        let rows = env.outbox();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].payload, OutboxPayload::SetState { state_id: "in_review".into() });
        let conn = env.db.lock().unwrap();
        store::unlink_task(&conn, &t.id, 3).unwrap();
        assert!(!store::enqueue_status(&conn, &t.id, TaskStatus::Blocked, 4).unwrap());
        drop(conn);
        assert!(env.outbox().is_empty(), "unlink clears the outbox");
    }

    #[test]
    fn pull_updates_title_plan_and_status() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        {
            let mut d = env.fake.data();
            let i = d.items.get_mut("uuid-1").unwrap();
            i.title = "Título nuevo".into();
            i.description_md = Some("Descripción nueva.".into());
            i.state = state("s-done");
        }
        let r = env.run(50);
        assert_eq!(r.pulled, 1, "{r:?}");
        let t2 = env.task(&t.id);
        assert_eq!(t2.title, "Título nuevo");
        assert_eq!(t2.status, TaskStatus::Done);
        assert_eq!(t2.closed_at, Some(50));
        assert_eq!(t2.source.as_ref().unwrap().external_state.as_ref().unwrap().id, "s-done");
        let plan = std::fs::read_to_string(plan_path(&env.dir, &t.id)).unwrap();
        assert!(plan.contains("Descripción nueva.") && plan.contains("# ENG-1 · Título nuevo"));

        // Cambio local en Nodal con el estado externo quieto: el pull no lo pisa.
        env.db.lock().unwrap().execute("UPDATE tasks SET status = 'blocked' WHERE id = ?1", [&t.id]).unwrap();
        env.run(60);
        assert_eq!(env.task(&t.id).status, TaskStatus::Blocked);
    }

    #[test]
    fn pull_does_not_overwrite_a_pending_local_status() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        env.enqueue_status(&t.id, TaskStatus::InReview, 1);
        env.db.lock().unwrap().execute("UPDATE tasks SET status = 'in_review' WHERE id = ?1", [&t.id]).unwrap();
        env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::Transient, "HTTP 502"));
        // Mientras el push espera, alguien mueve el ítem en el proveedor.
        env.fake.data().items.get_mut("uuid-1").unwrap().state = state("s-done");
        env.run(10);
        let t2 = env.task(&t.id);
        assert_eq!(t2.status, TaskStatus::InReview);
        assert_eq!(t2.source.unwrap().external_state.unwrap().id, "s-todo", "no se da por visto");
    }

    #[test]
    fn pull_respects_overridden_plan() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        let path = plan_path(&env.dir, &t.id);
        std::fs::write(&path, "mi plan").unwrap();
        env.db.lock().unwrap().execute("UPDATE tasks SET plan_overridden = 1 WHERE id = ?1", [&t.id]).unwrap();
        env.fake.data().items.get_mut("uuid-1").unwrap().description_md = Some("otra".into());
        env.run(10);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mi plan");
    }

    #[test]
    fn pull_does_not_touch_status_with_active_run() {
        let env = Env::new();
        let t = env.import(1, "s-progress");
        env.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO runs (id, task_id, cwd, executor_json, kind, prompt, finish, status, queue_position, queued_at)
                 VALUES ('run1', ?1, '/tmp', '{\"kind\":\"claude\"}', 'work', 'p', 'pr', 'launched', 1, 1)",
                [&t.id],
            )
            .unwrap();
        env.fake.data().items.get_mut("uuid-1").unwrap().state = state("s-canceled");
        env.run(10);
        let t2 = env.task(&t.id);
        assert_eq!(t2.status, TaskStatus::InProgress);
        // El cambio queda consumido: al terminar el run no se aplica tarde.
        assert_eq!(t2.source.unwrap().external_state.unwrap().id, "s-canceled");
        env.db.lock().unwrap().execute("UPDATE runs SET status = 'finished'", []).unwrap();
        env.run(20);
        assert_eq!(env.task(&t.id).status, TaskStatus::InProgress);
    }

    #[test]
    fn pull_marks_unmapped_and_missing() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        let t2 = env.import(2, "s-todo");
        let qa = ExternalState { id: "s-qa".into(), name: "QA".into(), kind: ExtKind::Started, color: None };
        {
            let mut d = env.fake.data();
            d.states.push(qa.clone());
            d.items.get_mut("uuid-1").unwrap().state = qa;
            d.items.remove("uuid-2");
        }
        let r = env.run(10);
        assert!(r.notices.iter().any(|n| n.contains("new states to map: QA")), "{r:?}");
        let a = env.task(&t.id);
        assert_eq!(a.status, TaskStatus::Todo);
        assert!(a.source.as_ref().unwrap().sync_error.as_ref().unwrap().contains("\"QA\" is not mapped"));
        assert!(a.source.as_ref().unwrap().unmapped);
        let b = env.task(&t2.id);
        let bs = b.source.unwrap();
        assert!(bs.sync_error.unwrap().contains("Not found"));
        assert!(!bs.unmapped);
        assert_eq!(r.pulled, 1);
        let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
        assert_eq!((l.last_synced_at, l.last_sync_error), (Some(10), None), "un ítem faltante no es error del link");
        let pending = l.pending_state_changes.expect("QA queda pendiente de revisar");
        assert_eq!(pending.added.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s-qa"]);
        assert!(pending.removed.is_empty());
        // Sin mapear no se da por visto: cuando se mapea, el pull siguiente lo aplica.
        assert_eq!(a.source.as_ref().unwrap().external_state.as_ref().unwrap().id, "s-todo");
        let mut map = env.link.state_map.clone();
        map.pull.insert("s-qa".into(), TaskStatus::InReview);
        map.known_states = env.fake.data().states.clone();
        store::save_state_map(&env.db.lock().unwrap(), &env.link.id, &map).unwrap();
        let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
        assert_eq!(l.pending_state_changes, None, "guardar el mapeo limpia lo pendiente");
        env.run(20);
        let a = env.task(&t.id);
        assert_eq!(a.status, TaskStatus::InReview);
        let src = a.source.unwrap();
        assert_eq!(src.sync_error, None);
        assert!(!src.unmapped);
        let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
        assert_eq!((l.last_synced_at, l.pending_state_changes), (Some(20), None));
    }

    #[test]
    fn link_sync_error_is_recorded_and_cleared() {
        let env = Env::new();
        env.import(1, "s-todo");
        env.fake.data().fail_states = Some(ProviderError { kind: ErrorKind::Transient, message: "boom".into() });
        let r = env.run(10);
        assert!(!r.errors.is_empty());
        let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
        assert_eq!(l.last_synced_at, Some(10));
        assert!(l.last_sync_error.unwrap().contains("boom"));
        env.fake.data().fail_states = None;
        env.run(20);
        let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
        assert_eq!((l.last_synced_at, l.last_sync_error), (Some(20), None));
    }

    #[test]
    fn auto_import_routes_by_rule_or_reports() {
        let mut env = Env::new();
        env.set_link(|l| l.auto_import = true);
        {
            let mut d = env.fake.data();
            let mut docs = item(10, Some("## Criterios\n- Doc publicada\n"));
            docs.labels = vec!["docs".into()];
            let plain = item(11, None);
            let mut closed = item(12, None);
            closed.state = state("s-done");
            for i in [docs, plain, closed] {
                d.items.insert(i.external_id.clone(), i);
            }
        }
        let r = env.run(10);
        assert_eq!(r.imported, 1, "{r:?}");
        assert!(r.notices.iter().any(|n| n.starts_with("ENG-11: not auto-imported")), "{r:?}");
        let t = store::task_by_external(&env.db.lock().unwrap(), "fake", "uuid-10").unwrap().unwrap();
        assert_eq!(t.repo_id, "r-docs");
        assert_eq!(t.acceptance, vec!["Doc publicada"]);
        assert!(store::task_by_external(&env.db.lock().unwrap(), "fake", "uuid-12").unwrap().is_none());

        // Con repo por defecto, la que faltaba entra; la ya importada no se duplica.
        env.set_link(|l| l.default_repo_id = Some("r-web".into()));
        let r = env.run(20);
        assert_eq!(r.imported, 1, "{r:?}");
        let t = store::task_by_external(&env.db.lock().unwrap(), "fake", "uuid-11").unwrap().unwrap();
        assert_eq!(t.repo_id, "r-web");
        assert_eq!(env.run(30).imported, 0);
    }

    #[test]
    fn auto_import_with_deleted_repo_is_reported_not_retried() {
        let mut env = Env::new();
        // `default_repo_id` tiene FK (ON DELETE SET NULL); las reglas no: pueden quedar colgadas.
        env.set_link(|l| {
            l.auto_import = true;
            l.repo_rules = vec![RepoRule { label: "api".into(), repo_id: "r-gone".into() }];
        });
        let mut it = item(20, None);
        it.labels = vec!["api".into()];
        env.fake.data().items.insert("uuid-20".into(), it);
        let r = env.run(10);
        assert_eq!(r.imported, 0);
        assert!(r.errors.iter().any(|e| e.contains("waiting for 1 item (ENG-20)")), "{r:?}");
        assert!(!r.errors.iter().any(|e| e.starts_with("Auto-import")), "no import attempt: {r:?}");
        let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
        assert!(l.last_sync_error.unwrap().contains("no longer exists"));
        env.set_link(|l| l.repo_rules[0].repo_id = "r-web".into());
        assert_eq!(env.run(20).imported, 1);
    }

    #[test]
    fn disconnect_leaves_tasks_local() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        env.enqueue_comment(&t.id, "hola", 1);
        let n = store::disconnect_link(&mut env.db.lock().unwrap(), "l1", 5).unwrap();
        assert_eq!(n, 1);
        let t2 = env.task(&t.id);
        assert!(t2.source.is_none());
        assert_eq!(t2.title, t.title);
        assert!(env.outbox().is_empty());
        assert!(rows::get_source_link(&env.db.lock().unwrap(), "l1").unwrap().is_none());
    }

    #[test]
    fn decide_pull_is_pure() {
        let env = Env::new();
        let t = env.import(1, "s-todo");
        let mut it = item(1, None);
        it.state = state("s-review");
        let d = decide_pull(&t, &it, &env.link.state_map, false, false);
        assert_eq!(d.status, Some(TaskStatus::InReview));
        assert_eq!(d.title, None);
        assert_eq!(decide_pull(&t, &it, &env.link.state_map, true, false).status, None);
        let pending = decide_pull(&t, &it, &env.link.state_map, false, true);
        assert_eq!((pending.status, pending.record_state, pending.unmapped), (None, false, false));
        it.state = state("s-todo");
        assert_eq!(decide_pull(&t, &it, &env.link.state_map, false, false).status, None);
    }
}
