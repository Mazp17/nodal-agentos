//! The app side: a Unix socket in the data folder that runs tool requests.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nodal_app::App;
use nodal_mcp_proto::paths::mcp_socket;
use nodal_mcp_proto::protocol::{Reply, Request, MAX_LINE};
use tokio::net::UnixListener as TokioUnixListener;

/// Backoff after a genuine `accept` error (e.g. out of file descriptors). Idle waiting for
/// the next connection costs nothing: `accept().await` parks on the OS reactor (kqueue on
/// macOS) instead of polling, so there is no wakeup to back off from in the common case.
const ERROR_BACKOFF: Duration = Duration::from_millis(100);

/// A running listener. Dropping it keeps the task alive; `stop` aborts it.
#[derive(Debug)]
pub struct Listening {
    path: PathBuf,
    /// `(dev, ino)` of the socket we bound, so `stop` never deletes another instance's.
    id: (u64, u64),
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Listening {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stops accepting connections and removes the socket. Requests already being served
    /// finish on their own threads.
    pub fn stop(self) {
        // Removed while the listener is still open: nobody else can have taken the path yet.
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.id) {
            let _ = fs::remove_file(&self.path);
        }
        self.task.abort();
    }
}

/// Listens on `<data_dir>/mcp.sock` in a background task. Fails if another Nodal with the
/// same data folder is already listening.
pub fn start(app: Arc<App>) -> Result<Listening, String> {
    let path = mcp_socket(app.agent_api.data_dir());
    let listener = bind_private(&path)?;
    let fail = |e: io::Error| format!("Couldn't start the MCP server: {e}");
    let id = fs::symlink_metadata(&path)
        .map(|m| (m.dev(), m.ino()))
        .map_err(fail)?;
    listener.set_nonblocking(true).map_err(fail)?;
    // `from_std` registers the fd with the reactor right away, so it needs a runtime in
    // scope even though `start` itself is a plain, synchronous call.
    let handle = tauri::async_runtime::handle();
    let listener = {
        let _enter = handle.inner().enter();
        TokioUnixListener::from_std(listener).map_err(fail)?
    };
    let task = tauri::async_runtime::spawn(async move {
        loop {
            let stream = match listener.accept().await {
                Ok((s, _)) => s,
                Err(e) => {
                    eprintln!("mcp: {e}");
                    tokio::time::sleep(ERROR_BACKOFF).await;
                    continue;
                }
            };
            let stream = match stream.into_std() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("mcp: {e}");
                    continue;
                }
            };
            // Accepted sockets inherit non-blocking mode; the connection handler below does
            // synchronous, blocking I/O on its own thread.
            if let Err(e) = stream.set_nonblocking(false) {
                eprintln!("mcp: {e}");
                continue;
            }
            let app = app.clone();
            let spawned = thread::Builder::new()
                .name("mcp-conn".into())
                .spawn(move || {
                    if let Err(e) = handle_conn(stream, |r| respond(&app, r)) {
                        eprintln!("mcp: {e}");
                    }
                });
            if let Err(e) = spawned {
                eprintln!("mcp: {e}");
            }
        }
    });
    Ok(Listening { path, id, task })
}

/// One request: `AgentApi::call` runs the same `ops` as the UI with the database locked, then
/// emits the same `nodal://changed` the Tauri commands emit.
fn respond(app: &App, req: Request) -> Reply {
    match app.agent_api.call(&req.tool, req.arguments) {
        Ok(value) => Reply::Result(value),
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
    let dir = path
        .parent()
        .ok_or_else(|| format!("Invalid socket path: {}", path.display()))?;
    fs::create_dir_all(dir).map_err(fail)?;
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_socket() => {
            if UnixStream::connect(path).is_ok() {
                return Err(format!(
                    "Another Nodal is already listening on {}.",
                    path.display()
                ));
            }
            fs::remove_file(path).map_err(fail)?;
        }
        Ok(_) => return Err(format!("{} exists and is not a socket.", path.display())),
        Err(_) => {}
    }
    let staging = dir.join(format!(".mcp-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&staging)
        .map_err(fail)?;
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
mod tests;
