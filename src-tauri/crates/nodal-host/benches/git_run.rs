//! Baseline for P06 (`git::run`'s per-call 2 threads + 15ms poll): 1000 sequential
//! invocations of the chokepoint every git operation in the app goes through. Run with:
//! `cargo bench -p nodal-host --features test-support`.

use criterion::{criterion_group, criterion_main, Criterion};

use nodal_host::git;
use nodal_host::testutil::{init_repo, TempDir};

const CALLS: usize = 1000;

fn bench_git_run(c: &mut Criterion) {
    let dir = TempDir::new("bench-git-run");
    init_repo(&dir.0);

    let mut group = c.benchmark_group("git::run");
    group.sample_size(10);
    group.bench_function(format!("run x{CALLS} (rev-parse HEAD)"), |b| {
        b.iter(|| {
            for _ in 0..CALLS {
                git::run(&dir.0, &["rev-parse", "HEAD"]).unwrap();
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_git_run);
criterion_main!(benches);
