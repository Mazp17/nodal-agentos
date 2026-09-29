//! Facade tests: `SourcesHub`'s methods over an in-memory database, a fake provider (shared
//! through its factory so tests can seed its data) and a clock/notifier local to this module.

use std::sync::Mutex;

use nodal_domain::model::{ExternalState, Project};

use super::*;
use crate::sources::fake::FakeProvider;
use crate::sources::keys::testing::MemoryBackend;

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

/// Records every notice, for assertions.
#[derive(Default)]
struct RecordingNotifier {
    events: Mutex<Vec<(ChangeKind, Option<String>)>>,
}

impl RecordingNotifier {
    fn kinds(&self) -> Vec<ChangeKind> {
        self.events.lock().unwrap().iter().map(|(k, _)| *k).collect()
    }
}

impl ChangeNotifier for RecordingNotifier {
    fn notify(&self, kind: ChangeKind, project_id: Option<&str>) {
        self.events.lock().unwrap().push((kind, project_id.map(str::to_string)));
    }
}

/// Always returns the same `FakeProvider` (cloning shares its data), so tests can seed states
/// or items before exercising the hub.
struct FakeFactory(FakeProvider);

impl nodal_domain::ports::ProviderFactory for FakeFactory {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn build(&self, _key: String) -> Provider {
        Arc::new(self.0.clone())
    }
}

struct Fx {
    _t: nodal_host::testutil::TempDir,
    hub: Arc<SourcesHub>,
    notifier: Arc<RecordingNotifier>,
    fake: FakeProvider,
}

fn fx(name: &str) -> Fx {
    let t = nodal_host::testutil::TempDir::new(name);
    let fake = FakeProvider::default();
    let notifier = Arc::new(RecordingNotifier::default());
    let hub = SourcesHub::new(HubDeps {
        db: Some(Db::open_in_memory().unwrap()),
        data_dir: t.0.clone(),
        rt: crate::testutil::rt(),
        clock: Arc::new(FixedClock(1_000)),
        secrets: keys::Secrets::new(Arc::new(MemoryBackend::default())),
        providers: registry::ProviderRegistry::new(vec![Arc::new(FakeFactory(fake.clone()))]),
        notifier: notifier.clone(),
        plans: Arc::new(nodal_host::adapters::HostPlanFiles),
    });
    Fx { _t: t, hub, notifier, fake }
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    crate::testutil::block_on(fut)
}

fn seed_project(hub: &SourcesHub) {
    let db = hub.db.clone().unwrap();
    block_on(nodal_store::with_db(&db, |c| {
        nodal_store::rows::insert_project(
            c,
            &Project {
                id: "p1".into(),
                name: "Acme".into(),
                key: "ACM".into(),
                next_task_number: 1,
                color: "#fff".into(),
                default_executor: None,
                reviewer: None,
                created_at: 1,
                archived_at: None,
                root_path: None,
                description: None,
            },
        )
    }))
    .unwrap();
}

fn scope() -> ScopeRef {
    ScopeRef { kind: "team".into(), id: "team-eng".into(), name: "Engineering".into() }
}

fn seed_repo(hub: &SourcesHub, id: &str, project_id: &str) {
    let db = hub.db.clone().unwrap();
    let (id, project_id) = (id.to_string(), project_id.to_string());
    block_on(nodal_store::with_db(&db, move |c| {
        nodal_store::rows::insert_repo(
            c,
            &nodal_domain::model::Repo {
                id,
                project_id,
                path: "/tmp/repo".into(),
                name: "repo".into(),
                launch: nodal_domain::model::LaunchOptions::default(),
                default_executor: None,
                default_isolation: nodal_domain::model::Isolation::Worktree,
                default_finish: nodal_domain::model::Finish::Pr,
                default_review: true,
                reviewer: None,
                position: 0,
                created_at: 1,
            },
        )
    }))
    .unwrap();
}

/// A provider item: `uuid-{n}` / `ENG-{n}`, in the Engineering team, `Todo`.
fn item(n: u32) -> ExternalItem {
    ExternalItem {
        external_id: format!("uuid-{n}"),
        identifier: format!("ENG-{n}"),
        url: format!("https://example.com/{n}"),
        title: format!("Issue {n}"),
        description_md: None,
        state: ExternalState { id: "s-todo".into(), name: "Todo".into(), kind: ExtKind::Unstarted, color: None },
        scopes: vec![scope()],
        parent: None,
        children: Vec::new(),
        labels: Vec::new(),
        assignee: None,
        priority: nodal_domain::model::Priority::None,
        updated_at: "2026-01-01T00:00:00.000Z".into(),
        created_at: Some("2026-01-01T00:00:00.000Z".into()),
        closed_at: None,
    }
}

