use std::thread::sleep;

use tracing_subscriber::layer::SubscriberExt;

use super::*;

#[test]
fn latency_layer_computes_percentiles_from_span_close_events() {
    let layer = LatencyLayer::default();
    let subscriber = tracing_subscriber::registry().with(layer.clone());
    let durations_ms = [1u64, 2, 3, 4, 5, 6, 7, 8, 9, 20];
    tracing::subscriber::with_default(subscriber, || {
        for &ms in &durations_ms {
            let span = tracing::info_span!("bench_span_for_percentile_test");
            span.in_scope(|| sleep(Duration::from_millis(ms)));
        }
    });

    let snapshot = layer.snapshot();
    let (_, stats) = snapshot
        .into_iter()
        .find(|(name, _)| name.ends_with("bench_span_for_percentile_test"))
        .expect("span recorded in the snapshot");

    assert_eq!(stats.count, durations_ms.len());
    assert!(stats.p50_us <= stats.p95_us);
    assert!(stats.p95_us <= stats.max_us);
    // Sleep guarantees at least the requested duration: the 20ms sample dominates the tail.
    assert!(stats.max_us >= 19_000, "max_us = {}", stats.max_us);
    assert!(stats.p95_us >= 8_000, "p95_us = {}", stats.p95_us);
}

#[test]
fn percentile_helper_matches_known_values() {
    let sorted = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    assert_eq!(percentile(&sorted, 0.50), 60);
    assert_eq!(percentile(&sorted, 0.95), 100);
    assert_eq!(percentile(&[], 0.50), 0);
}

#[test]
fn samples_are_bounded_per_span_name() {
    let layer = LatencyLayer::default();
    let subscriber = tracing_subscriber::registry().with(layer.clone());
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..(MAX_SAMPLES_PER_SPAN + 50) {
            tracing::info_span!("bounded_span").in_scope(|| {});
        }
    });
    let snapshot = layer.snapshot();
    let (_, stats) = snapshot.into_iter().find(|(name, _)| name.ends_with("bounded_span")).expect("span recorded");
    assert_eq!(stats.count, MAX_SAMPLES_PER_SPAN);
}

#[test]
fn ipc_counters_are_monotonic_and_independent_of_each_other() {
    let n1 = record_ipc_notify();
    let n2 = record_ipc_notify();
    assert!(n2 > n1);

    let e1 = record_chat_emit();
    let e2 = record_chat_emit();
    assert!(e2 > e1);
}
