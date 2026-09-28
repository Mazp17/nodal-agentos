use super::*;
use crate::db::open_in_memory;
use crate::domain::*;
use crate::providers::fake::FakeProvider;
use crate::providers::import::tests::{seed, tmp_dir};
use crate::providers::plan::tests::item;
use crate::providers::state_map::tests::team_states;
use crate::providers::{ErrorKind, ProviderError};

fn state(id: &str) -> ExternalState {
    team_states().into_iter().find(|s| s.id == id).unwrap()
}

struct Env {
    db: Db,
    dir: PathBuf,
    fake: FakeProvider,
    providers: Vec<Provider>,
    link: SourceLink,
    memo: std::cell::RefCell<SyncMemo>,
}

impl Drop for Env {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

impl Env {
    fn new() -> Self {
        let db = open_in_memory().unwrap();
        let link = seed(&db.lock().unwrap());
        let fake = FakeProvider::default();
        fake.data().states = team_states();
        Env {
            db,
            dir: tmp_dir(),
            providers: vec![std::sync::Arc::new(fake.clone())],
            fake,
            link,
            memo: Default::default(),
        }
    }

    /// Imports item `n` (in state `state_id`) into the web repo and leaves it in the fake.
    fn import(&self, n: u32, state_id: &str) -> Task {
        let mut it = item(n, Some("Original description."));
        it.state = state(state_id);
        self.fake
            .data()
            .items
            .insert(it.external_id.clone(), it.clone());
        let mut conn = self.db.lock().unwrap();
        let r = import_items(
            &mut conn,
            &self.dir,
            &self.link,
            vec![(it, "r-web".into())],
            1,
        )
        .unwrap();
        r.imported.into_iter().next().unwrap()
    }

    fn set_link(&mut self, f: impl FnOnce(&mut SourceLink)) {
        f(&mut self.link);
        store::save_link(&self.db.lock().unwrap(), &self.link).unwrap();
    }

    fn run(&self, now: i64) -> SyncReport {
        self.run_with(now, false)
    }

    /// Manual sync: no pause and re-reading states.
    fn run_forced(&self, now: i64) -> SyncReport {
        self.run_with(now, true)
    }

    fn run_with(&self, now: i64, force: bool) -> SyncReport {
        let mut memo = self.memo.borrow_mut();
        tauri::async_runtime::block_on(sync_run(
            &self.db,
            &self.dir,
            &self.providers,
            None,
            now,
            &mut memo,
            force,
        ))
    }

    fn task(&self, id: &str) -> Task {
        rows::get_task(&self.db.lock().unwrap(), id)
            .unwrap()
            .unwrap()
    }

    fn outbox(&self) -> Vec<OutboxItem> {
        store::due_outbox(&self.db.lock().unwrap(), "fake", i64::MAX).unwrap()
    }

    fn enqueue_status(&self, task_id: &str, s: TaskStatus, now: i64) {
        assert!(store::enqueue_status(&self.db.lock().unwrap(), task_id, s, now).unwrap());
    }

    fn enqueue_comment(&self, task_id: &str, body: &str, now: i64) {
        assert!(store::enqueue_comment(&self.db.lock().unwrap(), task_id, body, now).unwrap());
    }
}

#[test]
fn push_retries_with_backoff_then_succeeds() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    env.enqueue_status(&t.id, TaskStatus::InReview, 100);
    env.enqueue_comment(&t.id, "closing", 100);
    env.fake.data().fail_writes = Some(ProviderError::new(
        ErrorKind::Transient,
        "Linear is unavailable (HTTP 502)",
    ));

    let r = env.run(100);
    assert_eq!(r.pushed, 0);
    // The comment waits for the failed status change.
    assert_eq!(r.errors, vec!["ENG-1: Linear is unavailable (HTTP 502)"]);
    assert_eq!(env.outbox()[1].attempts, 0);
    let row = &env.outbox()[0];
    assert_eq!((row.attempts, row.next_attempt_at), (1, 100 + 30_000));
    assert_eq!(
        row.last_error.as_deref(),
        Some("Linear is unavailable (HTTP 502)")
    );
    assert!(env
        .task(&t.id)
        .source
        .unwrap()
        .sync_error
        .unwrap()
        .contains("502"));

    // No retry before the backoff expires.
    env.run(100 + 29_999);
    assert_eq!(env.outbox()[0].attempts, 1);
    env.run(100 + 30_000);
    let row = &env.outbox()[0];
    assert_eq!(
        (row.attempts, row.next_attempt_at),
        (2, 100 + 30_000 + 60_000)
    );