/// Links `p1` to the fake provider (`create_source_link` with an empty mapping/rules).
fn linked(hub: &SourcesHub) -> SourceLink {
    block_on(hub.create_source_link(NewSourceLink {
        project_id: "p1".into(),
        provider: "fake".into(),
        scope: scope(),
        default_repo_id: None,
        repo_rules: Vec::new(),
        auto_import: false,
    }))
    .unwrap()
}

#[test]
fn unknown_provider_is_rejected() {
    let f = fx("unknown-provider");
    assert!(f.hub.check_provider("fake").is_ok());
    assert!(f.hub.check_provider("jira").unwrap_err().contains("Unknown provider"));
}

#[test]
fn provider_status_shape() {
    let s = ProviderStatus {
        provider: "linear".into(),
        has_key: true,
        viewer: Some("Ana".into()),
        error: None,
        key_hint: key_hint("lin_api_0000000000abcd"),
        paused_until: Some(5),
        pause_reason: None,
    };
    let v = serde_json::to_value(s).unwrap();
    assert_eq!(v["hasKey"], true);
    assert_eq!(v["keyHint"], "abcd");
    assert_eq!(v["viewer"], "Ana");
    assert!(v["error"].is_null());
    assert_eq!(v["pausedUntil"], 5);
}

#[test]
fn key_hint_never_reveals_the_key() {
    assert_eq!(key_hint("  lin_api_0123456789wxyz \n").as_deref(), Some("wxyz"));
    assert_eq!(key_hint("short-key"), None, "short keys give no hint");
    assert_eq!(key_hint(""), None);
}

#[test]
fn provider_status_without_a_key_reports_no_key() {
    let f = fx("status-no-key");
    let s = block_on(f.hub.provider_status("fake".into())).unwrap();
    assert!(!s.has_key);
    assert_eq!(s.viewer, None);
}

#[test]
fn provider_set_key_validates_and_round_trips_through_status_and_clear() {
    let f = fx("set-key");
    let err = block_on(f.hub.provider_set_key("fake".into(), Some("  ".into()))).unwrap_err();
    assert!(err.to_string().contains("API key is empty"));
    let s = block_on(f.hub.provider_set_key("fake".into(), Some("lin_api_0123456789wxyz".into()))).unwrap();
    assert!(s.has_key);
    assert_eq!(s.viewer.as_deref(), Some("Fake User"));
    assert_eq!(s.key_hint.as_deref(), Some("wxyz"));
    let s = block_on(f.hub.provider_status("fake".into())).unwrap();
    assert!(s.has_key);
    let s = block_on(f.hub.provider_clear_key("fake".into())).unwrap();
    assert!(!s.has_key);
    let s = block_on(f.hub.provider_status("fake".into())).unwrap();
    assert!(!s.has_key);
}

#[test]
fn provider_scopes_rejects_an_unknown_provider() {
    let f = fx("scopes-unknown");
    let err = block_on(f.hub.provider_scopes("jira".into())).unwrap_err();
    assert!(err.to_string().contains("Unknown provider"));
}

#[test]
fn create_source_link_persists_and_notifies() {
    let f = fx("create-link");
    seed_project(&f.hub);
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    let input = NewSourceLink {
        project_id: "p1".into(),
        provider: "fake".into(),
        scope: scope(),
        default_repo_id: None,
        repo_rules: Vec::new(),
        auto_import: false,
    };
    let link = block_on(f.hub.create_source_link(input)).unwrap();
    assert_eq!(link.project_id, "p1");
    assert_eq!(f.notifier.kinds(), vec![ChangeKind::Sources]);
    let links = block_on(f.hub.list_source_links(None)).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].id, link.id);
}

#[test]
fn create_source_link_rejects_an_unknown_provider() {
    let f = fx("create-link-unknown");
    seed_project(&f.hub);
    let input =
        NewSourceLink { project_id: "p1".into(), provider: "jira".into(), scope: scope(), default_repo_id: None, repo_rules: Vec::new(), auto_import: false };
    let err = block_on(f.hub.create_source_link(input)).unwrap_err();
    assert!(err.to_string().contains("Unknown provider"));
    assert!(f.notifier.kinds().is_empty());
}

