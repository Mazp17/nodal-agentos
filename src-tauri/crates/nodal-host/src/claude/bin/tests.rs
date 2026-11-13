use super::*;

#[test]
fn resolve_claude_and_git_are_memoized() {
    // `OnceLock`-backed (P06): repeated calls must keep returning the same result.
    assert_eq!(resolve_claude(), resolve_claude());
    assert_eq!(resolve_git(), resolve_git());
}

#[test]
fn augmented_path_is_stable_and_includes_extra_dirs() {
    let first = augmented_path();
    assert_eq!(first, augmented_path());
    for d in super::extra_dirs() {
        assert!(std::env::split_paths(&first).any(|p| p == d), "{d:?} missing from {first:?}");
    }
}

#[test]
fn resolve_git_finds_the_test_runner_s_git() {
    if !crate::testutil::git_available() {
        return;
    }
    let git = resolve_git().expect("git should resolve when it's on PATH");
    assert!(git.exists());
}
