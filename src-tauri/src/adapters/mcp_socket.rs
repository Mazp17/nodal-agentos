//! The app side: a Unix socket in the data folder (a named pipe on Windows) that runs tool
//! requests.

#[cfg(unix)]
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nodal_app::App;
use nodal_mcp_proto::paths::mcp_socket;
use nodal_mcp_proto::protocol::{Reply, Request, MAX_LINE};
#[cfg(unix)]
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
    #[cfg(unix)]
    id: (u64, u64),
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Listening {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stops accepting connections and removes the socket (a pipe has no file to remove).
    /// Requests already being served finish on their own threads.
    pub fn stop(self) {
        // Removed while the listener is still open: nobody else can have taken the path yet.
        #[cfg(unix)]
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.id) {
            let _ = fs::remove_file(&self.path);
        }
        self.task.abort();
    }
}

/// Listens on `<data_dir>/mcp.sock` in a background task. Fails if another Nodal with the
/// same data folder is already listening.
#[cfg(unix)]
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
                    let served = stream
                        .try_clone()
                        .and_then(|r| handle_conn(r, &stream, |r| respond(&app, r)));
                    if let Err(e) = served {
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

/// Listens on the named pipe derived from the data folder. Fails if another Nodal with the
/// same data folder is already listening.
#[cfg(windows)]
pub fn start(app: Arc<App>) -> Result<Listening, String> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let path = mcp_socket(app.agent_api.data_dir());
    let pipe = path.clone();
    let make = move |first: bool| {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create(&pipe)
    };
    let rt = tauri::async_runtime::handle().inner().clone();
    // Creating the pipe registers it with the reactor, so it needs the runtime in scope.
    let first = {
        let _enter = rt.enter();
        make(true)
    };
    let mut server = first.map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            format!("Another Nodal is already listening on {}.", path.display())
        } else {
            format!("Couldn't start the MCP server: {e}")
        }
    })?;
    let listening_path = path.clone();
    let task = tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = server.connect().await {
                eprintln!("mcp: {e}");
                tokio::time::sleep(ERROR_BACKOFF).await;
                continue;
            }
            // The next instance must exist before this client is handed off, or a second
            // client would find no pipe to open.
            let next = loop {
                match make(false) {
                    Ok(s) => break s,
                    Err(e) => {
                        eprintln!("mcp: {e}");
                        tokio::time::sleep(ERROR_BACKOFF).await;
                    }
                }
            };
            let conn = std::mem::replace(&mut server, next);
            let (app, rt) = (app.clone(), rt.clone());
            let spawned = thread::Builder::new()
                .name("mcp-conn".into())
                .spawn(move || {
                    let (r, w) = tokio::io::split(conn);
                    let served =
                        handle_conn(SyncIo(r, rt.clone()), SyncIo(w, rt), |r| respond(&app, r));
                    // The client drops the pipe after its one reply: not worth a log line.
                    if let Err(e) = served {
                        if e.kind() != io::ErrorKind::BrokenPipe {
                            eprintln!("mcp: {e}");
                        }
                    }
                });
            if let Err(e) = spawned {
                eprintln!("mcp: {e}");
            }
        }
    });
    Ok(Listening {
        path: listening_path,
        task,
    })
}

/// Blocking `Read`/`Write` over an async pipe half, for `handle_conn` on its own thread.
#[cfg(windows)]
struct SyncIo<T>(T, tokio::runtime::Handle);

#[cfg(windows)]
impl<T: tokio::io::AsyncRead + Unpin> Read for SyncIo<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use tokio::io::AsyncReadExt;
        self.1.block_on(self.0.read(buf))
    }
}

#[cfg(windows)]
impl<T: tokio::io::AsyncWrite + Unpin> Write for SyncIo<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        use tokio::io::AsyncWriteExt;
        self.1.block_on(self.0.write(buf))
    }
    fn flush(&mut self) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        self.1.block_on(self.0.flush())
    }
}

/// One request: `AgentApi::call` runs the same `ops` as the UI with the database locked, then
/// emits the same `nodal://changed` the Tauri commands emit.
fn respond(app: &App, req: Request) -> Reply {
    match app.agent_api.call(&req.tool, req.arguments) {
        Ok(value) => Reply::Result(value),
        Err(e) => Reply::Error(e),
    }
}

fn handle_conn(
    reader: impl Read,
    mut writer: impl Write,
    mut respond: impl FnMut(Request) -> Reply,
) -> io::Result<()> {
    let mut reader = BufReader::new(reader);
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
#[cfg(unix)]
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
