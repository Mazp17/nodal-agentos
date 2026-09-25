//! Linear in read-only mode. All calls go out from Rust, so the API key never goes
//! through the webview and the CSP does not need to open `connect-src` to linear.app.

pub(crate) mod client;
mod detail;
mod error;
mod key;
pub(crate) mod model;

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
    /// `secrets` is the shared instance (`State<Secrets>`): a key saved from `linear_*`
    /// or from `provider_*` is visible on both sides.
    pub fn new(secrets: crate::secrets::Secrets) -> Self {
        Self { http: http_client(), key: KeyCache::new(secrets) }
    }

    /// For `providers::linear`: same HTTP client.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
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

/// Validates the key against Linear and stores it in the keychain only if it is valid.
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

/// Open issues + issues closed in the last 14 days. Empty or missing `team_ids` = all.
#[tauri::command]
pub async fn linear_board(
    state: State<'_, LinearState>,
    team_ids: Option<Vec<String>>,
) -> Result<Board, LinearError> {
    let key = state.key.require().await?;
    LinearClient::new(&state.http, &key).board(team_ids.as_deref()).await
}

/// Full detail of an issue for the side panel. `issue_id` accepts a UUID or an
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
    //! Against the real API. `cargo test -- --ignored live_` with LINEAR_API_KEY in the
    //! environment. Never prints the key.
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

    /// Needs no real key: confirms Linear rejects a fake key and that this maps to
    /// `InvalidKey` (real format of the auth error).
    #[test]
    #[ignore]
    fn live_bogus_key_is_invalid() {
        tauri::async_runtime::block_on(async {
            let http = http_client();
            let err = LinearClient::new(&http, "lin_api_fake_key_for_test").viewer().await;
            assert_eq!(err.unwrap_err(), LinearError::InvalidKey);
        });
    }
}
