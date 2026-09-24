//! Proveedores de tareas (Linear hoy; Asana, Azure DevOps después) y su sync con Nodal.
//!
//! - `TaskProvider`: interfaz genérica de un proveedor. Despacho estático por el enum
//!   `Provider` (sin `dyn`); cada adapter vive en su archivo (`linear.rs`).
//! - `state_map`: propuesta de mapeo de estados (pura).
//! - `plan`: materialización de `plan.md`, extracción de criterios y comentario de cierre.
//! - `import`: importación de ítems como tareas.
//! - `sync`: worker (pull, push del outbox, auto-import) y `sync_now`.
//! - `store`: SQL sobre `tasks`, `source_links`, `sync_outbox` y `runs` que usa todo lo
//!   anterior (vive acá para no pisar `db/queries`).
//! - `commands`: comandos de Tauri.
//!
//! API para la cola (`work`): `enqueue_status` y `enqueue_comment` agregan filas al outbox en
//! la misma transacción que la transición, y `plan::closing_comment` arma el comentario.

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

use crate::domain::{ExtKind, ExternalState, Priority, ScopeRef};

pub use store::{enqueue_comment, enqueue_status};

/// Los comandos de Tauri rechazan con un string listo para mostrar (en inglés).
pub type PResult<T> = Result<T, String>;

/// Clase de error de un proveedor: decide qué hace el outbox con la fila.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Red, 5xx: reintentar con backoff (hasta `sync::MAX_ATTEMPTS`).
    Transient,
    /// Reintentar más tarde y no seguir drenando este proveedor en esta pasada.
    RateLimited,
    /// Key ausente o inválida: igual que `RateLimited`.
    Auth,
    /// El proveedor rechazó la escritura (ítem borrado, sin permiso, input inválido): no
    /// tiene sentido reintentar.
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
            // La clasificación viene del status HTTP / `extensions.code` (`linear::model`).
            L::Network(_) | L::Keychain(_) | L::Unavailable(_) => ErrorKind::Transient,
            L::Api(_) => ErrorKind::Permanent,
        };
        ProviderError::new(kind, e.to_string())
    }
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// Proveedores que Nodal conoce (el resto se rechaza con "Unknown provider").
pub const KNOWN_PROVIDERS: &[&str] = &["linear"];

/// Tipos de estado "abiertos": el default del listado de importables y del auto-import.
pub const OPEN_KINDS: &[ExtKind] = &[ExtKind::Triage, ExtKind::Backlog, ExtKind::Unstarted, ExtKind::Started];

// ---------- Modelo genérico ----------

/// Referencia liviana a otro ítem (el padre).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemRef {
    pub external_id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
}

/// Sub-ítem (sub-issue), con su estado para marcar las ya terminadas.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildItem {
    pub external_id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state: ExternalState,
}

/// Un ítem del proveedor (issue de Linear, tarea de Asana, work item de ADO).
/// En el listado de importables `description_md`, `parent`, `children` y `assignee` pueden
/// venir vacíos: para importar se vuelve a pedir el ítem completo con `pull`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalItem {
    /// Id estable del proveedor (UUID en Linear). Único por proveedor.
    pub external_id: String,
    /// Id legible (`ENG-142`).
    pub identifier: String,
    pub url: String,
    pub title: String,
    pub description_md: Option<String>,
    pub state: ExternalState,
    /// Scopes a los que pertenece (team y, si tiene, proyecto).
    pub scopes: Vec<ScopeRef>,
    pub parent: Option<ItemRef>,
    pub children: Vec<ChildItem>,
    pub labels: Vec<String>,
    /// Nombre visible del asignado en el proveedor (informativo).
    pub assignee: Option<String>,
    pub priority: Priority,
    /// ISO 8601, tal cual lo da el proveedor.
    pub updated_at: String,
}

/// Consulta del listado de importables.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportQuery {
    pub scope: ScopeRef,
    /// Texto libre: título o identifier.
    pub text: Option<String>,
    /// Vacío = todos los tipos.
    pub state_kinds: Vec<ExtKind>,
    /// Solo ítems creados después de este instante (epoch ms). Lo usa el auto-import.
    pub created_after: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Page {
    pub items: Vec<ExternalItem>,
    pub next_cursor: Option<String>,
}

