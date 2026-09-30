use crate::testutil::{block_on, fake_claude_counting as fake_claude, spawn_count, TempDir};

use super::AgentsRaw;

#[test]
fn reads_within_the_ttl_spawn_claude_once() {
    let t = TempDir::new("agents-raw-single-flight");
    let count_file = t.0.join("count");
    let cache = AgentsRaw::with_program(fake_claude(&t.0, &count_file));
    block_on(async {
        for _ in 0..5 {
            cache.get().await.expect("get");
        }
    });
    assert_eq!(spawn_count(&count_file), 1, "5 reads inside the TTL window should spawn `claude` once");
}

#[test]
fn invalidate_forces_the_next_read_to_spawn_again() {
    let t = TempDir::new("agents-raw-invalidate");
    let count_file = t.0.join("count");
    let cache = AgentsRaw::with_program(fake_claude(&t.0, &count_file));
    block_on(async {
        cache.get().await.expect("get");
        cache.invalidate().await;
        cache.get().await.expect("get");
    });
    assert_eq!(spawn_count(&count_file), 2, "invalidate() should force a second spawn");
}

/// The actual P01 fix: `cli::list_runs` and `activity::list_agents` used to each spawn their
/// own `claude agents`; sharing one `AgentsRaw` collapses that to a single spawn.
#[test]
fn list_runs_and_list_agents_share_the_cache() {
    let t = TempDir::new("agents-raw-shared-by-cli-and-activity");
    let count_file = t.0.join("count");
    let cache = AgentsRaw::with_program(fake_claude(&t.0, &count_file));
    block_on(async {
        crate::claude::cli::list_runs(&cache).await.expect("list_runs");
        crate::claude::activity::list_agents(&cache).await.expect("list_agents");
    });
    assert_eq!(spawn_count(&count_file), 1, "cli::list_runs and activity::list_agents should share the same spawn");
}
