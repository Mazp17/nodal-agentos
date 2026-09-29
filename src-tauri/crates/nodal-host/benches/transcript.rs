//! P03 (`count_new_tool_calls`, incremental cursor): a synthetic ~14 MB session transcript,
//! read through the same entry points the app uses. Run with:
//! `cargo bench -p nodal-host --features test-support`.

use criterion::{criterion_group, criterion_main, Criterion};

use nodal_host::claude::fs::transcript::{count_new_tool_calls, read_session_transcript};
use nodal_host::testutil::TempDir;

const TARGET_BYTES: usize = 14 * 1024 * 1024;

/// One assistant turn (a text block plus a tool call) and one user turn (a tool result),
/// close to what a real `<session>.jsonl` line looks like.
fn turn(i: usize) -> String {
    let assistant = serde_json::json!({
        "type": "assistant",
        "message": {
            "content": [
                {"type": "text", "text": format!("Looking at file {i}, this needs a small fix.")},
                {
                    "type": "tool_use",
                    "id": format!("tool_{i}"),
                    "name": "Read",
                    "input": {"file_path": format!("/repo/src/module_{i}.rs")}
                }
            ]
        }
    });
    let user = serde_json::json!({
        "type": "user",
        "message": {
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": format!("tool_{i}"),
                    "content": format!("fn module_{i}() {{\n    // {}\n}}\n", "x".repeat(120))
                }
            ]
        }
    });
    format!("{}\n{}\n", assistant, user)
}

fn synthetic_transcript() -> String {
    let mut out = String::with_capacity(TARGET_BYTES + 4096);
    let mut i = 0;
    while out.len() < TARGET_BYTES {
        out.push_str(&turn(i));
        i += 1;
    }
    out
}

fn bench_transcript(c: &mut Criterion) {
    let dir = TempDir::new("bench-transcript");
    let path = dir.0.join("session.jsonl");
    let text = synthetic_transcript();
    std::fs::write(&path, &text).unwrap();

    let mut group = c.benchmark_group("claude::fs::transcript");
    group.sample_size(10);
    group.bench_function(format!("read_session_transcript ({} MB)", text.len() / (1024 * 1024)), |b| {
        b.iter(|| {
            read_session_transcript(&path, "session1", Some("Claude".to_string()), None, usize::MAX).unwrap()
        });
    });
    group.finish();
}

/// P03: an active run's toolCalls, read with an incremental cursor instead of re-parsing the
/// whole transcript on every poll. `full` = first poll (nothing cached yet); `steady` = a
/// later poll with no new data appended, the common case while the app waits between turns.
fn bench_tool_call_progress(c: &mut Criterion) {
    let dir = TempDir::new("bench-tool-call-progress");
    let path = dir.0.join("session.jsonl");
    let text = synthetic_transcript();
    std::fs::write(&path, &text).unwrap();

    let mut group = c.benchmark_group("claude::fs::transcript::progress");
    group.sample_size(10);
    group.bench_function(format!("count_new_tool_calls, full ({} MB)", text.len() / (1024 * 1024)), |b| {
        b.iter(|| count_new_tool_calls(&path, 0).unwrap());
    });
    let (_, offset) = count_new_tool_calls(&path, 0).unwrap();
    group.bench_function("count_new_tool_calls, steady state (no new data)", |b| {
        b.iter(|| count_new_tool_calls(&path, offset).unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_transcript, bench_tool_call_progress);
criterion_main!(benches);