#[test]
fn update_source_link_applies_the_patch_and_notifies() {
    let f = fx("update-link");
    seed_project(&f.hub);
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    let link = linked(&f.hub);
    let patch: SourceLinkPatch = serde_json::from_value(serde_json::json!({"autoImport": true})).unwrap();
    let updated = block_on(f.hub.update_source_link(link.id.clone(), patch)).unwrap();
    assert!(updated.auto_import);
    assert_eq!(f.notifier.kinds(), vec![ChangeKind::Sources, ChangeKind::Sources]);
}

#[test]
fn delete_source_link_disconnects_and_notifies() {
    let f = fx("delete-link");
    seed_project(&f.hub);
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    let link = linked(&f.hub);
    block_on(f.hub.delete_source_link(link.id)).unwrap();
    assert!(block_on(f.hub.list_source_links(None)).unwrap().is_empty());
    assert_eq!(f.notifier.kinds(), vec![ChangeKind::Sources, ChangeKind::Sources, ChangeKind::Tasks]);
}

#[test]
fn save_state_map_confirms_and_persists() {
    let f = fx("save-state-map");
    seed_project(&f.hub);
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    f.fake.data().states = vec![ExternalState { id: "s-todo".into(), name: "Todo".into(), kind: ExtKind::Unstarted, color: None }];
    let link = linked(&f.hub);
    let report = block_on(f.hub.source_states(link.id.clone())).unwrap();
    assert_eq!(report.states.len(), 1);
    let saved = block_on(f.hub.save_state_map(link.id, link.state_map.clone())).unwrap();
    assert_eq!(saved.state_map.confirmed_at, Some(1_000));
    assert_eq!(saved.state_map.known_states.len(), 1);
}

#[test]
fn provider_list_importable_returns_items_from_the_provider() {
    let f = fx("list-importable");
    seed_project(&f.hub);
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    f.fake.data().items.insert("uuid-1".into(), item(1));
    let link = linked(&f.hub);
    let items = block_on(f.hub.provider_list_importable(link.id, None, None)).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].external_id, "uuid-1");
}

#[test]
fn import_tasks_imports_and_reports_missing_items() {
    let f = fx("import-tasks");
    seed_project(&f.hub);
    seed_repo(&f.hub, "r1", "p1");
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    f.fake.data().items.insert("uuid-1".into(), item(1));
    let link = linked(&f.hub);
    let result = block_on(f.hub.import_tasks(
        "p1".into(),
        link.id,
        vec![
            ImportRequest { external_id: "uuid-1".into(), repo_id: "r1".into() },
            ImportRequest { external_id: "uuid-404".into(), repo_id: "r1".into() },
        ],
    ))
    .unwrap();
    assert_eq!(result.imported.len(), 1);
    assert_eq!(result.skipped.len(), 1);
    assert!(result.skipped[0].reason.contains("Not found"));
    assert_eq!(f.notifier.kinds(), vec![ChangeKind::Sources, ChangeKind::Tasks]);
}

#[test]
fn import_tasks_rejects_a_link_from_another_project() {
    let f = fx("import-tasks-wrong-project");
    seed_project(&f.hub);
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    let link = linked(&f.hub);
    let err = block_on(f.hub.import_tasks("p2".into(), link.id, Vec::new())).unwrap_err();
    assert!(err.to_string().contains("another project"));
}

#[test]
fn source_rule_projects_returns_the_providers_projects() {
    let f = fx("rule-projects");
    seed_project(&f.hub);
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    let proj = ScopeRef { kind: "project".into(), id: "proj-a".into(), name: "A".into() };
    f.fake.data().projects = vec![proj.clone()];
    let link = linked(&f.hub);
    let projects = block_on(f.hub.source_rule_projects(link.id)).unwrap();
    assert_eq!(projects, vec![proj]);
}

#[test]
fn unlink_task_makes_it_local() {
    let f = fx("unlink-task");
    seed_project(&f.hub);
    seed_repo(&f.hub, "r1", "p1");
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    f.fake.data().items.insert("uuid-1".into(), item(1));
    let link = linked(&f.hub);
    let result =
        block_on(f.hub.import_tasks("p1".into(), link.id, vec![ImportRequest { external_id: "uuid-1".into(), repo_id: "r1".into() }])).unwrap();
    let task = result.imported.into_iter().next().unwrap();
    assert!(task.source.is_some());
    let unlinked = block_on(f.hub.unlink_task(task.id)).unwrap();
    assert!(unlinked.source.is_none());
}

