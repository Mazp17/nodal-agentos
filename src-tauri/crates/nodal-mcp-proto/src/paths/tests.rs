use super::*;

#[test]
fn standalone_data_dir_matches_the_app_and_keeps_debug_apart() {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../../../tauri.conf.json")).unwrap();
    assert_eq!(conf["identifier"], APP_IDENTIFIER);
    let dir = data_dir_standalone().unwrap();
    #[cfg(target_os = "macos")]
    assert_eq!(
        dir.parent().unwrap(),
        home().unwrap().join("Library/Application Support")
    );
    assert_eq!(dir.to_string_lossy().ends_with(".nodal.dev"), DEV);
    #[cfg(unix)]
    assert_eq!(mcp_socket(&dir), dir.join("mcp.sock"));
    #[cfg(windows)]
    assert_eq!(mcp_socket(&dir), PathBuf::from(pipe_name(&dir)));
}

#[test]
fn pipe_name_is_flat_distinct_per_data_dir_and_bounded() {
    let prod = pipe_name(Path::new(
        r"C:\Users\a\AppData\Roaming\io.github.mazp17.nodal",
    ));
    let dev = pipe_name(Path::new(
        r"C:\Users\a\AppData\Roaming\io.github.mazp17.nodal.dev",
    ));
    assert_eq!(
        prod,
        r"\\.\pipe\nodal-mcp-C__Users_a_AppData_Roaming_io.github.mazp17.nodal"
    );
    assert_ne!(prod, dev);
    assert!(!prod[r"\\.\pipe\".len()..].contains(['\\', '/']));
    let long = pipe_name(Path::new(&format!("/{}/x.dev", "a".repeat(400))));
    assert!(long.len() <= r"\\.\pipe\".len() + 256 && long.ends_with("x.dev"));
}
