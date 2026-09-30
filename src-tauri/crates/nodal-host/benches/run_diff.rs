//! Baseline for P02 (a `git diff --no-index` process per untracked file): `git::diff::collect`
//! with 10/50/200 new (untracked) files. Run with:
//! `cargo bench -p nodal-host --features test-support`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use nodal_host::git::diff::collect;
use nodal_host::testutil::{init_repo, TempDir};

const COUNTS: &[usize] = &[10, 50, 200];

/// A repo with `n` new, untracked files of a few lines each (nothing staged/committed
/// besides the initial commit `init_repo` makes).
fn repo_with_untracked(n: usize) -> TempDir {
    let dir = TempDir::new(&format!("bench-run-diff-{n}"));
    init_repo(&dir.0);
    for i in 0..n {
        let path = dir.0.join(format!("new_file_{i}.txt"));
        let body: String = (0..20).map(|l| format!("line {l} of file {i}\n")).collect();
        std::fs::write(path, body).unwrap();
    }
    dir
}

fn bench_run_diff(c: &mut Criterion) {
    let mut group = c.benchmark_group("git::diff::collect");
    group.sample_size(10);
    for &n in COUNTS {
        let dir = repo_with_untracked(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| collect(&dir.0, None).unwrap());
        });
    }
    group.finish();
}

criterion_group!(benches, bench_run_diff);
criterion_main!(benches);
