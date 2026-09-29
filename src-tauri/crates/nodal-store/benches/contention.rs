//! ADR-002/P10 re-measurement (Ola 2, after Ola 1's changes): a file-backed `Db` (WAL), 8
//! concurrent readers hammering `Db::read` plus one pump/sync-shaped `Db::write` transaction,
//! 10k synthetic runs. Prints p50/p95/max for both; not a pass/fail test, a documented
//! measurement (see ADR-002 in the vault for the decision this feeds — this bench doubles as
//! the before/after for P10 itself: run it, `git stash` `db.rs`'s reader pool, run it again).
//!
//! `cargo bench -p nodal-store --features test-support --bench contention`

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nodal_domain::model::{Executor, RunKind, RunStatus};
use nodal_domain::testutil::{project_of, repo_of, run_of, task_of};
use nodal_store::execution::runs;
use nodal_store::rows::{insert_project, insert_repo, insert_task};
use nodal_store::{Conn, Db, StoreError};

const TASKS: usize = 200;
const RUNS: usize = 10_000;
const READERS: usize = 8;
const LOAD_DURATION: Duration = Duration::from_secs(3);

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
        r.status = if i == 0 { RunStatus::Launched } else { RunStatus::Finished };
        if r.status == RunStatus::Finished {
            r.finished_at = Some(i as i64 + 1);
        }
        runs::insert(conn, &r).unwrap();
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn summarize(name: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    let max = samples.last().copied().unwrap_or_default();
    println!("{name}: n={} p50={p50:?} p95={p95:?} max={max:?}", samples.len());
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(READERS + 2)
        .enable_time()
        .build()
        .unwrap();

    let path = std::env::temp_dir().join(format!("nodal-contention-bench-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    rt.block_on(async {
        let db = Db::open(&path).unwrap();
        {
            let guard = db.guard();
            seed(&guard);
        }

        let stop_at = Instant::now() + LOAD_DURATION;
        let read_samples: Arc<Mutex<Vec<Duration>>> = Arc::default();
        let write_samples: Arc<Mutex<Vec<Duration>>> = Arc::default();

        let mut tasks = Vec::new();
        for _ in 0..READERS {
            let db = db.clone();
            let read_samples = read_samples.clone();
            tasks.push(tokio::spawn(async move {
                while Instant::now() < stop_at {
                    let start = Instant::now();
                    db.read(|c| runs::list_filtered_light(c, Some("p1"), None).map(|_| ()))
                        .await
                        .map_err(|e: StoreError| e)
                        .unwrap();
                    read_samples.lock().unwrap().push(start.elapsed());
                }
            }));
        }

        // One writer, like a pump/sync pass: flips a run's status and clears it back,
        // in the same transaction shape `apply_end`/`sync_run` use (read + write, committed).
        {
            let db = db.clone();
            let write_samples = write_samples.clone();
            tasks.push(tokio::spawn(async move {
                let mut launched = true;
                while Instant::now() < stop_at {
                    let start = Instant::now();
                    db.write(move |tx| {
                        let (from, to) =
                            if launched { (RunStatus::Launched, RunStatus::Finished) } else { (RunStatus::Finished, RunStatus::Launched) };
                        runs::transition(tx, "run0", from, to)?;
                        Ok::<_, StoreError>(())
                    })
                    .await
                    .unwrap();
                    launched = !launched;
                    write_samples.lock().unwrap().push(start.elapsed());
                    // A real pump/sync tick isn't a tight loop; give readers a fair shot too.
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }));
        }

        for t in tasks {
            t.await.unwrap();
        }

        summarize(&format!("read (list_filtered_light, {READERS} concurrent readers)"), read_samples.lock().unwrap().clone());
        summarize("write (pump/sync-shaped transition tx)", write_samples.lock().unwrap().clone());
    });

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}