    env.fake.data().fail_writes = None;
    let r = env.run(200_000);
    assert_eq!(r.pushed, 2, "{r:?}");
    assert!(env.outbox().is_empty());
    assert_eq!(
        env.fake.data().set_states,
        vec![("uuid-1".to_string(), "s-review".to_string())]
    );
    let src = env.task(&t.id).source.unwrap();
    assert_eq!(src.external_state.unwrap().id, "s-review");
    assert_eq!(src.sync_error, None);
}

#[test]
fn comment_waits_for_earlier_state_in_backoff() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    env.enqueue_status(&t.id, TaskStatus::InReview, 100);
    env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::Transient, "HTTP 502"));
    env.run(100);
    env.fake.data().fail_writes = None;
    // The comment arrives while the status change waits for its backoff: it does not jump
    // ahead.
    env.enqueue_comment(&t.id, "closing", 200);
    let r = env.run(1_000);
    assert_eq!(r.pushed, 0, "{r:?}");
    assert!(env.fake.data().comments.is_empty());
    let r = env.run(100 + 30_000);
    assert_eq!(r.pushed, 2, "{r:?}");
    assert_eq!(env.fake.data().set_states.len(), 1);
    assert_eq!(env.fake.data().comments.len(), 1);
}

#[test]
fn comment_already_posted_is_not_reposted() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    env.enqueue_comment(&t.id, "closing", 1);
    let id = env.outbox()[0].id;
    // An earlier attempt reached Linear but the row was not marked (timeout, crash).
    env.fake
        .data()
        .comments
        .push(("uuid-1".into(), marked_body("closing", id)));
    let r = env.run(10);
    assert_eq!(r.pushed, 1, "{r:?}");
    assert!(env.outbox().is_empty());
    assert_eq!(env.fake.data().comments.len(), 1, "not reposted");
}

#[test]
fn permanent_errors_and_attempt_cap_drop_the_row() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    env.enqueue_comment(&t.id, "hello", 1);
    env.fake.data().fail_writes =
        Some(ProviderError::new(ErrorKind::Permanent, "Entity not found"));
    let r = env.run(10);
    assert!(r.errors[0].contains("not retried"), "{r:?}");
    assert!(env.outbox().is_empty());
    assert!(env
        .task(&t.id)
        .source
        .unwrap()
        .sync_error
        .unwrap()
        .contains("Entity not found"));

    env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::Transient, "timeout"));
    env.enqueue_comment(&t.id, "hello", 20);
    env.db
        .lock()
        .unwrap()
        .execute("UPDATE sync_outbox SET attempts = ?1", [MAX_ATTEMPTS - 1])
        .unwrap();
    env.run(30);
    assert!(env.outbox().is_empty(), "gave up after MAX_ATTEMPTS");
}

#[test]
fn rate_limit_stops_draining_the_provider() {
    let env = Env::new();
    let a = env.import(1, "s-todo");
    let b = env.import(2, "s-todo");
    env.enqueue_comment(&a.id, "one", 1);
    env.enqueue_comment(&b.id, "two", 1);
    env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::RateLimited, "rate limited"));
    let r = env.run(10);
    assert_eq!(r.errors.len(), 1, "{r:?}");
    let rows = env.outbox();
    // The rate limit spends no attempts: it only pushes back the next one.
    assert_eq!((rows[0].attempts, rows[1].attempts), (0, 0));
    assert_eq!(rows[0].next_attempt_at, 10 + BACKOFF_BASE_MS);
}

#[test]
fn rate_limit_pauses_the_provider_between_passes() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    env.fake.data().fail_states = Some(ProviderError::new(ErrorKind::RateLimited, "rate limited"));
    let r = env.run(10);
    assert!(
        r.notices
            .iter()
            .any(|n| n.contains("fake: sync paused until")),
        "{r:?}"
    );
    env.fake.data().fail_states = None;
    env.fake.data().items.get_mut("uuid-1").unwrap().title = "New".into();
    let calls = env.fake.data().states_calls;
    // Paused: the worker leaves the provider alone.
    let r = env.run(20);
    assert_eq!(
        (r.pulled, env.fake.data().states_calls),
        (0, calls),
        "{r:?}"
    );
    assert_eq!(env.task(&t.id).title, t.title);
    let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
    assert!(
        l.last_sync_error.unwrap().contains("rate limited"),
        "the link keeps the reason"
    );
    // Once the pause expires, it syncs again.
    let r = env.run(10 + RATE_LIMIT_PAUSE_MS);
    assert_eq!(r.pulled, 1, "{r:?}");
    assert_eq!(env.task(&t.id).title, "New");
    assert!(env.memo.borrow().paused.is_empty());
}

