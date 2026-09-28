#[test]
fn git_version_reports_git() {
    if !crate::util::paths::tests::git_available() {
        return;
    }
    let v = tauri::async_runtime::block_on(super::git_version()).unwrap();
    assert!(v.starts_with("git version "), "{v}");
}
