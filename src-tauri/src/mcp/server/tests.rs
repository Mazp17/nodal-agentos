use super::*;
use crate::db::open_in_memory;
use crate::db::queries::projects;
use crate::mcp::stdio::{forward, OPEN_NODAL};
use crate::work::dto::{NewProject, NewRepo};
use crate::work::{ops, Cleaning, Env};
use serde_json::json;
use std::sync::Mutex;

/// Short names: a socket path is limited to 104 bytes on macOS.
fn inner(root: &Path) -> Arc<Inner> {
    let env = Env { data_dir: root.join("d"), worktrees_root: root.join("w"), claude_dir: None };
    Arc::new(Inner {
        db: open_in_memory().unwrap(),
        env,
        pump: tokio::sync::Mutex::new(()),
        events: Default::default(),
        pump_error: Mutex::new(None),
        cleaning: Cleaning::default(),
    })
}

#[test]
fn socket_is_private_and_serves_the_same_ops_as_the_ui() {
    let t = crate::util::paths::tests::TempDir::new("mcps");
    let inner = inner(&t.0);
    let repo_dir = t.0.join("web");
    fs::create_dir_all(&repo_dir).unwrap();
    {
        let c = inner.db.lock().unwrap();
        let p = ops::create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None, description: None, root_path: None }, 1).unwrap();
        ops::add_repo(&c, &p.id, &NewRepo::default(), &repo_dir, 2).unwrap();
    }
    let socket = paths::mcp_socket(&inner.env.data_dir);
    assert_eq!(forward(&socket, "list_projects", json!({})).unwrap_err(), OPEN_NODAL);

    let listening = start(inner.clone()).unwrap();
    assert_eq!(listening.path(), socket);
    let mode = fs::metadata(&socket).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert!(fs::read_dir(&inner.env.data_dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".mcp-")));

    let created = forward(&socket, "create_task", json!({"repo": "web", "title": "From an agent", "plan": "# Plan"})).unwrap();
    assert_eq!(created["key"], "PAY-1");
    let listed = forward(&socket, "list_tasks", json!({"status": "todo"})).unwrap();
    assert_eq!(listed[0]["title"], "From an agent");
    let err = forward(&socket, "update_task", json!({"task": "PAY-1", "title": " "})).unwrap_err();
    assert_eq!(err, "The title is empty.");
    assert_eq!(projects::list(&inner.db.lock().unwrap(), false).unwrap()[0].next_task_number, 2);

    // Several requests on one connection, and garbage gets an error instead of a hang.
    let mut s = UnixStream::connect(&socket).unwrap();
    s.write_all(b"not json\n{\"tool\":\"list_projects\"}\n").unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let lines: Vec<Reply> =
        BufReader::new(s).lines().map(|l| serde_json::from_str(&l.unwrap()).unwrap()).collect();
    assert!(matches!(&lines[0], Reply::Error(e) if e.starts_with("Invalid request")));
    assert!(matches!(&lines[1], Reply::Result(v) if v[0]["key"] == "PAY"));

    // A second app with the same data folder does not steal the socket.
    assert!(start(inner.clone()).unwrap_err().contains("already listening"));

    // Stopped: the socket is gone and agents are told to open Nodal; it can start again.
    listening.stop();
    assert!(!socket.exists());
    assert_eq!(forward(&socket, "list_projects", json!({})).unwrap_err(), OPEN_NODAL);
    let again = start(inner.clone()).unwrap();
    assert_eq!(forward(&socket, "list_projects", json!({})).unwrap()[0]["key"], "PAY");

    // Someone else's socket at the same path survives our stop.
    fs::remove_file(&socket).unwrap();
    let other = UnixListener::bind(&socket).unwrap();
    again.stop();
    assert!(socket.exists());
    drop(other);
}

#[test]
fn stale_socket_is_replaced_and_other_files_are_kept() {
    let t = crate::util::paths::tests::TempDir::new("mcpst");
    let path = t.0.join("s.sock");
    drop(UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    let l = bind_private(&path).unwrap();
    assert!(UnixStream::connect(&path).is_ok());
    drop(l);

    let file = t.0.join("f.sock");
    fs::write(&file, "x").unwrap();
    assert!(bind_private(&file).unwrap_err().contains("not a socket"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "x");
}
