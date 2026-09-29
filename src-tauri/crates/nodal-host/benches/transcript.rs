//! P03 (`count_new_tool_calls`, incremental cursor) and P12 (`read_session_close`, one read
//! instead of 3): a synthetic ~14 MB session transcript, read through the same entry points
//! the app uses. Run with: `cargo bench -p nodal-host --features test-support`.

use criterion::{criterion_group, criterion_main, Criterion};

use nodal_host::claude::fs::paths::project_slug;
use nodal_host::claude::fs::readout::read_session_close;
use nodal_host::claude::fs::transcript::{count_new_tool_calls, read_session_transcript};
use nodal_host::testutil::TempDir;

const CWD: &str = "/repo";
const SESSION_ID: &str = "ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8";

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

/// P12 after: run closing reads the `.jsonl` once (before: 3 separate opens, see the P12
/// commit for the measured before/after).
fn bench_session_close(c: &mut Criterion) {
    let dir = TempDir::new("bench-session-close");
    let claude_dir = dir.0.join("claude");
    let slug_dir = claude_dir.join("projects").join(project_slug(CWD));
    std::fs::create_dir_all(&slug_dir).unwrap();
    let text = synthetic_transcript();
    std::fs::write(slug_dir.join(format!("{SESSION_ID}.jsonl")), &text).unwrap();
    // SAFETY: this binary only runs criterion benchmarks sequentially (no other threads read
    // or write this env var while `benches` runs).
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", &claude_dir) };

    let mut group = c.benchmark_group("claude::fs::readout");
    group.sample_size(10);
    group.bench_function(format!("read_session_close ({} MB)", text.len() / (1024 * 1024)), |b| {
        b.iter(|| read_session_close(SESSION_ID, CWD));
    });
    group.finish();
    unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") };
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

criterion_group!(benches, bench_transcript, bench_session_close, bench_tool_call_progress);
criterion_main!(benches);
