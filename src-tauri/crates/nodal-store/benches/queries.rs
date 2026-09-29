//! Baseline for P09 (explicit columns, no `?1 IS NULL OR`, `prepare_cached`): 10k synthetic
//! runs across 200 tasks in one project, in-memory DB. Run with:
//! `cargo bench -p nodal-store --features test-support`.

use criterion::{criterion_group, criterion_main, Criterion};

use nodal_domain::model::{Executor, RunKind, RunStatus};
use nodal_domain::testutil::{project_of, repo_of, run_of, task_of};
use nodal_store::execution::runs;
use nodal_store::rows::{insert_project, insert_repo, insert_task};
use nodal_store::{Conn, Db};

const TASKS: usize = 200;
const RUNS: usize = 10_000;

fn seed(conn: &Conn) {
    insert_project(conn, &project_of("p1", "ACME")).unwrap();
    insert_repo(conn, &repo_of("r1", "p1", "/repo")).unwrap();
    for i in 0..TASKS {
        let mut t = task_of(&format!("t{i}"));
        t.number = i as i64 + 1;
        insert_task(conn, &t).unwrap();
    }
    for i in 0..RUNS {
        let mut r = run_of(Executor::Claude, RunKind::Work, false);
        r.id = format!("run{i}");
        r.task_id = Some(format!("t{}", i % TASKS));
        r.repo_id = Some("r1".into());
        r.queued_at = i as i64;
        r.claude_run_id = None;
        r.session_id = None;
        r.status = match i % 3 {
            0 => RunStatus::Queued,
            1 => RunStatus::Launched,
            _ => RunStatus::Finished,
        };
        if r.status == RunStatus::Finished {
            r.finished_at = Some(i as i64 + 1);
        }
        runs::insert(conn, &r).unwrap();
    }
}

fn bench_queries(c: &mut Criterion) {
    let db = Db::open_in_memory().unwrap();
    {
        let guard = db.guard();
        seed(&guard);
    }
    let guard = db.guard();
    let conn: &Conn = &guard;

    c.bench_function("runs::list_filtered(project, 10k runs)", |b| {
        b.iter(|| runs::list_filtered(conn, Some("p1"), None).unwrap());
    });
    c.bench_function("runs::latest_by_task(project, 10k runs)", |b| {
        b.iter(|| runs::latest_by_task(conn, Some("p1")).unwrap());
    });
    c.bench_function("runs::pending_of(project, 10k runs)", |b| {
        b.iter(|| runs::pending_of(conn, Some("p1")).unwrap());
    });
    c.bench_function("runs::queue(10k runs)", |b| {
        b.iter(|| runs::queue(conn).unwrap());
    });
}

criterion_group!(benches, bench_queries);
criterion_main!(benches);
