//! Linear en modo sólo lectura. Todas las llamadas salen desde Rust, así la API key
//! no pasa por el webview y la CSP no necesita abrir `connect-src` a linear.app.

mod client;
mod detail;
mod error;
mod key;
mod model;

use client::{http_client, LinearClient};
use detail::IssueDetail;
pub use error::LinearError;
use key::KeyCache;
use model::{Board, Team, Viewer};
use serde::Serialize;
use tauri::State;

pub struct LinearState {
    http: reqwest::Client,
    key: KeyCache,
}

impl LinearState {
    pub fn new() -> Self {
        Self { http: http_client(), key: KeyCache::default() }
    }
}

#[derive(Serialize)]
pub struct KeyStatus {
    configured: bool,
}

#[tauri::command]
pub async fn linear_key_status(state: State<'_, LinearState>) -> Result<KeyStatus, LinearError> {
    Ok(KeyStatus { configured: state.key.load().await?.is_some() })
}

/// Valida la key contra Linear y sólo si es válida la guarda en el llavero.
#[tauri::command]
pub async fn linear_set_api_key(
    state: State<'_, LinearState>,
    key: String,
) -> Result<Viewer, LinearError> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(LinearError::MissingKey);
    }
    let viewer = LinearClient::new(&state.http, &key).viewer().await?;
    state.key.store(key).await?;
    Ok(viewer)
}

#[tauri::command]
pub async fn linear_clear_api_key(state: State<'_, LinearState>) -> Result<(), LinearError> {
    state.key.clear().await
}

#[tauri::command]
pub async fn linear_viewer(state: State<'_, LinearState>) -> Result<Viewer, LinearError> {
    let key = state.key.require().await?;
    LinearClient::new(&state.http, &key).viewer().await
}

#[tauri::command]
pub async fn linear_teams(state: State<'_, LinearState>) -> Result<Vec<Team>, LinearError> {
    let key = state.key.require().await?;
    LinearClient::new(&state.http, &key).teams().await
}

/// Issues abiertas + cerradas en los últimos 14 días. `team_ids` vacío o ausente = todos.
#[tauri::command]
pub async fn linear_board(
    state: State<'_, LinearState>,
    team_ids: Option<Vec<String>>,
) -> Result<Board, LinearError> {
    let key = state.key.require().await?;
    LinearClient::new(&state.http, &key).board(team_ids.as_deref()).await
}

/// Detalle completo de una issue para el panel lateral. `issue_id` acepta UUID o
/// identifier ("ACME-8").
#[tauri::command]
pub async fn linear_issue_detail(
    state: State<'_, LinearState>,
    issue_id: String,
) -> Result<IssueDetail, LinearError> {
    let key = state.key.require().await?;
    LinearClient::new(&state.http, &key).issue_detail(&issue_id).await
}

#[cfg(test)]
mod live_tests {
    //! Contra la API real. `cargo test -- --ignored live_` con LINEAR_API_KEY en el
    //! entorno. Nunca imprime la key.
    use super::*;

    fn env_key() -> Option<String> {
        std::env::var("LINEAR_API_KEY").ok().filter(|k| !k.trim().is_empty())
    }

    #[test]
    #[ignore]
    fn live_viewer_teams_board() {
        let Some(key) = env_key() else {
            eprintln!("LINEAR_API_KEY not set: skipping live test");
            return;
        };
        tauri::async_runtime::block_on(async {
            let http = http_client();
            let c = LinearClient::new(&http, key.trim());
            let v = c.viewer().await.expect("viewer");
            println!("viewer: {}", v.name);
            let teams = c.teams().await.expect("teams");
            let keys: Vec<_> = teams.iter().map(|t| t.key.as_str()).collect();
            println!("{} teams: {}", teams.len(), keys.join(", "));
            let board = c.board(None).await.expect("board");
            println!(
                "board: {} issues, {} teams with states, truncated={}",
                board.issues.len(),
                board.teams.len(),
                board.truncated
            );
            if let Some(first) = board.issues.first() {
                let d = c.issue_detail(&first.identifier).await.expect("issue detail");
                println!(
                    "detail {}: {} children, {} relations, {} comments",
                    d.identifier,
                    d.children.len(),
                    d.relations.len(),
                    d.comments.len()
                );
            }
        });
    }

    /// No necesita key real: confirma que Linear rechaza una key falsa y que eso se
    /// traduce a `InvalidKey` (formato real del error de auth).
    #[test]
    #[ignore]
    fn live_bogus_key_is_invalid() {
        tauri::async_runtime::block_on(async {
            let http = http_client();
            let err = LinearClient::new(&http, "lin_api_clave_falsa_para_test").viewer().await;
            assert_eq!(err.unwrap_err(), LinearError::InvalidKey);
        });
    }
}
