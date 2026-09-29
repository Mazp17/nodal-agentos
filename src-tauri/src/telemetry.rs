//! Process-wide `tracing` setup (ADR-007): an env-filtered stderr subscriber with per-span
//! close timing, plus [`command_latency_snapshot`] to read the same timings back in-process
//! (for a debug log line, or a future debug command) without re-parsing stderr.
//!
//! `RUST_LOG` controls verbosity (default `info`); every `#[tracing::instrument]`d IPC
//! command emits an `info`-level span, so the default already carries per-command timing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tracing::span::{Attributes, Id};
use tracing::Subscriber;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// How many recent samples a span name keeps for the percentile snapshot. Bounded so a
/// long-lived process doesn't grow this unboundedly; recent latency is what matters for
/// live profiling, not a lifetime histogram.
const MAX_SAMPLES_PER_SPAN: usize = 512;

static LATENCY: OnceLock<LatencyLayer> = OnceLock::new();

/// Installs the process-wide subscriber. Call once, before anything else logs (first thing
/// in `run()`).
pub fn init() {
    let latency = LatencyLayer::default();
    let _ = LATENCY.set(latency.clone());
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // No ANSI: stderr usually ends up in a redirected log file (`RUST_LOG=info nodal 2>
    // out.log`, `scripts/perf-measure.sh -l`), where escape codes just get in the way of
    // `grep`.
    let fmt_layer = tracing_subscriber::fmt::layer().with_target(true).with_ansi(false).with_span_events(FmtSpan::CLOSE);
    let _ = tracing_subscriber::registry().with(filter).with(fmt_layer).with(latency).try_init();
}

struct Timing {
    busy: Duration,
    entered_at: Option<Instant>,
}

/// Per-span-name latency, aggregated in-process from span enter/exit pairs (the same "busy
/// time" `tracing_subscriber::fmt`'s `FmtSpan::CLOSE` prints, computed independently so it
/// can be queried without parsing logs).
#[derive(Clone, Default)]
struct LatencyLayer {
    samples: std::sync::Arc<Mutex<HashMap<String, Vec<u64>>>>,
}

impl<S> Layer<S> for LatencyLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, _attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(Timing { busy: Duration::ZERO, entered_at: None });
        }
    }

    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            if let Some(timing) = span.extensions_mut().get_mut::<Timing>() {
                timing.entered_at = Some(Instant::now());
            }
        }
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            if let Some(timing) = span.extensions_mut().get_mut::<Timing>() {
                if let Some(start) = timing.entered_at.take() {
                    timing.busy += start.elapsed();
                }
            }
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let busy_micros = {
            let mut ext = span.extensions_mut();
            let Some(timing) = ext.remove::<Timing>() else { return };
            let mut busy = timing.busy;
            if let Some(start) = timing.entered_at {
                busy += start.elapsed();
            }
            busy.as_micros() as u64
        };
        let key = format!("{}::{}", span.metadata().target(), span.name());
        let mut samples = self.samples.lock().unwrap_or_else(|p| p.into_inner());
        let entry = samples.entry(key).or_default();
        entry.push(busy_micros);
        if entry.len() > MAX_SAMPLES_PER_SPAN {
            entry.remove(0);
        }
    }
}

/// p50/p95/max (microseconds) and sample count for one span name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyStats {
    pub count: usize,
    pub p50_us: u64,
    pub p95_us: u64,
    pub max_us: u64,
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn stats_of(mut samples: Vec<u64>) -> LatencyStats {
    samples.sort_unstable();
    LatencyStats {
        count: samples.len(),
        p50_us: percentile(&samples, 0.50),
        p95_us: percentile(&samples, 0.95),
        max_us: samples.last().copied().unwrap_or(0),
    }
}

impl LatencyLayer {
    fn snapshot(&self) -> Vec<(String, LatencyStats)> {
        let samples = self.samples.lock().unwrap_or_else(|p| p.into_inner());
        samples.iter().map(|(name, v)| (name.clone(), stats_of(v.clone()))).collect()
    }
}

/// p50/p95 per instrumented span (IPC commands, `git::run`, `claude_bin::output_with_timeout`),
/// computed from the samples recorded since the process started (or since the ring buffer
/// wrapped: see `MAX_SAMPLES_PER_SPAN`). Empty before [`init`] runs.
pub fn command_latency_snapshot() -> Vec<(String, LatencyStats)> {
    LATENCY.get().map(LatencyLayer::snapshot).unwrap_or_default()
}

/// Structured debug log of the current latency snapshot plus the spawn/IPC-event counters —
/// the "exposed by a debug command or structured log" half of the Ola 0 deliverable, without
/// adding a new IPC command (`check-commands.sh` would fail a debug-only command with no
/// frontend `invoke`).
pub fn log_metrics_snapshot() {
    for (span, stats) in command_latency_snapshot() {
        tracing::info!(
            target: "nodal::latency",
            span = %span,
            count = stats.count,
            p50_us = stats.p50_us,
            p95_us = stats.p95_us,
            max_us = stats.max_us,
            "span latency snapshot"
        );
    }
    tracing::info!(
        target: "nodal::spawn",
        claude_spawns = nodal_host::metrics::claude_spawn_count(),
        git_spawns = nodal_host::metrics::git_spawn_count(),
        "spawn counters snapshot"
    );
    tracing::info!(
        target: "nodal::ipc",
        notify_calls = ipc_notify_count(),
        chat_emit_calls = chat_emit_count(),
        "IPC event counters snapshot"
    );
}

// ---------- IPC event counters (Events::notify, TauriChatSink::emit) ----------

static IPC_NOTIFY_CALLS: AtomicU64 = AtomicU64::new(0);
static CHAT_EMIT_CALLS: AtomicU64 = AtomicU64::new(0);

/// Call once per `Events::notify` (before its 150ms debounce): distinct from the actual
/// `app.emit` calls it coalesces into.
pub fn record_ipc_notify() -> u64 {
    let total = IPC_NOTIFY_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::debug!(target: "nodal::ipc", kind = "notify", total, "IPC notify");
    total
}

/// Call once per `TauriChatSink::emit` (`nodal://chat`).
pub fn record_chat_emit() -> u64 {
    let total = CHAT_EMIT_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::debug!(target: "nodal::ipc", kind = "chat_emit", total, "IPC chat emit");
    total
}

pub fn ipc_notify_count() -> u64 {
    IPC_NOTIFY_CALLS.load(Ordering::Relaxed)
}

pub fn chat_emit_count() -> u64 {
    CHAT_EMIT_CALLS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests;
