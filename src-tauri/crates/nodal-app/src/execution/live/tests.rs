use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use nodal_domain::error::HostError;
use nodal_domain::model::activity::AgentSession;
use nodal_domain::model::claude::{ExtraFlags, LaunchError, RunRef, RunSummary};
use nodal_domain::model::LaunchOptions;
use nodal_domain::ports::{BoxFut, ClaudeCli};

use crate::testutil::block_on;

use super::LiveSessions;

#[derive(Default)]
struct FakeClaude {
    run_calls: AtomicUsize,
    agent_calls: AtomicUsize,
}

fn summary(id: &str) -> RunSummary {
    RunSummary {
        id: id.into(),
        session_id: id.into(),
        cwd: None,
        name: None,
        started_at: None,
        pid: None,
        status: None,
        state: None,
        waiting_for: None,
    }
}

impl ClaudeCli for FakeClaude {
    fn list_sessions(&self) -> BoxFut<'_, Result<Vec<RunSummary>, HostError>> {
        let n = self.run_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(vec![summary(&n.to_string())]) })
    }

    fn list_agent_sessions(&self) -> BoxFut<'_, Result<Vec<AgentSession>, HostError>> {
        self.agent_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(vec![]) })
    }

    fn launch_bg<'a>(
        &'a self,
        cwd: String,
        _prompt: String,
        _opts: &'a LaunchOptions,
        _extra: &'a ExtraFlags,
    ) -> BoxFut<'a, Result<RunRef, LaunchError>> {
        Box::pin(async move { Ok(RunRef { id: "bg1".into(), cwd }) })
    }

    fn stop<'a>(&'a self, _short_id: &'a str) -> BoxFut<'a, Result<(), HostError>> {
        Box::pin(async move { Ok(()) })
    }
}

#[test]
fn list_sessions_is_single_flight_within_the_ttl() {
    let fake = Arc::new(FakeClaude::default());
    let live = LiveSessions::new(fake.clone());
    block_on(async {
        for _ in 0..5 {
            live.list_sessions().await.unwrap();
        }
    });
    assert_eq!(fake.run_calls.load(Ordering::SeqCst), 1, "5 reads inside the TTL window should call the inner port once");
}

#[test]
fn list_agent_sessions_is_single_flight_within_the_ttl() {
    let fake = Arc::new(FakeClaude::default());
    let live = LiveSessions::new(fake.clone());
    block_on(async {
        for _ in 0..5 {
            live.list_agent_sessions().await.unwrap();
        }
    });
    assert_eq!(fake.agent_calls.load(Ordering::SeqCst), 1, "5 reads inside the TTL window should call the inner port once");
}

#[test]
fn the_two_typed_caches_are_independent() {
    let fake = Arc::new(FakeClaude::default());
    let live = LiveSessions::new(fake.clone());
    block_on(async {
        live.list_sessions().await.unwrap();
        live.list_agent_sessions().await.unwrap();
    });
    assert_eq!(fake.run_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fake.agent_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn launch_bg_invalidates_both_caches() {
    let fake = Arc::new(FakeClaude::default());
    let live = LiveSessions::new(fake.clone());
    block_on(async {
        live.list_sessions().await.unwrap();
        live.list_agent_sessions().await.unwrap();
        live.launch_bg("/x".into(), "hi".into(), &LaunchOptions::default(), &ExtraFlags::default()).await.unwrap();
        live.list_sessions().await.unwrap();
        live.list_agent_sessions().await.unwrap();
    });
    assert_eq!(fake.run_calls.load(Ordering::SeqCst), 2, "launch_bg should force a fresh list_sessions");
    assert_eq!(fake.agent_calls.load(Ordering::SeqCst), 2, "launch_bg should force a fresh list_agent_sessions");
}

#[test]
fn stop_invalidates_both_caches() {
    let fake = Arc::new(FakeClaude::default());
    let live = LiveSessions::new(fake.clone());
    block_on(async {
        live.list_sessions().await.unwrap();
        live.stop("bg1").await.unwrap();
        live.list_sessions().await.unwrap();
    });
    assert_eq!(fake.run_calls.load(Ordering::SeqCst), 2, "stop should force a fresh list_sessions");
}
