//! Baseline for P05 (stream-json parsing): a synthetic turn's worth of lines, dominated by the
//! `stream_event`/`text_delta` lines a chat streams while the model is writing (the hot path
//! `parse_line` runs on every line of stdout). Run with:
//! `cargo bench -p nodal-host --features test-support --bench stream_json`.

use criterion::{criterion_group, criterion_main, Criterion};

use nodal_host::claude::stream_json::parse_line;

/// One `stream_event`/`text_delta` line, a handful of words long (close to what a model emits
/// per token/chunk).
fn delta_line(i: usize) -> String {
    serde_json::json!({
        "type": "stream_event",
        "event": {"delta": {"type": "text_delta", "text": format!(" word{i}")}},
        "parent_tool_use_id": null,
    })
    .to_string()
}

fn assistant_line(i: usize) -> String {
    serde_json::json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "text", "text": format!("Looking at file {i}.")},
                {"type": "tool_use", "id": format!("tool_{i}"), "name": "Read", "input": {"file_path": format!("/repo/src/module_{i}.rs")}},
            ],
            "usage": {"input_tokens": 1200, "cache_read_input_tokens": 800},
        },
        "parent_tool_use_id": null,
    })
    .to_string()
}

fn user_line(i: usize) -> String {
    serde_json::json!({
        "type": "user",
        "message": {
            "content": [
                {"type": "tool_result", "tool_use_id": format!("tool_{i}"), "content": "fn f() {}\n"},
            ],
        },
        "parent_tool_use_id": null,
    })
    .to_string()
}

/// ~200 delta lines (a short streamed answer) plus a handful of the structural lines around a
/// tool call, close to one real chat turn's stdout.
fn synthetic_turn() -> Vec<String> {
    let mut lines = vec![serde_json::json!({
        "type": "system", "subtype": "init", "session_id": "00000000-0000-4000-8000-000000000001",
        "model": "claude-opus-4", "cwd": "/repo", "permissionMode": "default",
    })
    .to_string()];
    for i in 0..200 {
        lines.push(delta_line(i));
    }
    lines.push(assistant_line(0));
    lines.push(user_line(0));
    for i in 200..260 {
        lines.push(delta_line(i));
    }
    lines.push(serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "session_id": "00000000-0000-4000-8000-000000000001", "total_cost_usd": 0.02}).to_string());
    lines
}

fn bench_parse_line(c: &mut Criterion) {
    let lines = synthetic_turn();
    let mut group = c.benchmark_group("claude::stream_json");
    group.bench_function(format!("parse_line (one turn, {} lines)", lines.len()), |b| {
        b.iter(|| {
            for l in &lines {
                std::hint::black_box(parse_line(l));
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_parse_line);
criterion_main!(benches);