#[test]
fn manual_sync_ignores_the_pause() {
    let env = Env::new();
    env.import(1, "s-todo");
    env.fake.data().fail_states = Some(ProviderError::new(ErrorKind::Auth, "key revoked"));
    env.run(10);
    assert_eq!(env.memo.borrow().paused["fake"].until, 10 + AUTH_PAUSE_MS);
    env.fake.data().fail_states = None;
    assert_eq!(env.run_forced(20).pulled, 1);
    assert!(env.memo.borrow().paused.is_empty());
}

#[test]
fn saving_the_mapping_invalidates_cached_states() {
    let mut env = Env::new();
    let t = env.import(1, "s-todo");
    env.run(10);
    // New state in the provider, mapped by the user within the TTL.
    let qa = ExternalState {
        id: "s-qa".into(),
        name: "QA".into(),
        kind: ExtKind::Started,
        color: None,
    };
    env.fake.data().states.push(qa);
    env.set_link(|l| {
        l.state_map
            .push
            .insert(TaskStatus::InReview, Some("s-qa".into()));
    });
    env.enqueue_status(&t.id, TaskStatus::InReview, 20);
    let r = env.run(30);
    assert_eq!(r.pushed, 1, "{r:?}");
    assert_eq!(
        env.fake.data().set_states,
        vec![("uuid-1".to_string(), "s-qa".to_string())]
    );
}

#[test]
fn states_are_cached_between_passes() {
    let env = Env::new();
    env.import(1, "s-todo");
    env.run(10);
    env.run(20);
    assert_eq!(env.fake.data().states_calls, 1);
    env.run(10 + STATES_TTL_MS);
    assert_eq!(env.fake.data().states_calls, 2);
    env.run_forced(10 + STATES_TTL_MS + 1);
    assert_eq!(env.fake.data().states_calls, 3);
}

#[test]
fn auth_errors_never_drop_changes() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    env.enqueue_comment(&t.id, "hello", 1);
    env.db
        .lock()
        .unwrap()
        .execute("UPDATE sync_outbox SET attempts = ?1", [MAX_ATTEMPTS - 1])
        .unwrap();
    env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::Auth, "key revoked"));
    let mut now = 10;
    for _ in 0..20 {
        env.run(now);
        now += BACKOFF_MAX_MS;
    }
    let rows = env.outbox();
    assert_eq!(rows.len(), 1, "a revoked key keeps the change queued");
    assert_eq!(rows[0].attempts, MAX_ATTEMPTS - 1);
    env.fake.data().fail_writes = None;
    assert_eq!(env.run(now).pushed, 1);
}

#[test]
fn unlinked_items_are_not_auto_imported_again() {
    let mut env = Env::new();
    env.set_link(|l| {
        l.auto_import = true;
        l.default_repo_id = Some("r-web".into());
    });
    let t = env.import(1, "s-todo");
    store::unlink_task(&env.db.lock().unwrap(), &t.id, 5).unwrap();
    assert_eq!(env.run(10).imported, 0);
    assert!(
        store::task_by_external(&env.db.lock().unwrap(), "fake", "uuid-1")
            .unwrap()
            .is_none()
    );
    // Known settings are not affected by the tombstone.
    assert_eq!(
        rows::load_settings(&env.db.lock().unwrap()).unwrap(),
        Settings::default()
    );
}

#[test]
fn pending_map_skips_state_but_sends_comment() {
    let mut env = Env::new();
    let t = env.import(1, "s-todo");
    env.set_link(|l| l.state_map.confirmed_at = None);
    env.enqueue_status(&t.id, TaskStatus::InReview, 1);
    env.enqueue_comment(&t.id, "**Nodal** · In Review", 1);
    let r = env.run(10);
    assert_eq!(r.pushed, 1);
    assert!(env.outbox().is_empty());
    let d = env.fake.data();
    assert!(d.set_states.is_empty());
    assert_eq!(d.comments.len(), 1);
    assert_eq!(d.comments[0].0, "uuid-1");
    assert!(
        d.comments[0]
            .1
            .starts_with("**Nodal** · In Review\n\n<!-- nodal:outbox:"),
        "{:?}",
        d.comments
    );
}

