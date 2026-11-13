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

#[cfg(unix)]
#[test]
fn spawn_retrying_retries_text_file_busy_then_gives_up() {
    let busy = || std::io::Error::from_raw_os_error(libc::ETXTBSY);
    let mut calls = 0;
    let ok = spawn_retrying(|| {
        calls += 1;
        if calls < 3 { Err(busy()) } else { Ok(calls) }
    });
    assert_eq!(ok.unwrap(), 3);

    let mut calls = 0;
    let err = spawn_retrying::<()>(|| {
        calls += 1;
        Err(busy())
    });
    assert_eq!(err.unwrap_err().raw_os_error(), Some(libc::ETXTBSY));
    assert_eq!(calls, 6);

    let mut calls = 0;
    let other = spawn_retrying::<()>(|| {
        calls += 1;
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    });
    assert!(other.is_err());
    assert_eq!(calls, 1, "other errors aren't retried");
}
