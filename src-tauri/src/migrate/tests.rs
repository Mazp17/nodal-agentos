use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::*;
use crate::db::{open_in_memory, rows};

const T0: i64 = 1_800_000_000_000;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/migrate/fixtures").join(name)
}

/// Carpeta temporal propia del test; se borra al soltarla.
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

/// Todos los archivos de un árbol, con su contenido (ruta relativa → bytes).
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
    let none = Path::new("/nonexistent");
    copy_dir(from, to, &Skip { dest: none, data_dir: none }, &mut Vec::new()).unwrap();
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

fn counts(conn: &Connection) -> Vec<i64> {
    ["projects", "repos", "source_links", "tasks", "runs", "settings", "legacy_imports"]
        .iter()
        .map(|t| count(conn, t))
        .collect()
}

fn run_by_label(conn: &Connection, label: &str) -> Run {
    conn.query_row("SELECT * FROM runs WHERE legacy_label = ?1", [label], rows::run_from_row).unwrap()
}

fn repo_id_of(conn: &Connection, path: &str) -> String {
    conn.query_row("SELECT id FROM repos WHERE path = ?1", [path], |r| r.get(0)).unwrap()
}

#[test]
fn legacy_import_is_idempotent_and_maps_everything() {
    let data = TempDir::new("data");
    let db = open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();

    let r1 = import_folder(&mut conn, &fixture("legacy"), &data.0, T0).unwrap();
    assert_eq!((r1.projects, r1.repos, r1.tasks, r1.runs), (4, 4, 3, 6), "{r1:#?}");
    assert_eq!(r1.already_imported, 0);
    assert_eq!(r1.skipped.len(), 2, "{:?}", r1.skipped);
    assert!(r1.skipped.iter().any(|s| s.starts_with("config.json: record #4")));
    assert!(r1.skipped.iter().any(|s| s.starts_with("issue-runs.json: record #3")));
    let after_first = counts(&conn);

    // Segunda vez: nada nuevo, todo cuenta como ya importado.
    let r2 = import_folder(&mut conn, &fixture("legacy"), &data.0, T0 + 1).unwrap();
    assert_eq!((r2.projects, r2.repos, r2.tasks, r2.runs), (0, 0, 0, 0), "{r2:#?}");
    // 3 repos + 3 links + concurrency + 3 tareas + 6 runs.
    assert_eq!(r2.already_imported, 16);
    assert_eq!(counts(&conn), after_first);
    assert_ne!(r1.backup_dir, r2.backup_dir);

    // Proyectos: uno por repo mapeado (nombre de carpeta: el legacy no guarda nombres) + Local.
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

    // Repos: opciones y finish del mapeo; el `/` final se normaliza.
    let api = rows::get_repo(&conn, &repo_id_of(&conn, "/Users/me/Code/acme-api")).unwrap().unwrap();
    assert_eq!(api.launch.model.as_deref(), Some("opus"));
    assert_eq!(api.launch.effort.as_deref(), Some("high"));
    assert_eq!(api.default_finish, Finish::Commit);
    let web_id = repo_id_of(&conn, "/Users/me/Code/acme-web");

    // SourceLinks: team solo → team; team + proyecto → proyecto. Mapeo pendiente.
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

    // Tareas: numeradas por fecha de creación; la de un repo sin mapeo va a "Local".
    let t1 = rows::get_task(&conn, "t01aaaaaaaa").unwrap().unwrap();
    assert_eq!((t1.number, t1.status, t1.plan.clone()), (1, TaskStatus::Todo, PlanRef::Text));
    assert_eq!(t1.repo_id, api.id);
    assert_eq!(t1.assignee, Some(Executor::Workflow { name: "plan-task".into() }));
    let t2 = rows::get_task(&conn, "t02bbbbbbbb").unwrap().unwrap();
    let local = rows::get_project(&conn, &t2.project_id).unwrap().unwrap();
    assert_eq!(local.name, "Local");
    assert_eq!(local.next_task_number, 2);
    assert_eq!((t2.status, t2.closed_at), (TaskStatus::Done, Some(1_700_000_009_000)));
    let t3 = rows::get_task(&conn, "t03cccccccc").unwrap().unwrap();
    assert_eq!(t3.plan, PlanRef::File { path: "/Users/me/Code/acme-web/docs/plan.md".into() });
    assert_eq!(t3.repo_id, web_id);

    // Planes copiados a `<data>/tasks/<id>/plan.md`.
    assert_eq!(
        fs::read(data.0.join("tasks/t01aaaaaaaa/plan.md")).unwrap(),
        fs::read(fixture("legacy/tasks/t01aaaaaaaa/plan.md")).unwrap()
    );
    assert!(data.0.join("tasks/t02bbbbbbbb/plan.md").is_file());

    // Runs.
    let r = run_by_label(&conn, "Agregar endpoint de salud");
    assert_eq!(r.task_id.as_deref(), Some("t01aaaaaaaa"));
    assert_eq!((r.status, r.outcome), (RunStatus::Finished, None));
    assert_eq!(r.executor, Executor::Workflow { name: "plan-task".into() });
    assert_eq!((r.finish, r.claude_run_id.as_deref()), (Finish::Commit, Some("a1b2c3d4")));

    let q = run_by_label(&conn, "Limpiar scripts de deploy");
    assert_eq!((q.status, q.error.as_deref()), (RunStatus::Queued, None));
    assert!(crate::work::queue::awaiting_confirmation(&q), "no se lanza sin confirmar");

    let orphan = run_by_label(&conn, "t09zzzzzzzz");
    assert_eq!(orphan.task_id, None);
    assert_eq!(orphan.repo_id.as_deref(), Some(api.id.as_str()));
    assert_eq!(orphan.executor, Executor::Claude);
    assert_eq!((orphan.status, orphan.error.as_deref()), (RunStatus::Failed, Some("claude exited with code 1")));

    let launching = run_by_label(&conn, "Rebrand del header");
    assert_eq!((launching.status, launching.error.as_deref()), (RunStatus::Failed, Some(LAUNCHING_ERROR)));

    let eng1 = run_by_label(&conn, "ENG-1");
    assert_eq!(eng1.task_id, None);
    assert_eq!(eng1.repo_id.as_deref(), Some(api.id.as_str()));
    assert_eq!(eng1.prompt, "/linear-issue ENG-1");
    assert_eq!(eng1.options.model.as_deref(), Some("opus"));
    assert_eq!(eng1.finish, Finish::Commit, "el finish sale del repo mapeado");
    let eng2 = run_by_label(&conn, "ENG-2");
    assert_eq!((eng2.repo_id.as_deref(), eng2.status), (None, RunStatus::Queued));

    // Lo que quedó en cola espera confirmación: ninguno sin `legacy_label`.
    let unconfirmed: i64 = conn
        .query_row("SELECT COUNT(*) FROM runs WHERE status = 'queued' AND legacy_label IS NULL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(unconfirmed, 0);
    assert!(crate::work::queue::awaiting_confirmation(&eng2));

    assert_eq!(rows::load_settings(&conn).unwrap().concurrency, 5);
}

#[test]
fn source_folder_stays_intact_and_backup_is_a_full_copy() {
    // Se importa desde una copia en temp, para poder comparar metadatos sin tocar el repo.
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

    let db = open_in_memory().unwrap();
    let report = import_folder(&mut db.lock().unwrap(), &src.0, &data.0, T0).unwrap();

    assert_eq!(snapshot(&src.0), before, "el origen no cambia byte a byte");
    assert_eq!(mtimes(&src.0), mtimes_before);
    let backup = PathBuf::from(&report.backup_dir);
    assert_eq!(backup.parent().unwrap(), data.0);
    assert!(backup.file_name().unwrap().to_string_lossy().starts_with(BACKUP_PREFIX));
    assert_eq!(snapshot(&backup), before, "el backup es una copia exacta");
}

#[test]
fn importing_the_data_folder_itself_skips_backups_and_db() {
    let data = TempDir::new("self");
    copy_tree(&fixture("legacy"), &data.0);
    fs::write(data.0.join("nodal.db"), b"sqlite").unwrap();
    fs::create_dir(data.0.join("legacy-backup-1")).unwrap();
    fs::write(data.0.join("legacy-backup-1/old.json"), b"{}").unwrap();

    let db = open_in_memory().unwrap();
    let report = import_folder(&mut db.lock().unwrap(), &data.0, &data.0, T0).unwrap();
    let copied = snapshot(Path::new(&report.backup_dir));
    assert!(copied.contains_key("tasks.json"));
    assert!(!copied.keys().any(|k| k.starts_with("nodal.db") || k.starts_with(BACKUP_PREFIX)));
    assert_eq!(report.tasks, 3);
}

#[test]
fn corrupt_files_are_skipped_without_aborting() {
    let data = TempDir::new("corrupt");
    let db = open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    let r = import_folder(&mut conn, &fixture("corrupt"), &data.0, T0).unwrap();
    assert_eq!((r.projects, r.repos, r.tasks, r.runs), (1, 1, 0, 1), "{r:#?}");
    assert!(r.skipped.iter().any(|s| s.starts_with("tasks.json: corrupt JSON")), "{:?}", r.skipped);
    assert!(r.skipped.iter().any(|s| s.starts_with("task-runs.json: corrupt JSON")), "{:?}", r.skipped);
    assert_eq!(rows::load_settings(&conn).unwrap().concurrency, MAX_CONCURRENCY, "se acota");
    let eng1 = run_by_label(&conn, "ENG-1");
    assert_eq!((eng1.status, eng1.error.as_deref()), (RunStatus::Failed, Some(LAUNCHING_ERROR)));
}

#[test]
fn current_config_format_with_links() {
    let data = TempDir::new("current");
    let db = open_in_memory().unwrap();
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
    assert_eq!(rows::load_settings(&conn).unwrap().concurrency, 2);

    let again = import_folder(&mut conn, &fixture("current"), &data.0, T0 + 5).unwrap();
    assert_eq!((again.projects, again.repos, again.already_imported), (0, 0, 4));
}

#[test]
fn existing_repo_is_reused_and_keys_do_not_collide() {
    let data = TempDir::new("existing");
    let db = open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    // El usuario ya creó un proyecto con el repo web y otro que usa la key "AA".
    for (id, key) in [("mine", "WEB"), ("other", "AA")] {
        rows::insert_project(
            &conn,
            &Project {
                id: id.into(),
                name: id.into(),
                key: key.into(),
                next_task_number: 1,
                color: "#fff".into(),
                default_executor: None,
                reviewer: None,
                created_at: 1,
                archived_at: None,
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
            default_executor: None,
            default_isolation: Isolation::InPlace,
            default_finish: Finish::Changes,
            default_review: false,
            reviewer: None,
            position: 0,
            created_at: 1,
        },
    )
    .unwrap();

    let r = import_folder(&mut conn, &fixture("legacy"), &data.0, T0).unwrap();
    assert_eq!((r.projects, r.repos), (3, 3), "acme-web ya existía");
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
    // El repo del usuario no se toca.
    assert_eq!(rows::get_repo(&conn, "web").unwrap().unwrap().default_finish, Finish::Changes);
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
    assert_eq!(executor_from_prompt("/plan-task {}"), Executor::Workflow { name: "plan-task".into() });
    assert_eq!(executor_from_prompt("hola"), Executor::Claude);
    assert!(map_run_status("weird", None).is_err());
}

#[test]
fn reimport_after_state_changes_and_duplicate_entries() {
    let src = TempDir::new("changing");
    let data = TempDir::new("changing-data");
    // Mismo path en dos entradas (team y proyecto): un repo, dos links, nada "ya importado".
    fs::write(
        src.0.join("config.json"),
        r#"{"repos":[{"teamId":"team-a","path":"/Users/me/Code/acme-one"},
                     {"projectId":"proj-b","path":"/Users/me/Code/acme-one/"}]}"#,
    )
    .unwrap();
    let queued = r#"{"runs":[{"issueId":"i1","identifier":"ENG-9","workflow":"linear-issue",
        "cwd":"/Users/me/Code/acme-one","queuedAt":10,"status":"queued"}]}"#;
    fs::write(src.0.join("issue-runs.json"), queued).unwrap();

    let db = open_in_memory().unwrap();
    let mut conn = db.lock().unwrap();
    let r1 = import_folder(&mut conn, &src.0, &data.0, T0).unwrap();
    assert_eq!((r1.projects, r1.repos, r1.runs, r1.already_imported), (1, 1, 1, 0), "{r1:#?}");
    assert_eq!(count(&conn, "source_links"), 2);

    // La versión vieja lanzó el run después: ahora tiene runId. No se duplica.
    let launched = queued.replace(r#""status":"queued""#, r#""status":"launched","runId":"abcd1234""#);
    fs::write(src.0.join("issue-runs.json"), launched).unwrap();
    let r2 = import_folder(&mut conn, &src.0, &data.0, T0 + 1).unwrap();
    // Por registro: 2 entradas de repo + 2 links + 1 run.
    assert_eq!((r2.runs, r2.already_imported), (0, 5), "{r2:#?}");
    assert_eq!(count(&conn, "runs"), 1);
}

#[test]
fn nested_data_dir_is_filtered_at_any_level() {
    let src = TempDir::new("nested");
    let data = src.0.join("app/data");
    fs::create_dir_all(data.join("legacy-backup-1")).unwrap();
    fs::write(data.join("nodal.db"), b"x").unwrap();
    fs::write(data.join("tasks.json"), b"{\"tasks\":[]}").unwrap();
    let db = open_in_memory().unwrap();
    let r = import_folder(&mut db.lock().unwrap(), &src.0, &data, T0).unwrap();
    let copied = snapshot(Path::new(&r.backup_dir));
    assert_eq!(copied.keys().collect::<Vec<_>>(), ["app/data/tasks.json"]);
}