#[test]
fn no_sync_and_vanished_targets_are_dropped() {
    let mut env = Env::new();
    let t = env.import(1, "s-todo");
    env.set_link(|l| {
        l.state_map.push.insert(TaskStatus::Blocked, None);
    });
    env.enqueue_status(&t.id, TaskStatus::Blocked, 1);
    let r = env.run(10);
    assert_eq!((r.pushed, env.outbox().len()), (0, 0));
    assert!(env.fake.data().set_states.is_empty());

    // "In Review" vanishes from the provider: it is not pushed and a notice is raised.
    env.fake.data().states.retain(|s| s.id != "s-review");
    env.enqueue_status(&t.id, TaskStatus::InReview, 20);
    let r = env.run_forced(30);
    assert!(env.fake.data().set_states.is_empty());
    assert!(
        r.notices
            .iter()
            .any(|n| n.contains("\"In Review\" no longer exists")),
        "{r:?}"
    );
    assert!(env
        .task(&t.id)
        .source
        .unwrap()
        .sync_error
        .unwrap()
        .contains("no longer exists"));
}

#[test]
fn todo_is_pushed_when_mapped_and_silently_skipped_otherwise() {
    let mut env = Env::new();
    let t = env.import(1, "s-progress");
    env.enqueue_status(&t.id, TaskStatus::Todo, 1);
    let r = env.run(10);
    assert_eq!(
        (r.pushed, r.errors.len(), r.notices.len()),
        (1, 0, 0),
        "{r:?}"
    );
    assert_eq!(
        env.fake.data().set_states,
        vec![("uuid-1".to_string(), "s-todo".to_string())]
    );

    // Mapping confirmed before the Todo row existed: not synced, no notice.
    env.set_link(|l| {
        l.state_map.push.remove(&TaskStatus::Todo);
    });
    env.fake.data().items.get_mut("uuid-1").unwrap().state = state("s-progress");
    env.run_forced(20);
    env.enqueue_status(&t.id, TaskStatus::Todo, 30);
    let r = env.run(40);
    assert_eq!(
        (r.pushed, r.errors.len(), r.notices.len()),
        (0, 0, 0),
        "{r:?}"
    );
    assert!(env.outbox().is_empty());
    assert_eq!(env.fake.data().set_states.len(), 1);
}

#[test]
fn enqueue_status_keeps_only_latest_and_ignores_local_tasks() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    env.enqueue_status(&t.id, TaskStatus::InProgress, 1);
    env.enqueue_status(&t.id, TaskStatus::InReview, 2);
    let rows = env.outbox();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].payload,
        OutboxPayload::SetState {
            state_id: "in_review".into()
        }
    );
    let conn = env.db.lock().unwrap();
    store::unlink_task(&conn, &t.id, 3).unwrap();
    assert!(!store::enqueue_status(&conn, &t.id, TaskStatus::Blocked, 4).unwrap());
    drop(conn);
    assert!(env.outbox().is_empty(), "unlink clears the outbox");
}

#[test]
fn pull_updates_title_plan_and_status() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    {
        let mut d = env.fake.data();
        let i = d.items.get_mut("uuid-1").unwrap();
        i.title = "New title".into();
        i.description_md = Some("New description.".into());
        i.state = state("s-done");
    }
    let r = env.run(50);
    assert_eq!(r.pulled, 1, "{r:?}");
    let t2 = env.task(&t.id);
    assert_eq!(t2.title, "New title");
    assert_eq!(t2.status, TaskStatus::Done);
    assert_eq!(t2.closed_at, Some(50));
    assert_eq!(
        t2.source
            .as_ref()
            .unwrap()
            .external_state
            .as_ref()
            .unwrap()
            .id,
        "s-done"
    );
    let plan = std::fs::read_to_string(plan_path(&env.dir, &t.id)).unwrap();
    assert!(plan.contains("New description.") && plan.contains("# ENG-1 · New title"));

    // Local change in Nodal while the external state stays put: the pull does not
    // overwrite it.
    env.db
        .lock()
        .unwrap()
        .execute("UPDATE tasks SET status = 'blocked' WHERE id = ?1", [&t.id])
        .unwrap();
    env.run(60);
    assert_eq!(env.task(&t.id).status, TaskStatus::Blocked);
}

#[test]
fn pull_does_not_overwrite_a_pending_local_status() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    env.enqueue_status(&t.id, TaskStatus::InReview, 1);
    env.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET status = 'in_review' WHERE id = ?1",
            [&t.id],
        )
        .unwrap();
    env.fake.data().fail_writes = Some(ProviderError::new(ErrorKind::Transient, "HTTP 502"));
    // While the push waits, someone moves the item in the provider.
    env.fake.data().items.get_mut("uuid-1").unwrap().state = state("s-done");
    env.run(10);
    let t2 = env.task(&t.id);
    assert_eq!(t2.status, TaskStatus::InReview);
    assert_eq!(
        t2.source.unwrap().external_state.unwrap().id,
        "s-todo",
        "not marked as seen"
    );
}

