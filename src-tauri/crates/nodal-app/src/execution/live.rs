//! `LiveSessions`: a `ClaudeCli` decorator that caches `list_sessions`/`list_agent_sessions`
//! (both come from the same `claude agents --json --all`) for a short TTL, single-flight, so
//! the pump, `list_runs`, `work_summary` and `external_sessions` share one read instead of
//! each spawning its own `claude agents` (P01). Forwards `launch_bg`/`stop` to the wrapped
//! `ClaudeCli` unchanged and invalidates both caches when they return, since either can change
//! the live set.
//!
//! The actual process spawn (and its own single-flight raw-stdout cache, shared by
//! `nodal-host`'s `cli::list_runs`/`activity::list_agents`) stays host-side, behind the
//! `ClaudeCli` port: nodal-app can't do I/O (`clippy.toml`) and the wire format is only parsed
//! inside nodal-host. This cache sits one layer up, over the port's typed results, so a caller
//! never even crosses into nodal-host when another one just refreshed them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nodal_domain::error::HostError;
use nodal_domain::model::activity::AgentSession;
use nodal_domain::model::claude::{ExtraFlags, LaunchError, RunRef, RunSummary};
use nodal_domain::model::LaunchOptions;
use nodal_domain::ports::{BoxFut, ClaudeCli};

/// Must stay `> half` the pump's and the UI's shared 5 s tick: since the two run on
/// independent timers, any relative phase between them must still leave at least one inside
/// the other's cached window. A 2 s TTL left a `pump_offset` band of [2 s, 3 s) where both
/// missed the cache and each spawned its own `claude agents`, failing S1 (see ADR-013). 3 s
/// still leaked under timer jitter at a 2.5 s offset; 4 s leaves 1.5 s of margin.
const TTL: Duration = Duration::from_secs(4);

struct Slot<T> {
    at: Instant,
    value: Vec<T>,
}

pub struct LiveSessions {
    inner: Arc<dyn ClaudeCli>,
    runs: tokio::sync::Mutex<Option<Slot<RunSummary>>>,
    agents: tokio::sync::Mutex<Option<Slot<AgentSession>>>,
}

impl LiveSessions {
    pub fn new(inner: Arc<dyn ClaudeCli>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            runs: tokio::sync::Mutex::new(None),
            agents: tokio::sync::Mutex::new(None),
        })
    }

    /// Drops both cached slots: called after `launch_bg`/`stop` so the next read reflects the
    /// change instead of waiting out `TTL`.
    async fn invalidate(&self) {
        *self.runs.lock().await = None;
        *self.agents.lock().await = None;
    }
}

impl ClaudeCli for LiveSessions {
    fn list_sessions(&self) -> BoxFut<'_, Result<Vec<RunSummary>, HostError>> {
        Box::pin(async move {
            let mut slot = self.runs.lock().await;
            if let Some(s) = slot.as_ref() {
                if s.at.elapsed() < TTL {
                    return Ok(s.value.clone());
                }
            }
            let value = self.inner.list_sessions().await?;
            *slot = Some(Slot { at: Instant::now(), value: value.clone() });
            Ok(value)
        })
    }

    fn list_agent_sessions(&self) -> BoxFut<'_, Result<Vec<AgentSession>, HostError>> {
        Box::pin(async move {
            let mut slot = self.agents.lock().await;
            if let Some(s) = slot.as_ref() {
                if s.at.elapsed() < TTL {
                    return Ok(s.value.clone());
                }
            }
            let value = self.inner.list_agent_sessions().await?;
            *slot = Some(Slot { at: Instant::now(), value: value.clone() });
            Ok(value)
        })
    }

    fn launch_bg<'a>(
        &'a self,
        cwd: String,
        prompt: String,
        opts: &'a LaunchOptions,
        extra: &'a ExtraFlags,
    ) -> BoxFut<'a, Result<RunRef, LaunchError>> {
        Box::pin(async move {
            let r = self.inner.launch_bg(cwd, prompt, opts, extra).await;
            self.invalidate().await;
            r
        })
    }

    fn stop<'a>(&'a self, short_id: &'a str) -> BoxFut<'a, Result<(), HostError>> {
        Box::pin(async move {
            let r = self.inner.stop(short_id).await;
            self.invalidate().await;
            r
        })
    }
}

#[cfg(test)]
mod tests;
