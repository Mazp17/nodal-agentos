use super::*;
use crate::db::open_in_memory;
use crate::db::rows::{insert_project, insert_repo, insert_task};
use crate::domain::*;
use crate::work::testutil::{project_of, repo_of, run_of, task_of};

fn seed(c: &rusqlite::Connection) {
    insert_project(c, &project_of("p1", "PAY")).unwrap();
    insert_repo(c, &repo_of("r1", "p1", "/r1")).unwrap();
    insert_task(c, &task_of("t1")).unwrap();
}

fn queued(id: &str, pos: f64) -> Run {
    let mut r = run_of(Executor::Claude, RunKind::Work, false);
    r.id = id.into();
    r.status = RunStatus::Queued;
    r.queue_position = pos;
    r.queued_at = pos as i64;
    r.claude_run_id = None;
    r.session_id = None;
    r
}

#[test]
fn queue_order_reorder_and_pending() {
    let db = open_in_memory().unwrap();
    let mut c = db.lock().unwrap();
    seed(&c);
    assert_eq!(runs::next_queue_position(&c).unwrap(), 1.0);
    for (id, pos) in [("a", 1.0), ("b", 2.0), ("c", 3.0)] {
        runs::insert(&c, &queued(id, pos)).unwrap();
    }
    let mut done = queued("d", 0.5);
    done.status = RunStatus::Finished;
    done.finished_at = Some(100);
    runs::insert(&c, &done).unwrap();
    let ids = |v: Vec<Run>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(ids(runs::queue(&c).unwrap()), ["a", "b", "c"]);
    assert_eq!(ids(runs::pending(&c).unwrap()), ["a", "b", "c"]);
    assert_eq!(runs::pending_in_project(&c, "p1").unwrap(), 3);

    runs::reorder_queue(&mut c, &["c".into(), "a".into(), "b".into()]).unwrap();
    assert_eq!(ids(runs::queue(&c).unwrap()), ["c", "a", "b"]);
    // Tiene que ser exactamente el conjunto en cola.
    assert!(runs::reorder_queue(&mut c, &["c".into(), "a".into()]).is_err());
    assert!(runs::reorder_queue(&mut c, &["c".into(), "a".into(), "a".into()]).is_err());
    assert!(runs::reorder_queue(&mut c, &["c".into(), "a".into(), "d".into()]).is_err());

    // Compare-and-set.
    assert!(runs::transition(&c, "a", RunStatus::Queued, RunStatus::Launching).unwrap());
    assert!(!runs::transition(&c, "a", RunStatus::Queued, RunStatus::Launching).unwrap());
    assert_eq!(runs::fail_interrupted_launches(&c, 5).unwrap(), 1);
    let a = runs::get(&c, "a").unwrap();
    assert_eq!(a.status, RunStatus::Failed);
    assert!(a.error.unwrap().contains("closed while"));

    assert_eq!(runs::last_finished(&c, "t1").unwrap().unwrap().id, "d");
    assert_eq!(runs::list(&c, Some("t1")).unwrap().len(), 4);
    assert!(runs::launched_refs(&c).unwrap().is_empty());
    let mut b = runs::get(&c, "b").unwrap();
    b.claude_run_id = Some("abcd1234".into());
    runs::update(&c, &b).unwrap();
    assert_eq!(runs::launched_refs(&c).unwrap(), vec![(Some("abcd1234".into()), None)]);

    // Borrar la tarea deja sus runs con task_id NULL.
    tasks::delete(&c, "t1").unwrap();
    assert_eq!(runs::get(&c, "b").unwrap().task_id, None);
}

#[test]
fn task_number_counter_and_status() {
    let db = open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    assert_eq!(projects::take_task_number(&c, "p1").unwrap(), 1);
    assert_eq!(projects::take_task_number(&c, "p1").unwrap(), 2);
    assert!(projects::take_task_number(&c, "nope").is_err());
    tasks::set_status(&c, "t1", TaskStatus::Done, 50).unwrap();
    assert_eq!(tasks::get(&c, "t1").unwrap().closed_at, Some(50));
    tasks::set_status(&c, "t1", TaskStatus::Canceled, 60).unwrap();
    assert_eq!(tasks::get(&c, "t1").unwrap().closed_at, Some(50), "conserva el primer cierre");
    tasks::set_status(&c, "t1", TaskStatus::Todo, 70).unwrap();
    assert_eq!(tasks::get(&c, "t1").unwrap().closed_at, None);
    assert!(repos::find_by_path(&c, "/r1").unwrap().is_some());
    assert_eq!(repos::next_position(&c, "p1").unwrap(), 1);
}