#[test]
fn pull_respects_overridden_plan() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    let path = plan_path(&env.dir, &t.id);
    std::fs::write(&path, "my plan").unwrap();
    env.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET plan_overridden = 1 WHERE id = ?1",
            [&t.id],
        )
        .unwrap();
    env.fake
        .data()
        .items
        .get_mut("uuid-1")
        .unwrap()
        .description_md = Some("other".into());
    env.run(10);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "my plan");
}

#[test]
fn pull_does_not_touch_status_with_active_run() {
    let env = Env::new();
    let t = env.import(1, "s-progress");
    env.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO runs (id, task_id, cwd, executor_json, kind, prompt, finish, status, queue_position, queued_at)
             VALUES ('run1', ?1, '/tmp', '{\"kind\":\"claude\"}', 'work', 'p', 'pr', 'launched', 1, 1)",
            [&t.id],
        )
        .unwrap();
    env.fake.data().items.get_mut("uuid-1").unwrap().state = state("s-canceled");
    env.run(10);
    let t2 = env.task(&t.id);
    assert_eq!(t2.status, TaskStatus::InProgress);
    // The change is consumed: it is not applied late when the run finishes.
    assert_eq!(t2.source.unwrap().external_state.unwrap().id, "s-canceled");
    env.db
        .lock()
        .unwrap()
        .execute("UPDATE runs SET status = 'finished'", [])
        .unwrap();
    env.run(20);
    assert_eq!(env.task(&t.id).status, TaskStatus::InProgress);
}

#[test]
fn pull_marks_unmapped_and_missing() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    let t2 = env.import(2, "s-todo");
    let qa = ExternalState {
        id: "s-qa".into(),
        name: "QA".into(),
        kind: ExtKind::Started,
        color: None,
    };
    {
        let mut d = env.fake.data();
        d.states.push(qa.clone());
        d.items.get_mut("uuid-1").unwrap().state = qa;
        d.items.remove("uuid-2");
    }
    let r = env.run(10);
    assert!(
        r.notices
            .iter()
            .any(|n| n.contains("new states to map: QA")),
        "{r:?}"
    );
    let a = env.task(&t.id);
    assert_eq!(a.status, TaskStatus::Todo);
    assert!(a
        .source
        .as_ref()
        .unwrap()
        .sync_error
        .as_ref()
        .unwrap()
        .contains("\"QA\" is not mapped"));
    assert!(a.source.as_ref().unwrap().unmapped);
    let b = env.task(&t2.id);
    let bs = b.source.unwrap();
    assert!(bs.sync_error.unwrap().contains("Not found"));
    assert!(!bs.unmapped);
    assert_eq!(r.pulled, 1);
    let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
    assert_eq!(
        (l.last_synced_at, l.last_sync_error),
        (Some(10), None),
        "a missing item is not a link error"
    );
    let pending = l.pending_state_changes.expect("QA is left pending review");
    assert_eq!(
        pending
            .added
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["s-qa"]
    );
    assert!(pending.removed.is_empty());
    // Unmapped is not marked as seen: once mapped, the next pull applies it.
    assert_eq!(
        a.source
            .as_ref()
            .unwrap()
            .external_state
            .as_ref()
            .unwrap()
            .id,
        "s-todo"
    );
    let mut map = env.link.state_map.clone();
    map.pull.insert("s-qa".into(), TaskStatus::InReview);
    map.known_states = env.fake.data().states.clone();
    store::save_state_map(&env.db.lock().unwrap(), &env.link.id, &map).unwrap();
    let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
    assert_eq!(
        l.pending_state_changes, None,
        "saving the mapping clears what was pending"
    );
    env.run(20);
    let a = env.task(&t.id);
    assert_eq!(a.status, TaskStatus::InReview);
    let src = a.source.unwrap();
    assert_eq!(src.sync_error, None);
    assert!(!src.unmapped);
    let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
    assert_eq!(
        (l.last_synced_at, l.pending_state_changes),
        (Some(20), None)
    );
}

#[test]
fn link_sync_error_is_recorded_and_cleared() {
    let env = Env::new();
    env.import(1, "s-todo");
    env.fake.data().fail_states = Some(ProviderError {
        kind: ErrorKind::Transient,
        message: "boom".into(),
    });
    let r = env.run(10);
    assert!(!r.errors.is_empty());
    let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
    assert_eq!(l.last_synced_at, Some(10));
    assert!(l.last_sync_error.unwrap().contains("boom"));
    env.fake.data().fail_states = None;
    env.run(20);
    let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
    assert_eq!((l.last_synced_at, l.last_sync_error), (Some(20), None));
}

