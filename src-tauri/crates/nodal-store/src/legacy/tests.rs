use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::Db;

const T0: i64 = 1_800_000_000_000;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/legacy/fixtures").join(name)
}

/// The test's own temporary folder; deleted when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("nodal-migrate-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Self(p.canonicalize().unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Every file in a tree, with its content (relative path → bytes).
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(p.strip_prefix(root).unwrap().display().to_string(), fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn copy_tree(from: &Path, to: &Path) {
    for e in fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        let target = to.join(p.file_name().unwrap());
        if p.is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_tree(&p, &target);
        } else {
            fs::copy(&p, &target).unwrap();
        }
    }
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

fn counts(conn: &Connection) -> Vec<i64> {
    ["projects", "repos", "source_links", "tasks", "settings", "legacy_imports"]
        .iter()
        .map(|t| count(conn, t))
        .collect()
}

fn repo_id_of(conn: &Connection, path: &str) -> String {
    conn.query_row("SELECT id FROM repos WHERE path = ?1", [path], |r| r.get(0)).unwrap()
}

#[test]
fn legacy_import_is_idempotent_and_maps_everything() {
    let data = TempDir::new("data");
    let db = Db::open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();

    let r1 = import_folder(&mut conn, &fixture("legacy"), &data.0, T0).unwrap();
    assert_eq!((r1.projects, r1.repos, r1.tasks, r1.runs), (4, 4, 3, 0), "{r1:#?}");
    assert_eq!(r1.already_imported, 0);
    assert_eq!(r1.skipped.len(), 1, "{:?}", r1.skipped);
    assert!(r1.skipped.iter().any(|s| s.starts_with("config.json: record #4")));
    let after_first = counts(&conn);

    // Second time: nothing new, everything counts as already imported.
    let r2 = import_folder(&mut conn, &fixture("legacy"), &data.0, T0 + 1).unwrap();
    assert_eq!((r2.projects, r2.repos, r2.tasks, r2.runs), (0, 0, 0, 0), "{r2:#?}");
    // 3 repos + 3 links + 3 tasks.
    assert_eq!(r2.already_imported, 9);
    assert_eq!(counts(&conn), after_first);
    assert_eq!(r1.backup_dir, r2.backup_dir, "same content: the backup is reused");

    // Projects: one per mapped repo (folder name: legacy doesn't store names) + Local.
    let mut names: Vec<(String, String)> = conn
        .prepare("SELECT name, key FROM projects ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    names.sort();
    assert_eq!(
        names,
        [("Local", "LOCA"), ("acme-api", "AA"), ("acme-mobile", "AM"), ("acme-web", "AW")]
            .map(|(a, b)| (a.to_string(), b.to_string()))
    );

    // Repos: options from the mapping; the trailing `/` is normalized.
    let api = rows::get_repo(&conn, &repo_id_of(&conn, "/Users/me/Code/acme-api")).unwrap().unwrap();
    assert_eq!(api.launch.model.as_deref(), Some("opus"));
    assert_eq!(api.launch.effort.as_deref(), Some("high"));
    let web_id = repo_id_of(&conn, "/Users/me/Code/acme-web");

    // SourceLinks: team alone → team; team + project → project. Mapping pending.
    let links: Vec<SourceLink> = conn
        .prepare("SELECT * FROM source_links ORDER BY scope_id")
        .unwrap()
        .query_map([], rows::source_link_from_row)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let scopes: Vec<(&str, &str)> = links.iter().map(|l| (l.scope.kind.as_str(), l.scope.id.as_str())).collect();
    assert_eq!(scopes, [("project", "proj-mobile"), ("project", "proj-web"), ("team", "team-eng")]);
    assert!(links.iter().all(|l| l.provider == "linear" && l.state_map.confirmed_at.is_none()));
    let web_link = links.iter().find(|l| l.scope.id == "proj-web").unwrap();
    assert_eq!(web_link.default_repo_id.as_deref(), Some(web_id.as_str()));

    // Tasks: numbered by creation date; the one from an unmapped repo goes to "Local".
    let t1 = rows::get_task(&conn, "t01aaaaaaaa").unwrap().unwrap();
    assert_eq!((t1.number, t1.status, t1.plan.clone()), (1, TaskStatus::Todo, PlanRef::Text));
    assert_eq!(t1.repo_id, api.id);
    let t2 = rows::get_task(&conn, "t02bbbbbbbb").unwrap().unwrap();
    let local = rows::get_project(&conn, &t2.project_id).unwrap().unwrap();
    assert_eq!(local.name, "Local");
    assert_eq!(local.next_task_number, 2);
    assert_eq!((t2.status, t2.closed_at), (TaskStatus::Done, Some(1_700_000_009_000)));
    let t3 = rows::get_task(&conn, "t03cccccccc").unwrap().unwrap();
    assert_eq!(t3.plan, PlanRef::File { path: "/Users/me/Code/acme-web/docs/plan.md".into() });
    assert_eq!(t3.repo_id, web_id);

    // Plans copied to `<data>/tasks/<id>/plan.md`.
    assert_eq!(
        fs::read(data.0.join("tasks/t01aaaaaaaa/plan.md")).unwrap(),
        fs::read(fixture("legacy/tasks/t01aaaaaaaa/plan.md")).unwrap()
    );
    assert!(data.0.join("tasks/t02bbbbbbbb/plan.md").is_file());

}

#[test]
fn source_folder_stays_intact_and_backup_is_a_full_copy() {
    // Import from a copy in temp, so metadata can be compared without touching the repo.
    let src = TempDir::new("src");
    let data = TempDir::new("data");
    copy_tree(&fixture("legacy"), &src.0);
    let before = snapshot(&src.0);
    let mtimes = |root: &Path| -> Vec<std::time::SystemTime> {
        let mut v: Vec<_> = before.keys().map(|k| fs::metadata(root.join(k)).unwrap().modified().unwrap()).collect();
        v.push(fs::metadata(root).unwrap().modified().unwrap());
        v
    };
    let mtimes_before = mtimes(&src.0);

    let db = Db::open_in_memory().unwrap();
    let report = import_folder(&mut db.lock().unwrap(), &src.0, &data.0, T0).unwrap();

    assert_eq!(snapshot(&src.0), before, "the source is unchanged byte for byte");
    assert_eq!(mtimes(&src.0), mtimes_before);
    let backup = PathBuf::from(&report.backup_dir);
    assert_eq!(backup.parent().unwrap(), data.0);
    assert!(backup.file_name().unwrap().to_string_lossy().starts_with(BACKUP_PREFIX));
    assert_eq!(snapshot(&backup), before, "the backup is an exact copy");
}

#[test]
fn importing_the_data_folder_itself_skips_backups_and_db() {
    let data = TempDir::new("self");
    copy_tree(&fixture("legacy"), &data.0);
    fs::write(data.0.join("nodal.db"), b"sqlite").unwrap();
    fs::create_dir(data.0.join("legacy-backup-1")).unwrap();
    fs::write(data.0.join("legacy-backup-1/old.json"), b"{}").unwrap();

    let db = Db::open_in_memory().unwrap();
    let report = import_folder(&mut db.lock().unwrap(), &data.0, &data.0, T0).unwrap();
    let copied = snapshot(Path::new(&report.backup_dir));
    assert!(copied.contains_key("tasks.json"));
    assert!(!copied.keys().any(|k| k.starts_with("nodal.db") || k.starts_with(BACKUP_PREFIX)));
    assert_eq!(report.tasks, 3);
}

#[test]
fn corrupt_files_are_skipped_without_aborting() {
    let data = TempDir::new("corrupt");
    let db = Db::open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    let r = import_folder(&mut conn, &fixture("corrupt"), &data.0, T0).unwrap();
    assert_eq!((r.projects, r.repos, r.tasks, r.runs), (1, 1, 0, 0), "{r:#?}");
    assert!(r.skipped.iter().any(|s| s.starts_with("tasks.json: corrupt JSON")), "{:?}", r.skipped);
}

#[test]
fn current_config_format_with_links() {
    let data = TempDir::new("current");
    let db = Db::open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    let r = import_folder(&mut conn, &fixture("current"), &data.0, T0).unwrap();
    assert_eq!((r.projects, r.repos), (1, 1));
    let p: Project = conn.query_row("SELECT * FROM projects", [], rows::project_from_row).unwrap();
    assert_eq!((p.name.as_str(), p.key.as_str()), ("Payments", "PAYM"));
    let mut scopes: Vec<(String, String, String)> = conn
        .prepare("SELECT provider, scope_kind, scope_name FROM source_links ORDER BY scope_kind")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    scopes.sort();
    assert_eq!(
        scopes,
        [("linear", "project", "Checkout"), ("linear", "team", "Payments")]
            .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
    );

    let again = import_folder(&mut conn, &fixture("current"), &data.0, T0 + 5).unwrap();
    assert_eq!((again.projects, again.repos, again.already_imported), (0, 0, 3));
}

#[test]
fn existing_repo_is_reused_and_keys_do_not_collide() {
    let data = TempDir::new("existing");
    let db = Db::open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    // The user already created a project with the web repo and another using the key "AA".
    for (id, key) in [("mine", "WEB"), ("other", "AA")] {
        rows::insert_project(
            &conn,
            &Project {
                id: id.into(),
                name: id.into(),
                key: key.into(),
                next_task_number: 1,
                color: "#fff".into(),
                created_at: 1,
                archived_at: None,
                root_path: None,
                description: None,
            },
        )
        .unwrap();
    }
    rows::insert_repo(
        &conn,
        &Repo {
            id: "web".into(),
            project_id: "mine".into(),
            path: "/Users/me/Code/acme-web".into(),
            name: "acme-web".into(),
            launch: LaunchOptions::default(),
            position: 0,
            created_at: 1,
        },
    )
    .unwrap();

    let r = import_folder(&mut conn, &fixture("legacy"), &data.0, T0).unwrap();
    assert_eq!((r.projects, r.repos), (3, 3), "acme-web already existed");
    let link_project: String = conn
        .query_row("SELECT project_id FROM source_links WHERE scope_id = 'proj-web'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(link_project, "mine");
    let t3 = rows::get_task(&conn, "t03cccccccc").unwrap().unwrap();
    assert_eq!((t3.project_id.as_str(), t3.repo_id.as_str()), ("mine", "web"));
    let api_key: String = conn
        .query_row(
            "SELECT p.key FROM projects p JOIN repos r ON r.project_id = p.id WHERE r.path = '/Users/me/Code/acme-api'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(api_key, "AA2");
}

#[test]
fn helpers() {
    assert_eq!(key_base("my-app"), "MA");
    assert_eq!(key_base("payments"), "PAYM");
    assert_eq!(key_base("2fa"), "P2FA");
    assert_eq!(key_base("--"), "P");
    assert_eq!(canon_path("/Users/me/Code/x/"), "/Users/me/Code/x");
    assert_eq!(fnv("abc"), fnv("abc"));
    assert_eq!(fnv(""), "cbf29ce484222325");
}

#[test]
fn reimport_after_state_changes_and_duplicate_entries() {
    let src = TempDir::new("changing");
    let data = TempDir::new("changing-data");
    // Same path in two entries (team and project): one repo, two links, nothing "already imported".
    fs::write(
        src.0.join("config.json"),
        r#"{"repos":[{"teamId":"team-a","path":"/Users/me/Code/acme-one"},
                     {"projectId":"proj-b","path":"/Users/me/Code/acme-one/"}]}"#,
    )
    .unwrap();
    let db = Db::open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    let r1 = import_folder(&mut conn, &src.0, &data.0, T0).unwrap();
    assert_eq!((r1.projects, r1.repos, r1.runs, r1.already_imported), (1, 1, 0, 0), "{r1:#?}");
    assert_eq!(count(&conn, "source_links"), 2);
}

#[test]
fn reimport_reuses_a_repo_whose_path_changed() {
    let src = TempDir::new("moved");
    let data = TempDir::new("moved-data");
    let task = |id: &str, at: i64| {
        format!(r#"{{"id":"{id}","repoPath":"/Users/me/Code/acme-tools","title":"T {id}","plan":{{"kind":"text"}},"status":"todo","createdAt":{at}}}"#)
    };
    fs::write(src.0.join("tasks.json"), format!(r#"{{"tasks":[{}]}}"#, task("ta1", 1))).unwrap();
    fs::create_dir_all(src.0.join("tasks/ta1")).unwrap();
    fs::write(src.0.join("tasks/ta1/plan.md"), "plan").unwrap();
    let db = Db::open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    import_folder(&mut conn, &src.0, &data.0, T0).unwrap();
    let repo = repo_id_of(&conn, "/Users/me/Code/acme-tools");
    conn.execute("UPDATE repos SET path = '/Users/me/Code/tools-moved' WHERE id = ?1", [&repo]).unwrap();

    // The old version kept creating tasks with the old path.
    fs::write(src.0.join("tasks.json"), format!(r#"{{"tasks":[{},{}]}}"#, task("ta1", 1), task("ta2", 2))).unwrap();
    let r = import_folder(&mut conn, &src.0, &data.0, T0 + 1).unwrap();
    assert_eq!((r.tasks, r.repos), (1, 0), "{r:#?}");
    let repo_of: String = conn.query_row("SELECT repo_id FROM tasks WHERE id = 'ta2'", [], |r| r.get(0)).unwrap();
    assert_eq!(repo_of, repo);
}

#[test]
fn folder_without_legacy_files_is_rejected_and_nothing_is_copied() {
    // Like choosing `~`: lots of everything, nothing in the old format (nested files don't count).
    let src = TempDir::new("home");
    let data = TempDir::new("home-data");
    fs::create_dir_all(src.0.join("Code/app/data")).unwrap();
    fs::write(src.0.join("Code/app/data/tasks.json"), b"{\"tasks\":[]}").unwrap();
    fs::write(src.0.join("notes.txt"), b"hello").unwrap();
    let db = Db::open_in_memory().unwrap();
    let e = import_folder(&mut db.lock().unwrap(), &src.0, &data.0, T0).unwrap_err();
    assert!(e.contains("doesn't look like a data folder") && e.contains("Nothing was copied"), "{e}");
    assert_eq!(fs::read_dir(&data.0).unwrap().count(), 0, "no backup folder");
}

#[test]
fn only_known_files_are_copied() {
    let src = TempDir::new("mixed");
    let data = TempDir::new("mixed-data");
    fs::write(src.0.join("tasks.json"), b"{\"tasks\":[]}").unwrap();
    fs::write(src.0.join("nodal.db"), b"sqlite").unwrap();
    fs::write(src.0.join("big.bin"), vec![0u8; 1024]).unwrap();
    fs::create_dir_all(src.0.join("tasks/t1")).unwrap();
    fs::write(src.0.join("tasks/t1/plan.md"), b"plan").unwrap();
    fs::write(src.0.join("tasks/t1/notes.txt"), b"x").unwrap();
    fs::create_dir_all(src.0.join("legacy-backup-1")).unwrap();
    fs::write(src.0.join("legacy-backup-1/tasks.json"), b"{}").unwrap();
    let db = Db::open_in_memory().unwrap();
    let r = import_folder(&mut db.lock().unwrap(), &src.0, &data.0, T0).unwrap();
    let copied = snapshot(Path::new(&r.backup_dir));
    assert_eq!(copied.keys().collect::<Vec<_>>(), ["tasks.json", "tasks/t1/plan.md"]);
}

#[test]
fn oversized_data_is_rejected_before_copying() {
    let src = TempDir::new("huge");
    let data = TempDir::new("huge-data");
    // Sparse file: takes up `len` without writing 64 MB.
    let f = fs::File::create(src.0.join("tasks.json")).unwrap();
    f.set_len(MAX_BACKUP_BYTES + 1).unwrap();
    let e = backup(&src.0, &data.0, T0).unwrap_err();
    assert!(e.contains("more than the 64 MB"), "{e}");
    assert_eq!(fs::read_dir(&data.0).unwrap().count(), 0);
}

#[test]
fn backup_is_reused_only_while_content_is_identical() {
    let src = TempDir::new("reuse");
    let data = TempDir::new("reuse-data");
    fs::write(src.0.join("tasks.json"), b"{\"tasks\":[]}").unwrap();
    let (b1, _) = backup(&src.0, &data.0, T0).unwrap();
    let (b2, _) = backup(&src.0, &data.0, T0 + 1).unwrap();
    assert_eq!(b1, b2);
    fs::write(src.0.join("config.json"), b"{}").unwrap();
    let (b3, _) = backup(&src.0, &data.0, T0 + 2).unwrap();
    assert_ne!(b3, b1, "a new file means a new backup");
    fs::write(src.0.join("config.json"), b"{\"repos\":[]}").unwrap();
    let (b4, _) = backup(&src.0, &data.0, T0 + 3).unwrap();
    assert_ne!(b4, b3, "changed bytes mean a new backup");
    assert_eq!(fs::read_dir(&data.0).unwrap().count(), 3);
}
