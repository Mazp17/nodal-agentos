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
