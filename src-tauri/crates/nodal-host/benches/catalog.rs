//! Baseline for P14 (executor catalog listed from scratch on every call — cache with mtime
//! invalidation): `catalog::catalog` over a synthetic `~/.claude/agents` with 300 agents.
//! Run with: `cargo bench -p nodal-host --features test-support`.

use criterion::{criterion_group, criterion_main, Criterion};

use nodal_host::claude::catalog::catalog;
use nodal_host::testutil::TempDir;

const AGENTS: usize = 300;

fn synthetic_claude_dir() -> TempDir {
    let dir = TempDir::new("bench-catalog");
    let agents_dir = dir.0.join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    for i in 0..AGENTS {
        let body = format!(
            "---\nname: agent_{i}\ndescription: \"Synthetic agent {i} for the catalog bench.\"\ntools: [Read, Grep, Bash]\n---\n\nYou are agent {i}, a made-up executor used only to benchmark the catalog scan.\n"
        );
        std::fs::write(agents_dir.join(format!("agent_{i}.md")), body).unwrap();
    }
    dir
}

fn bench_catalog(c: &mut Criterion) {
    let dir = synthetic_claude_dir();

    let mut group = c.benchmark_group("claude::catalog");
    group.sample_size(20);
    group.bench_function(format!("catalog ({AGENTS} agents)"), |b| {
        b.iter(|| catalog(Some(&dir.0), None));
    });
    group.finish();
}

criterion_group!(benches, bench_catalog);
criterion_main!(benches);