/// Interfaz de un proveedor de tareas. Los adapters normalizan sus estados a `ExtKind`
/// para que la heurística de mapeo (`state_map`) sirva a todos.
// Crate privado: el lint de `async fn` en traits públicos (por los bounds `Send`) no aplica;
// el despacho es por enum concreto, así que los futures son `Send` cuando hace falta.
#[allow(async_fn_in_trait)]
pub trait TaskProvider {
    /// Nombre del proveedor (`"linear"`), igual a `TaskSource.provider`.
    fn name(&self) -> &'static str;
    /// Valida la key; devuelve el nombre del usuario.
    async fn status(&self) -> ProviderResult<String>;
    /// Scopes vinculables (en Linear: teams y proyectos activos).
    async fn scopes(&self) -> ProviderResult<Vec<ScopeRef>>;
    /// Estados actuales del scope, en orden del proveedor.
    async fn states(&self, scope: &ScopeRef) -> ProviderResult<Vec<ExternalState>>;
    async fn list_importable(&self, query: &ImportQuery) -> ProviderResult<Page>;
    /// Ítems completos por id. Los que ya no existen (o la key no ve) no vuelven.
    async fn pull(&self, external_ids: &[String]) -> ProviderResult<Vec<ExternalItem>>;
    #[allow(dead_code)] // Para el detalle (hoy `linear_issue_detail`) y proveedores futuros.
    async fn fetch(&self, external_id: &str) -> ProviderResult<Option<ExternalItem>> {
        Ok(self.pull(&[external_id.to_string()]).await?.into_iter().next())
    }
    /// Cambia el estado del ítem. `state_id` sale del mapeo; si pertenece a otro grupo de
    /// estados que el del ítem (proyecto de Linear con varios teams), el adapter usa el
    /// equivalente por nombre/tipo del grupo del ítem. Devuelve el estado resultante.
    async fn set_state(&self, external_id: &str, state_id: &str) -> ProviderResult<ExternalState>;
    async fn comment(&self, external_id: &str, body: &str) -> ProviderResult<()>;
    /// Si el ítem ya tiene un comentario que contiene `marker` (el outbox lo usa para no
    /// repostear un comentario que salió pero no llegó a marcarse como enviado).
    async fn has_comment_with(&self, external_id: &str, marker: &str) -> ProviderResult<bool>;
}

/// Despacho estático de proveedores.
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

/// Proveedor listo para usar, o `None` si no hay key guardada.
pub async fn resolve(app: &AppHandle, name: &str) -> PResult<Option<Provider>> {
    check_provider(name)?;
    Ok(resolve_with_key(app, name).await?.map(|(p, _)| p))
}

/// Como `resolve`, más la key leída (para derivar `key_hint` sin leer el keychain dos veces).
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

/// Keys más cortas no muestran pista: los últimos 4 serían demasiado de la key.
const KEY_HINT_MIN_CHARS: usize = 12;

/// Últimos 4 caracteres de la key para reconocerla en la UI; nunca la key entera.
pub fn key_hint(key: &str) -> Option<String> {
    let chars: Vec<char> = key.trim().chars().collect();
    (chars.len() >= KEY_HINT_MIN_CHARS).then(|| chars[chars.len() - 4..].iter().collect())
}

pub async fn require(app: &AppHandle, name: &str) -> PResult<Provider> {
    resolve(app, name)
        .await?
        .ok_or_else(|| format!("The {name} API key is missing. Add it in Settings → Providers."))
}

// ---------- Estado y arranque ----------

pub struct ProvidersState {
    /// `app_data_dir`: los planes van en `tasks/<id>/plan.md`.
    pub data_dir: PathBuf,
    /// Un solo sync a la vez (worker y `sync_now`), con su memoria entre pasadas.
    pub sync_lock: tokio::sync::Mutex<sync::SyncMemo>,
    /// Proveedores en pausa por rate limit o key rechazada. Aparte del lock de sync para que
    /// `provider_status` y el cambio de key no esperen a una pasada en curso.
    pauses: std::sync::Mutex<std::collections::HashMap<String, sync::Pause>>,
}

impl ProvidersState {
    pub fn paused(&self) -> std::collections::HashMap<String, sync::Pause> {
        self.pauses.lock().map(|m| m.clone()).unwrap_or_default()
    }

    pub fn set_paused(&self, p: std::collections::HashMap<String, sync::Pause>) {
        if let Ok(mut m) = self.pauses.lock() {
            *m = p;
        }
    }

    /// Pausa vigente de un proveedor.
    pub fn pause_of(&self, provider: &str, now: i64) -> Option<sync::Pause> {
        self.paused().remove(provider).filter(|p| p.until > now)
    }

    /// Una key nueva (o borrada) levanta la pausa.
    pub fn clear_pause(&self, provider: &str) {
        if let Ok(mut m) = self.pauses.lock() {
            m.remove(provider);
        }
    }
}

/// Registra el estado y arranca el worker de sync. Necesita `LinearState` y la base
/// (`db::Db`) ya registrados; si la base no abrió, el worker no hace nada.
pub fn init(app: &AppHandle) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| format!("Couldn't find the app data folder: {e}"))?;
    app.manage(ProvidersState {
        data_dir,
        sync_lock: tokio::sync::Mutex::new(sync::SyncMemo::default()),
        pauses: Default::default(),
    });
    sync::spawn_worker(app.clone());
    Ok(())
}

// ---------- Utilidades ----------

/// Epoch ms → `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC), sin depender de chrono.
pub fn iso_from_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Algoritmo "civil from days" de Howard Hinnant.
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
        // El texto no decide: un rechazo que menciona "unavailable" sigue siendo permanente.
        assert_eq!(kind(400, &gql("INVALID_INPUT", "state unavailable for this team")), ErrorKind::Permanent);
        assert_eq!(kind(400, &gql("FORBIDDEN", "no")), ErrorKind::Permanent);
    }

    #[test]
    fn unknown_provider_is_rejected() {
        assert!(check_provider("linear").is_ok());
        assert!(check_provider("jira").unwrap_err().contains("Unknown provider"));
    }
}