#[test]
fn auto_import_routes_by_rule_or_reports() {
    let mut env = Env::new();
    env.set_link(|l| l.auto_import = true);
    {
        let mut d = env.fake.data();
        let mut docs = item(10, Some("## Acceptance criteria\n- Docs published\n"));
        docs.labels = vec!["docs".into()];
        let plain = item(11, None);
        let mut closed = item(12, None);
        closed.state = state("s-done");
        for i in [docs, plain, closed] {
            d.items.insert(i.external_id.clone(), i);
        }
    }
    let r = env.run(10);
    assert_eq!(r.imported, 1, "{r:?}");
    assert!(
        r.notices
            .iter()
            .any(|n| n.starts_with("ENG-11: not auto-imported")),
        "{r:?}"
    );
    let t = store::task_by_external(&env.db.lock().unwrap(), "fake", "uuid-10")
        .unwrap()
        .unwrap();
    assert_eq!(t.repo_id, "r-docs");
    assert_eq!(t.acceptance, vec!["Docs published"]);
    assert!(
        store::task_by_external(&env.db.lock().unwrap(), "fake", "uuid-12")
            .unwrap()
            .is_none()
    );

    // With a default repo, the missing one comes in; the one already imported is not
    // duplicated.
    env.set_link(|l| l.default_repo_id = Some("r-web".into()));
    let r = env.run(20);
    assert_eq!(r.imported, 1, "{r:?}");
    let t = store::task_by_external(&env.db.lock().unwrap(), "fake", "uuid-11")
        .unwrap()
        .unwrap();
    assert_eq!(t.repo_id, "r-web");
    assert_eq!(env.run(30).imported, 0);
}

#[test]
fn auto_import_with_deleted_repo_is_reported_not_retried() {
    let mut env = Env::new();
    // `default_repo_id` has an FK (ON DELETE SET NULL); rules do not: they can dangle.
    env.set_link(|l| {
        l.auto_import = true;
        l.repo_rules = vec![RepoRule::label("api", "r-gone")];
    });
    let mut it = item(20, None);
    it.labels = vec!["api".into()];
    env.fake.data().items.insert("uuid-20".into(), it);
    let r = env.run(10);
    assert_eq!(r.imported, 0);
    assert!(
        r.errors
            .iter()
            .any(|e| e.contains("waiting for 1 item (ENG-20)")),
        "{r:?}"
    );
    assert!(
        !r.errors.iter().any(|e| e.starts_with("Auto-import")),
        "no import attempt: {r:?}"
    );
    let l = store::get_link(&env.db.lock().unwrap(), &env.link.id).unwrap();
    assert!(l.last_sync_error.unwrap().contains("no longer exists"));
    env.set_link(|l| l.repo_rules[0].repo_id = "r-web".into());
    assert_eq!(env.run(20).imported, 1);
}

fn in_project(it: ExternalItem, proj: &str) -> ExternalItem {
    crate::providers::import::tests::in_project(it, proj)
}

/// 2026-09-10T00:00:00Z.
const RULE_AT: i64 = 1_788_998_400_000;

