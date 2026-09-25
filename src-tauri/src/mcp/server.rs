//! The app side: a Unix socket in the data folder that runs tool requests.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use super::{tools, Reply, Request, MAX_LINE};
use crate::events::Kind;
use crate::util::{now_ms, paths};
use crate::work::Inner;

/// Listens on `<data_dir>/mcp.sock` in a background thread. Fails if another Nodal with the
/// same data folder is already listening.
pub fn start(inner: Arc<Inner>) -> Result<PathBuf, String> {
    let path = paths::mcp_socket(&inner.env.data_dir);
    let listener = bind_private(&path)?;
    thread::Builder::new()
        .name("mcp".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("mcp: {e}");
                        continue;
                    }
                };
                let inner = inner.clone();
                let spawned = thread::Builder::new().name("mcp-conn".into()).spawn(move || {
                    if let Err(e) = handle_conn(stream, |r| respond(&inner, r)) {
                        eprintln!("mcp: {e}");
                    }
                });
                if let Err(e) = spawned {
                    eprintln!("mcp: {e}");
                }
            }
        })
        .map_err(|e| format!("Couldn't start the MCP server: {e}"))?;
    Ok(path)
}

/// One request: the same `ops` as the UI with the database locked, then the same
/// `nodal://changed` the Tauri commands emit.
fn respond(inner: &Inner, req: Request) -> Reply {
    let out = {
        let mut conn = inner.db.lock().unwrap_or_else(|p| p.into_inner());
        tools::call(&mut conn, &inner.env, &req.tool, req.arguments, now_ms())
    };
    match out {
        Ok(o) => {
            if let Some(project) = &o.changed {
                inner.events.notify(Kind::Tasks, Some(project));
            }
            Reply::Result(o.value)
        }
        Err(e) => Reply::Error(e),
    }
}

fn handle_conn(stream: UnixStream, mut respond: impl FnMut(Request) -> Reply) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    loop {
        let mut line = String::new();
        let n = (&mut reader).take(MAX_LINE + 1).read_line(&mut line)?;
        if n == 0 {
            return Ok(());
        }
        let too_large = n as u64 > MAX_LINE;
        let reply = if too_large {
            Reply::Error("The request is too large.".into())
        } else if line.trim().is_empty() {
            continue;
        } else {
            match serde_json::from_str::<Request>(&line) {
                Ok(r) => respond(r),
                Err(e) => Reply::Error(format!("Invalid request: {e}")),
            }
        };
        let mut out = serde_json::to_vec(&reply)?;
        out.push(b'\n');
        writer.write_all(&out)?;
        if too_large {
            return Ok(());
        }
    }
}

/// Binds `path` readable only by the user. The socket is created inside a `0700` folder and
/// moved into place once it is `0600`, so it is never reachable with looser permissions.
fn bind_private(path: &Path) -> Result<UnixListener, String> {
    let fail = |e: io::Error| format!("Couldn't open the MCP socket at {}: {e}", path.display());
    let dir = path.parent().ok_or_else(|| format!("Invalid socket path: {}", path.display()))?;
    fs::create_dir_all(dir).map_err(fail)?;
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_socket() => {
            if UnixStream::connect(path).is_ok() {
                return Err(format!("Another Nodal is already listening on {}.", path.display()));
            }
            fs::remove_file(path).map_err(fail)?;
        }
        Ok(_) => return Err(format!("{} exists and is not a socket.", path.display())),
        Err(_) => {}
    }
    let staging = dir.join(format!(".mcp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::DirBuilder::new().mode(0o700).create(&staging).map_err(fail)?;
    let bound = (|| {
        let tmp = staging.join("s");
        let listener = UnixListener::bind(&tmp)?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        fs::rename(&tmp, path)?;
        Ok(listener)
    })();
    let _ = fs::remove_dir_all(&staging);
    bound.map_err(fail)
}

#[cfg(test)]
mod tests {
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
            let p = ops::create_project(&c, &NewProject { name: "Pay".into(), key: "PAY".into(), color: None, description: None }, 1).unwrap();
            ops::add_repo(&c, &p.id, &NewRepo::default(), &repo_dir, 2).unwrap();
        }
        let socket = paths::mcp_socket(&inner.env.data_dir);
        assert_eq!(forward(&socket, "list_projects", json!({})).unwrap_err(), OPEN_NODAL);

        assert_eq!(start(inner.clone()).unwrap(), socket);
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
}
