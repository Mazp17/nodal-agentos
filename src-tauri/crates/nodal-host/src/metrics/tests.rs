use super::*;

// Counters are process-global statics: serialize the two tests that touch them so they don't
// observe each other's increments under `cargo test`'s default multi-threaded runner.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn claude_spawn_counter_increments() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    reset_for_test();
    assert_eq!(claude_spawn_count(), 0);
    assert_eq!(record_claude_spawn(), 1);
    assert_eq!(record_claude_spawn(), 2);
    assert_eq!(claude_spawn_count(), 2);
    assert_eq!(git_spawn_count(), 0);
}

#[test]
fn git_spawn_counter_increments() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    reset_for_test();
    assert_eq!(git_spawn_count(), 0);
    assert_eq!(record_git_spawn(), 1);
    assert_eq!(git_spawn_count(), 1);
    assert_eq!(claude_spawn_count(), 0);
}