#[test]
fn project_rule_auto_imports_new_items_even_without_auto_import() {
    let mut env = Env::new();
    env.set_link(|l| {
        l.repo_rules.push(RepoRule::project(
            "proj-guides",
            "Guides",
            "r-docs",
            RULE_AT,
        ))
    });
    {
        let mut d = env.fake.data();
        let mut add = |n: u32, proj: &str, created: &str, st: &str| {
            let mut it = in_project(item(n, None), proj);
            it.created_at = Some(created.into());
            it.state = state(st);
            d.items.insert(it.external_id.clone(), it);
        };
        add(30, "guides", "2026-09-15T08:00:00.000Z", "s-todo"); // new and open: comes in
        add(31, "guides", "2026-09-01T08:00:00.000Z", "s-todo"); // older than the rule: backfill
        add(32, "guides", "2026-09-15T08:00:00.000Z", "s-done"); // closed
        add(33, "site", "2026-09-15T08:00:00.000Z", "s-todo"); // other project, link without auto-import
        add(34, "guides", "2026-09-16T08:00:00.000Z", "s-todo"); // unlinked by hand
    }
    let t34 = {
        let it = env.fake.data().items["uuid-34"].clone();
        let mut conn = env.db.lock().unwrap();
        import_items(
            &mut conn,
            &env.dir,
            &env.link,
            vec![(it, "r-docs".into())],
            1,
        )
        .unwrap()
        .imported
        .remove(0)
    };
    store::unlink_task(&env.db.lock().unwrap(), &t34.id, 2).unwrap();

    let r = env.run(RULE_AT + 1000);
    assert_eq!(r.imported, 1, "{r:?}");
    let c = env.db.lock().unwrap();
    let t = store::task_by_external(&c, "fake", "uuid-30")
        .unwrap()
        .unwrap();
    assert_eq!(t.repo_id, "r-docs");
    let src = t.source.unwrap();
    assert_eq!(src.rule_id.as_deref(), Some("rule-proj-guides"));
    assert_eq!(src.project.unwrap().name, "guides");
    for n in [31, 32, 33, 34] {
        assert!(
            store::task_by_external(&c, "fake", &format!("uuid-{n}"))
                .unwrap()
                .is_none(),
            "uuid-{n}"
        );
    }
    drop(c);
    let q = env.fake.data().queries.clone();
    assert_eq!(q.len(), 1, "only the rule's query: {q:?}");
    assert_eq!(
        (q[0].project_id.as_deref(), q[0].created_after),
        (Some("proj-guides"), Some(RULE_AT))
    );
    assert_eq!(env.run(RULE_AT + 2000).imported, 0);
}

fn move_to(env: &Env, n: u32, proj: Option<&str>) {
    let mut d = env.fake.data();
    let it = d.items.get_mut(&format!("uuid-{n}")).unwrap();
    it.scopes.retain(|s| s.kind != "project");
    if let Some(p) = proj {
        it.scopes.push(ScopeRef {
            kind: "project".into(),
            id: format!("proj-{p}"),
            name: p.into(),
        });
    }
}

fn import_in(env: &Env, n: u32, proj: &str, repo: &str) -> Task {
    let it = in_project(item(n, None), proj);
    env.fake
        .data()
        .items
        .insert(it.external_id.clone(), it.clone());
    let mut conn = env.db.lock().unwrap();
    import_items(&mut conn, &env.dir, &env.link, vec![(it, repo.into())], 1)
        .unwrap()
        .imported
        .remove(0)
}

fn resolve(env: &Env, id: &str, action: store::MovedAction) -> Result<Task, DbError> {
    store::resolve_moved(&env.db.lock().unwrap(), id, action, 99)
}

#[test]
fn project_change_flags_rule_tasks_and_resolves() {
    let mut env = Env::new();
    env.set_link(|l| {
        l.repo_rules
            .push(RepoRule::project("proj-site", "Site", "r-web", 1));
        l.repo_rules
            .push(RepoRule::project("proj-guides", "Guides", "r-docs", 1));
    });
    let t = import_in(&env, 40, "site", "r-web");
    assert_eq!(
        t.source.as_ref().unwrap().rule_id.as_deref(),
        Some("rule-proj-site")
    );
    let plain = import_in(&env, 42, "site", "r-docs"); // did not come in through the rule

    move_to(&env, 40, Some("guides"));
    move_to(&env, 42, Some("guides"));
    env.run(10);
    let t2 = env.task(&t.id);
    assert_eq!(t2.repo_id, "r-web", "does not move on its own");
    let m = t2.source.as_ref().unwrap().moved.clone().unwrap();
    assert_eq!(
        m.from_project,
        ExtProject {
            id: "proj-site".into(),
            name: "site".into()
        }
    );
    assert_eq!(m.to_project.unwrap().id, "proj-guides");
    assert_eq!(m.suggested_repo_id.as_deref(), Some("r-docs"));
    let p = env.task(&plain.id).source.unwrap();
    assert_eq!(
        (p.moved, p.project.unwrap().id),
        (None, "proj-guides".to_string())
    );
    assert_eq!(
        store::moved_ids(&env.db.lock().unwrap(), Some("p1")).unwrap(),
        vec![t.id.clone()]
    );

    // Back to the original project: the notice is cleared.
    move_to(&env, 40, Some("site"));
    env.run(20);
    assert_eq!(env.task(&t.id).source.unwrap().moved, None);

    // It leaves again; with an active run it cannot be moved, without one it can.
    move_to(&env, 40, Some("guides"));
    env.run(30);
    env.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO runs (id, task_id, cwd, executor_json, kind, prompt, finish, status, queue_position, queued_at)
             VALUES ('run1', ?1, '/tmp', '{\"kind\":\"claude\"}', 'work', 'p', 'pr', 'queued', 1, 1)",
            [&t.id],
        )
        .unwrap();
    let err = resolve(&env, &t.id, store::MovedAction::Move)
        .unwrap_err()
        .to_string();
    assert!(err.contains("queued or running run"), "{err}");
    assert_eq!(env.task(&t.id).repo_id, "r-web");
    env.db
        .lock()
        .unwrap()
        .execute("UPDATE runs SET status = 'finished'", [])
        .unwrap();
    env.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET wt_path = '/tmp/wt', wt_branch = 'b', wt_base = 'main' WHERE id = ?1",
            [&t.id],
        )
        .unwrap();
    let err = resolve(&env, &t.id, store::MovedAction::Move)
        .unwrap_err()
        .to_string();
    assert!(err.contains("worktree"), "{err}");
    env.db
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET wt_path = NULL, wt_branch = NULL, wt_base = NULL WHERE id = ?1",
            [&t.id],
        )
        .unwrap();
    let moved = resolve(&env, &t.id, store::MovedAction::Move).unwrap();
    assert_eq!(moved.repo_id, "r-docs");
    let src = moved.source.unwrap();
    assert_eq!(
        (src.moved, src.rule_id.as_deref()),
        (None, Some("rule-proj-guides"))
    );
    env.run(40);
    assert_eq!(
        env.task(&t.id).source.unwrap().moved,
        None,
        "does not notify again"
    );
    assert!(resolve(&env, &t.id, store::MovedAction::Keep)
        .unwrap_err()
        .to_string()
        .contains("no pending"));
}

