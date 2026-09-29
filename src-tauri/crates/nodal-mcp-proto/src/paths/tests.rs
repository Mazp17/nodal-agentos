use super::*;

#[test]
fn standalone_data_dir_matches_the_app_and_keeps_debug_apart() {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../../../tauri.conf.json")).unwrap();
    assert_eq!(conf["identifier"], APP_IDENTIFIER);
    let dir = data_dir_standalone().unwrap();
    assert_eq!(
        dir.parent().unwrap(),
        home().unwrap().join("Library/Application Support")
    );
    assert_eq!(dir.to_string_lossy().ends_with(".nodal.dev"), DEV);
    assert_eq!(mcp_socket(&dir), dir.join("mcp.sock"));
}