#[test]
fn sync_now_without_a_database_reports_the_message() {
    let t = nodal_host::testutil::TempDir::new("sync-no-db");
    let hub = SourcesHub::new(HubDeps {
        db: None,
        data_dir: t.0.clone(),
        rt: crate::testutil::rt(),
        clock: Arc::new(FixedClock(1)),
        secrets: keys::Secrets::new(Arc::new(MemoryBackend::default())),
        providers: registry::ProviderRegistry::new(vec![]),
        notifier: Arc::new(crate::testutil::NoopNotifier),
        plans: Arc::new(nodal_host::adapters::HostPlanFiles),
    });
    let err = block_on(hub.sync_now(None)).unwrap_err();
    assert_eq!(err.to_string(), "The database is not available.");
}

/// P15: a pass with nothing to pull/push/import and no errors/notices doesn't emit `Sources`
/// (`last_synced_at` still moves in the DB; only the change-notify event is gated).
#[test]
fn sync_now_without_changes_does_not_notify() {
    let f = fx("sync-no-changes");
    seed_project(&f.hub);
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    let _link = linked(&f.hub); // already notifies once (Sources), unrelated to the sync pass
    let before = f.notifier.kinds().len();
    let report = block_on(f.hub.sync_now(None)).unwrap();
    assert_eq!(report.pulled + report.pushed + report.imported, 0);
    assert!(report.errors.is_empty());
    assert!(report.notices.is_empty());
    assert_eq!(f.notifier.kinds().len(), before, "a no-op sync pass should not emit a Sources event");
}

/// A pass that actually imports something still notifies (`Sources` + `Tasks`).
#[test]
fn sync_now_with_changes_notifies() {
    let f = fx("sync-with-changes");
    seed_project(&f.hub);
    seed_repo(&f.hub, "r1", "p1");
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    f.fake.data().items.insert("uuid-1".into(), item(1));
    let link = linked(&f.hub);
    block_on(f.hub.update_source_link(
        link.id.clone(),
        serde_json::from_value(serde_json::json!({"defaultRepoId": "r1", "autoImport": true})).unwrap(),
    ))
    .unwrap();
    let before = f.notifier.kinds().len();
    let report = block_on(f.hub.sync_now(None)).unwrap();
    assert_eq!(report.imported, 1);
    assert!(f.notifier.kinds().len() > before, "an importing sync pass should emit a Sources event");
}

/// P18: a panic inside a sync pass (the exact task `spawn_sync_worker` awaits every tick)
/// must not take the runtime down with it — with `panic = "unwind"`, tokio catches it and
/// reports it as a `JoinError`; the hub's `sync_lock` (a `tokio::sync::Mutex`, which just
/// unlocks on drop, unwind or not) isn't left stuck either.
#[test]
fn a_panic_in_a_sync_pass_only_fails_that_task() {
    let f = fx("sync-panic");
    seed_project(&f.hub);
    block_on(f.hub.secrets.set("fake", "k")).unwrap();
    let _link = linked(&f.hub);
    f.fake.data().panic_on_states = true;

    let hub = f.hub.clone();
    let handle = f.hub.rt.spawn(async move { hub.sync_now(None).await });
    let joined = block_on(handle);
    let panic_payload = joined.expect_err("a panicking sync pass must surface as Err");
    assert!(panic_payload.is_panic(), "{panic_payload:?}");

    // The runtime is still healthy: unrelated work submitted right after still completes.
    let ok = f.hub.rt.spawn(async { 1 + 1 });
    assert_eq!(block_on(ok).unwrap(), 2);

    // `FakeProvider`'s inner `std::sync::Mutex` is now poisoned (the panic happened while its
    // guard was held); `FakeProvider::data()` recovers it the same way production `Mutex`es do
    // (`.unwrap_or_else(|p| p.into_inner())`) instead of propagating the poison forever.
    assert!(f.fake.0.lock().is_err(), "the fixture's mutex should be poisoned at this point");
    f.fake.data().panic_on_states = false;

    // The hub itself recovers once the fault clears: a normal pass isn't stuck behind the
    // panicking one's lock.
    let report = block_on(f.hub.sync_now(None)).unwrap();
    assert_eq!(report.pulled + report.pushed + report.imported, 0);
}