#[test]
fn project_change_keep_and_same_repo() {
    let mut env = Env::new();
    env.set_link(|l| {
        l.repo_rules
            .push(RepoRule::project("proj-site", "Site", "r-web", 1));
        l.repo_rules
            .push(RepoRule::project("proj-alt", "Alt", "r-web", 1));
    });
    let a = import_in(&env, 50, "site", "r-web");
    let b = import_in(&env, 51, "site", "r-web");
    // No rule, label or default for the new project: notice with no suggested repo.
    move_to(&env, 50, None);
    // The new project goes to the same repo: nothing to decide.
    move_to(&env, 51, Some("alt"));
    env.run(10);
    let m = env.task(&a.id).source.unwrap().moved.unwrap();
    assert_eq!((m.to_project, m.suggested_repo_id), (None, None));
    let sb = env.task(&b.id).source.unwrap();
    assert_eq!(
        (sb.moved, sb.rule_id.as_deref()),
        (None, Some("rule-proj-alt"))
    );

    let err = resolve(&env, &a.id, store::MovedAction::Move)
        .unwrap_err()
        .to_string();
    assert!(err.contains("No repo is suggested"), "{err}");
    let kept = resolve(&env, &a.id, store::MovedAction::Keep).unwrap();
    assert_eq!(kept.repo_id, "r-web");
    let src = kept.source.unwrap();
    assert_eq!((src.moved, src.rule_id), (None, None));
    env.run(20);
    assert_eq!(
        env.task(&a.id).source.unwrap().moved,
        None,
        "keep: does not notify again"
    );
}

#[test]
fn disconnect_leaves_tasks_local() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    env.enqueue_comment(&t.id, "hello", 1);
    let n = store::disconnect_link(&mut env.db.lock().unwrap(), "l1", 5).unwrap();
    assert_eq!(n, 1);
    let t2 = env.task(&t.id);
    assert!(t2.source.is_none());
    assert_eq!(t2.title, t.title);
    assert!(env.outbox().is_empty());
    assert!(rows::get_source_link(&env.db.lock().unwrap(), "l1")
        .unwrap()
        .is_none());
}

#[test]
fn decide_pull_is_pure() {
    let env = Env::new();
    let t = env.import(1, "s-todo");
    let mut it = item(1, None);
    it.state = state("s-review");
    let d = decide_pull(&t, &it, &env.link.state_map, false, false);
    assert_eq!(d.status, Some(TaskStatus::InReview));
    assert_eq!(d.title, None);
    assert_eq!(
        decide_pull(&t, &it, &env.link.state_map, true, false).status,
        None
    );
    let pending = decide_pull(&t, &it, &env.link.state_map, false, true);
    assert_eq!(
        (pending.status, pending.record_state, pending.unmapped),
        (None, false, false)
    );
    it.state = state("s-todo");
    assert_eq!(
        decide_pull(&t, &it, &env.link.state_map, false, false).status,
        None
    );
}
