use super::*;

#[test]
fn debug_builds_do_not_check() {
    assert!(!enabled(true));
    assert!(enabled(false));
    assert_eq!(updates_enabled(), !cfg!(debug_assertions));
}

#[test]
fn config_ships_signed_updater_artifacts() {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    assert_eq!(conf["bundle"]["createUpdaterArtifacts"], true);
    let updater = &conf["plugins"]["updater"];
    assert!(!updater["pubkey"].as_str().unwrap_or_default().is_empty());
    let endpoints = updater["endpoints"].as_array().unwrap();
    assert!(!endpoints.is_empty());
    for e in endpoints {
        let url = e.as_str().unwrap();
        assert!(
            url.starts_with("https://") && url.ends_with("/latest.json"),
            "{url}"
        );
    }
}
