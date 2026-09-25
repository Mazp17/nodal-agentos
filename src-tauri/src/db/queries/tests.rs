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
    // It must be exactly the queued set.
    assert!(runs::reorder_queue(&mut c, &["c".into(), "a".into()]).is_err());
    assert!(runs::reorder_queue(&mut c, &["c".into(), "a".into(), "a".into()]).is_err());
    assert!(runs::reorder_queue(&mut c, &["c".into(), "a".into(), "d".into()]).is_err());

    // Compare-and-set.
    assert!(runs::transition(&c, "a", RunStatus::Queued, RunStatus::Launching).unwrap());
    assert!(!runs::transition(&c, "a", RunStatus::Queued, RunStatus::Launching).unwrap());
    assert_eq!(runs::launching(&c).unwrap().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["a"]);
    assert!(runs::transition(&c, "a", RunStatus::Launching, RunStatus::Failed).unwrap());

    assert_eq!(runs::last_finished(&c, "t1").unwrap().unwrap().id, "d");
    assert_eq!(runs::list_filtered(&c, None, Some("t1")).unwrap().len(), 4);
    assert!(runs::launched_refs(&c).unwrap().is_empty());
    let mut b = runs::get(&c, "b").unwrap();
    b.claude_run_id = Some("abcd1234".into());
    runs::update(&c, &b).unwrap();
    assert_eq!(runs::launched_refs(&c).unwrap(), vec![(Some("abcd1234".into()), None)]);

    // Deleting the task leaves its runs with a NULL task_id.
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
    assert_eq!(tasks::get(&c, "t1").unwrap().closed_at, Some(50), "keeps the first close");
    tasks::set_status(&c, "t1", TaskStatus::Todo, 70).unwrap();
    assert_eq!(tasks::get(&c, "t1").unwrap().closed_at, None);
    assert!(repos::find_by_path(&c, "/r1").unwrap().is_some());
    assert_eq!(repos::next_position(&c, "p1").unwrap(), 1);
}

#[test]
fn runs_filtered_by_project_and_latest_by_task() {
    let db = open_in_memory().unwrap();
    let c = db.lock().unwrap();
    seed(&c);
    insert_project(&c, &project_of("p2", "WEB")).unwrap();
    insert_repo(&c, &repo_of("r2", "p2", "/r2")).unwrap();
    let mut t2 = task_of("t2");
    t2.number = 2;
    insert_task(&c, &t2).unwrap();
    let mut t3 = task_of("t3");
    t3.project_id = "p2".into();
    t3.repo_id = "r2".into();
    insert_task(&c, &t3).unwrap();
    let mk = |id: &str, task: Option<&str>, repo: &str, at: i64| {
        let mut r = queued(id, at as f64);
        r.task_id = task.map(Into::into);
        r.repo_id = Some(repo.into());
        r.status = RunStatus::Finished;
        r
    };
    for r in [
        mk("a", Some("t1"), "r1", 1),
        mk("b", Some("t1"), "r1", 3),
        mk("c", Some("t2"), "r1", 2),
        mk("d", Some("t3"), "r2", 4),
        mk("e", None, "r2", 5),
    ] {
        runs::insert(&c, &r).unwrap();
    }
    let ids = |v: Vec<Run>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(ids(runs::list_filtered(&c, None, None).unwrap()), ["e", "d", "b", "c", "a"]);
    assert_eq!(ids(runs::list_filtered(&c, Some("p1"), None).unwrap()), ["b", "c", "a"]);
    // A run without a task counts toward its repo's project.
    assert_eq!(ids(runs::list_filtered(&c, Some("p2"), None).unwrap()), ["e", "d"]);
    assert_eq!(ids(runs::list_filtered(&c, Some("p1"), Some("t2")).unwrap()), ["c"]);
    assert!(runs::list_filtered(&c, Some("p2"), Some("t1")).unwrap().is_empty());

    assert_eq!(ids(runs::latest_by_task(&c, None).unwrap()), ["d", "b", "c"]);
    assert_eq!(ids(runs::latest_by_task(&c, Some("p1")).unwrap()), ["b", "c"]);

    let light = serde_json::to_value(RunLight::from(runs::get(&c, "b").unwrap())).unwrap();
    assert!(light.get("prompt").is_none() && light.get("extraInstructions").is_none());
    assert_eq!(light["taskId"], "t1");
}
