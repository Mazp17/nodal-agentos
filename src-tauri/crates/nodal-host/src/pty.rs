//! Embedded terminal for "Attach": a PTY per background session, driven by `claude attach
//! <id>`, so the frontend can render the real TUI with xterm.js instead of opening Terminal.app.
//!
//! Detaching doesn't stop the background session: verified against `claude` 2.1.283, sending
//! the client either `SIGHUP` (closing the pty's master fd) or `SIGKILL` only detaches, the
//! session keeps running. `Session::detach` relies on that.
//!
//! Each session has three dedicated `std::thread`s (`portable_pty` is synchronous, so none of
//! this can run on the async runtime): a reader that forwards raw chunks and, on EOF, waits for
//! the child and reports its exit code; a coalescer that batches those chunks for `on_data`
//! (~8 ms or 64 KiB, whichever comes first, so a burst of small writes doesn't turn into one
//! IPC round trip per write); and a writer that owns a `dup(2)`'d handle to the pty's write
//! side (see `open_writer`) so a write never blocks the command it's called from.
//!
//! The shell wraps Tauri's `Channel`/`Webview` types into the plain callbacks `attach` takes
//! here, so this crate has no tauri dependency.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::claude::bin as claude_bin;

/// Environment markers of the terminal/multiplexer/IDE/shell the app happens to have been
/// launched from: left as inherited, they could make `claude attach`'s TUI misjudge the
/// terminal (or its size) it's actually drawing into. The pty's own size (set at open time,
/// and kept current via `resize`) is the only source of truth for `COLUMNS`/`LINES`.
const ENV_VARS_TO_REMOVE: &[&str] = &[
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "ITERM_SESSION_ID",
    "ITERM_PROFILE",
    "TMUX",
    "TMUX_PANE",
    "STY",
    "CLAUDECODE",
    "COLUMNS",
    "LINES",
];

/// How long the coalescer waits, after the first byte of a batch, before flushing to `on_data`.
const FLUSH_INTERVAL: Duration = Duration::from_millis(8);
/// Flush a batch early if it reaches this size, regardless of `FLUSH_INTERVAL`.
const FLUSH_MAX_BYTES: usize = 64 * 1024;
/// Size of the buffer used for each individual read from the pty.
const READ_BUF_SIZE: usize = 16 * 1024;
/// How long `Session::detach` waits, after `SIGHUP`, before escalating to `SIGKILL`.
const KILL_GRACE_PERIOD: Duration = Duration::from_millis(500);

const UNKNOWN_SESSION: &str = "Unknown terminal session.";

/// Delivers raw chunks read from the pty, batched by the coalescer.
pub type OnData = Box<dyn Fn(Vec<u8>) + Send>;
/// Delivers the child's exit code (`None` if it couldn't be determined) once.
pub type OnExit = Box<dyn FnOnce(Option<i32>) + Send>;

/// One `claude attach` session: everything needed to write to it, resize it and detach it.
/// The reader thread owns the `Read` half and the `Child` directly; they're not in here.
struct Session {
    /// Label of the webview that opened this session, so a reload can close only its own.
    owner: String,
    writer_tx: mpsc::Sender<Vec<u8>>,
    /// Sends `SIGHUP` (see the module docs); the background session survives.
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// For `detach`'s `SIGKILL` follow-up. `None` if the platform couldn't report it, in which
    /// case detaching relies on `SIGHUP` alone.
    pid: Option<u32>,
    /// Set by the reader thread, under this same lock, right before it calls `wait()`.
    /// `detach`'s watchdog locks this across its check-then-`SIGKILL` so the two can't
    /// interleave (see `detach`'s doc comment for why that matters).
    exited: Arc<Mutex<bool>>,
    /// Kept only for `resize`; the reader and writer threads have their own clones/handles.
    master: Box<dyn MasterPty + Send>,
}

impl Session {
    /// Detaches `claude attach` from the background session: `SIGHUP` right away (verified in
    /// the spike to be enough for a cooperating client), then, on its own thread, a hard
    /// `SIGKILL` after `KILL_GRACE_PERIOD` if the reader still hasn't seen EOF by then.
    ///
    /// The watchdog holds `exited`'s lock across both the check and the `kill(2)` call, and the
    /// reader thread sets it, under that same lock, right before its own `wait()`. Without that,
    /// a plain flag would leave a gap: the watchdog could read "still running" and then, right
    /// as it calls `kill`, lose a race with the reader reaping the child — signaling a pid the
    /// OS may since have recycled for an unrelated process. With the shared lock, whichever
    /// side gets there first forces the other to wait, so `wait()` can never happen while a
    /// `kill` decision is in flight. The rest of the session (pty fds, writer thread) is simply
    /// dropped.
    fn detach(mut self) {
        let _ = self.killer.kill();
        let Some(pid) = self.pid else { return };
        let exited = self.exited;
        // `Builder::spawn` (unlike `thread::spawn`) reports a failure to create the thread
        // instead of panicking; losing the `SIGKILL` follow-up in that case is an acceptable
        // degradation (the `SIGHUP` above already went out).
        let _ = thread::Builder::new().spawn(move || {
            thread::sleep(KILL_GRACE_PERIOD);
            let exited = exited.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if !*exited {
                #[cfg(unix)]
                // Safety: `pid` is a plain process id; `kill` fails harmlessly (`ESRCH`) if the
                // process is already gone.
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGKILL);
                }
            }
        });
    }
}

#[derive(Default)]
struct SessionsState {
    sessions: HashMap<u32, Session>,
    /// Bumped once per webview label every time `close_owned_by` runs for it (i.e. every full
    /// reload of that webview). See `PtySessions::insert_current`.
    generations: HashMap<String, u64>,
}

/// Sessions currently attached, keyed by the id handed to the frontend. Cheap to clone (an
/// `Arc` underneath): command handlers and the reader threads each hold their own handle.
#[derive(Clone, Default)]
pub struct PtySessions {
    next_id: Arc<AtomicU32>,
    state: Arc<Mutex<SessionsState>>,
}

impl PtySessions {
    /// Reserves the next session id (starting at 1: some frontend state might otherwise treat
    /// 0 as "no session"). Errors instead of silently wrapping around to 0 (which would then
    /// collide with that "no session" reading, or with id 0 if it were ever reused) if
    /// `u32::MAX` sessions have been attached in this run — astronomically unlikely, but cheap
    /// to rule out rather than assume.
    fn next_id(&self) -> Result<u32, String> {
        // A compare-exchange loop: `fetch_update` is deprecated on newer toolchains and its
        // replacement (`try_update`) doesn't exist on older ones.
        let mut current = self.next_id.load(Ordering::Relaxed);
        loop {
            let next = current
                .checked_add(1)
                .ok_or_else(|| "Ran out of terminal session ids.".to_string())?;
            match self.next_id.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(next),
                Err(actual) => current = actual,
            }
        }
    }

    /// A poisoned lock (only possible after a panic while holding it) shouldn't wedge every
    /// session forever: recover the data and move on.
    fn lock(&self) -> MutexGuard<'_, SessionsState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Current reload generation of webview `owner` (0 if it never reloaded).
    pub fn generation_for(&self, owner: &str) -> u64 {
        self.lock().generations.get(owner).copied().unwrap_or(0)
    }

    /// Inserts `session` under `id` (already reserved via `next_id`), unless its owner has
    /// moved on to a newer generation than `generation` (captured by the caller when it
    /// started attaching): a full reload can land between that capture and the pty finishing
    /// setup, and `close_owned_by` would have run too early to know about this session,
    /// leaking it. Detaches (and doesn't insert) `session` if so. Returns whether it was
    /// inserted.
    ///
    /// This doesn't catch every case: if the old page's attach *call* is only dispatched
    /// after `close_owned_by` already ran for this reload (say, one queued right as the page
    /// navigates away), it captures the *new* generation and this check lets it through. That
    /// residual window is accepted — closing it would mean invalidating in-flight IPC calls,
    /// which Tauri doesn't expose — and the frontend already closes its own sessions before it
    /// navigates away on purpose.
    fn insert_current(&self, id: u32, generation: u64, session: Session) -> bool {
        let mut state = self.lock();
        if state.generations.get(&session.owner).copied().unwrap_or(0) == generation {
            state.sessions.insert(id, session);
            return true;
        }
        drop(state); // `detach` must not run while the lock is held.
        session.detach();
        false
    }

    fn remove(&self, id: u32) -> Option<Session> {
        self.lock().sessions.remove(&id)
    }

    /// Hands `data` to the session's writer thread; never blocks on the actual write.
    pub fn write(&self, id: u32, data: Vec<u8>) -> Result<(), String> {
        // Taken before the match (not in its scrutinee) so the lock is released here instead
        // of being held for the match's whole body.
        let tx = self.lock().sessions.get(&id).map(|s| s.writer_tx.clone());
        match tx {
            Some(tx) => {
                // A send failure means the writer thread already ended (the session is gone or
                // going): nothing left to write to, but not the caller's problem.
                let _ = tx.send(data);
                Ok(())
            }
            None => Err(UNKNOWN_SESSION.into()),
        }
    }

    /// Resizes the pty; `claude attach`'s TUI redraws on `SIGWINCH`, which this triggers.
    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let state = self.lock();
        let session = state.sessions.get(&id).ok_or(UNKNOWN_SESSION)?;
        session
            .master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("Couldn't resize the terminal: {e}"))
    }

    /// Detaches a session (idempotent: `Ok` even if `id` is unknown, e.g. it already exited).
    /// This only detaches `claude attach`; the background session survives (see the module
    /// docs).
    pub fn close(&self, id: u32) {
        if let Some(session) = self.remove(id) {
            session.detach();
        }
    }

    /// Detaches every session opened by webview `owner`: used when that webview does a full
    /// reload (not Vite's HMR in dev, which patches modules without a full page load and so
    /// doesn't trigger this), which today would leave `claude attach` processes running with
    /// nobody reading their output. Also bumps `owner`'s generation, so an attach already in
    /// flight for it gets discarded instead of inserted once it finishes (`insert_current`).
    pub fn close_owned_by(&self, owner: &str) {
        let removed: Vec<Session> = {
            let mut state = self.lock();
            *state.generations.entry(owner.to_string()).or_insert(0) += 1;
            let ids: Vec<u32> = state.sessions.iter().filter(|(_, s)| s.owner == owner).map(|(id, _)| *id).collect();
            ids.into_iter().filter_map(|id| state.sessions.remove(&id)).collect()
        };
        for session in removed {
            session.detach();
        }
    }

    /// Detaches every remaining session: used on app exit.
    pub fn close_all(&self) {
        let removed: Vec<Session> = {
            let mut state = self.lock();
            state.sessions.drain().map(|(_, s)| s).collect()
        };
        for session in removed {
            session.detach();
        }
    }

    /// Attaches to a background session: opens a pty of `cols`×`rows`, spawns `claude attach
    /// <run_id>` into it and streams its output through `on_data` until it exits (reported
    /// once on `on_exit`). The initial size must be right (it can't be corrected by an
    /// immediate resize: verified in the spike, the TUI draws its first frame at whatever size
    /// the pty had at spawn).
    #[allow(clippy::too_many_arguments)]
    pub fn attach(
        &self,
        owner: String,
        generation: u64,
        run_id: String,
        cwd: Option<String>,
        cols: u16,
        rows: u16,
        on_data: OnData,
        on_exit: OnExit,
    ) -> Result<u32, String> {
        let claude = claude_bin::resolve_claude()?;
        let args = vec!["attach".to_string(), run_id];
        let req = AttachRequest { owner, generation, claude, args, cwd, cols, rows };
        spawn_session(self.clone(), req, on_data, on_exit)
    }

    /// Starts an interactive `claude` in `dir` (an existing directory): used to accept the
    /// workspace trust dialog of a repo from inside the app. Closing the session ends that
    /// `claude` (unlike `attach`, nothing keeps running in the background).
    #[allow(clippy::too_many_arguments)]
    pub fn open_claude(
        &self,
        owner: String,
        generation: u64,
        dir: String,
        cols: u16,
        rows: u16,
        on_data: OnData,
        on_exit: OnExit,
    ) -> Result<u32, String> {
        let path = Path::new(&dir);
        if !path.is_absolute() || !path.is_dir() {
            return Err(format!("The folder doesn't exist or isn't a directory: {dir}"));
        }
        let claude = claude_bin::resolve_claude()?;
        let req = AttachRequest { owner, generation, claude, args: Vec::new(), cwd: Some(dir), cols, rows };
        spawn_session(self.clone(), req, on_data, on_exit)
    }
}

/// Whether `value` (an `LC_ALL`/`LC_CTYPE`/`LANG`-shaped locale string) looks like a UTF-8
/// locale, e.g. `en_US.UTF-8` or `C.UTF-8`.
fn looks_like_utf8_locale(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    upper.contains("UTF-8") || upper.contains("UTF8")
}

/// Whether to fall back to `LANG=en_US.UTF-8`: the effective locale category for character
/// classification is whichever of `LC_ALL`, `LC_CTYPE`, `LANG` is set first, in that order
/// (glibc's and macOS's own precedence for `LC_CTYPE`, which is what decides whether
/// box-drawing characters render); this falls back if that's unset, blank, `C`/`POSIX`, or
/// otherwise doesn't look like a UTF-8 locale. `get_var` is injected so this stays a pure,
/// cheaply testable function.
fn needs_default_locale(get_var: impl Fn(&str) -> Option<String>) -> bool {
    let effective = ["LC_ALL", "LC_CTYPE", "LANG"].iter().find_map(|k| get_var(k)).filter(|v| !v.trim().is_empty());
    match effective {
        None => true,
        Some(v) => {
            let v = v.trim();
            v.eq_ignore_ascii_case("C") || v.eq_ignore_ascii_case("POSIX") || !looks_like_utf8_locale(v)
        }
    }
}

/// Child environment: an extended `PATH` (so `claude` finds git/node/etc even when the app was
/// opened from Finder), a real terminal type, and none of the multiplexer/IDE/shell markers
/// that could confuse `claude attach` about the terminal (or its size) it's running in.
fn configure_env(builder: &mut CommandBuilder) {
    builder.env("PATH", claude_bin::augmented_path());
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    if needs_default_locale(|k| std::env::var(k).ok()) {
        builder.env("LANG", "en_US.UTF-8");
    }
    for var in ENV_VARS_TO_REMOVE {
        builder.env_remove(var);
    }
}

/// Working directory for the child: `cwd` if it's an existing directory, else `$HOME` (and, if
/// that isn't set either, `/`, so the child always gets a valid directory).
fn resolve_cwd(cwd: Option<&str>) -> PathBuf {
    if let Some(dir) = cwd {
        let path = Path::new(dir);
        if path.is_dir() {
            return path.to_path_buf();
        }
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Opens a fresh write handle to the pty via a raw `dup(2)` of its fd, instead of
/// `MasterPty::take_writer`. Verified against `portable-pty` 0.9.0's own source
/// (`unix.rs`'s `UnixMasterWriter::drop`): that handle writes a newline + `Ctrl-D` into the pty
/// when dropped (its way of signaling EOF to a shell) — exactly the wrong thing to send into
/// `claude attach`'s raw-mode input on detach, where it could submit a half-typed prompt to the
/// background session. A plain `File`'s `Drop` just closes the fd, nothing more. This is the
/// only writer of the pty on the whole detach path; nothing else touches `master` for writing.
#[cfg(unix)]
fn open_writer(master: &dyn MasterPty) -> Result<std::fs::File, String> {
    use std::os::unix::io::FromRawFd;
    let fd = master.as_raw_fd().ok_or("Couldn't get the pty's file descriptor.")?;
    // Safety: `dup(2)` either returns a valid, freshly-opened fd distinct from (and unrelated
    // to the lifetime of) `fd`, or -1 on failure, handled below.
    let dup_fd = unsafe { libc::dup(fd) };
    if dup_fd < 0 {
        return Err(format!("Couldn't duplicate the pty's file descriptor: {}", std::io::Error::last_os_error()));
    }
    // Safety: `dup_fd` was just returned by `dup(2)` above, is open, and isn't owned anywhere
    // else yet, so `File` taking ownership of it here is sound.
    Ok(unsafe { std::fs::File::from_raw_fd(dup_fd) })
}

#[cfg(not(unix))]
fn open_writer(_master: &dyn MasterPty) -> Result<std::fs::File, String> {
    Err("Writing to the terminal is only supported on Unix.".into())
}

/// A chunk read from the pty, or the end of the stream (the reader thread's signal to the
/// coalescer that no more chunks are coming).
enum Output {
    Data(Vec<u8>),
    Eof,
}

/// Sends `pending` through `on_data` as one batch. No-op if there's nothing to send.
fn flush(on_data: &OnData, pending: &mut Vec<u8>) {
    if pending.is_empty() {
        return;
    }
    on_data(std::mem::take(pending));
}

/// Batches raw chunks from the reader thread into `on_data` calls: at most `FLUSH_MAX_BYTES`
/// per call, flushed after at most `FLUSH_INTERVAL` since the first byte of the batch.
fn spawn_coalescing_thread(raw_rx: mpsc::Receiver<Output>, on_data: OnData) -> Result<(), String> {
    thread::Builder::new()
        .spawn(move || {
            let mut pending = Vec::new();
            'batches: while let Ok(Output::Data(chunk)) = raw_rx.recv() {
                pending.extend_from_slice(&chunk);
                let deadline = Instant::now() + FLUSH_INTERVAL;
                while pending.len() < FLUSH_MAX_BYTES {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    match raw_rx.recv_timeout(remaining) {
                        Ok(Output::Data(chunk)) => pending.extend_from_slice(&chunk),
                        Ok(Output::Eof) => break 'batches,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => break 'batches,
                    }
                }
                flush(&on_data, &mut pending);
            }
            flush(&on_data, &mut pending);
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Reads the pty on its own thread until EOF, then waits for the child and reports its exit
/// code. Removes the session (if it's still there; `close` may have already removed it,
/// in which case this is a no-op) right after EOF and *before* `wait()`, not after: otherwise
/// a `close`/`close_all` racing with a just-exited child could send a signal to its pid
/// after it's been reaped and possibly recycled by the OS for an unrelated process.
fn spawn_reader_thread(
    sessions: PtySessions,
    id: u32,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn Child + Send + Sync>,
    raw_tx: mpsc::Sender<Output>,
    on_exit: OnExit,
    exited: Arc<Mutex<bool>>,
) -> Result<(), String> {
    thread::Builder::new()
        .spawn(move || {
            let mut buf = [0u8; READ_BUF_SIZE];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if raw_tx.send(Output::Data(buf[..n].to_vec())).is_err() {
                            break; // The coalescer is gone: nobody will ever read this again.
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            // Under the lock and before `wait()`: see `Session::detach`'s doc comment for why.
            {
                let mut exited = exited.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                *exited = true;
            }
            sessions.remove(id);
            let _ = raw_tx.send(Output::Eof);
            let code = child.wait().ok().map(|status| status.exit_code() as i32);
            on_exit(code);
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Writes to the pty on its own thread so a write never blocks. Ends when every `writer_tx`
/// clone is dropped (i.e. the session is removed).
fn spawn_writer_thread(mut writer: std::fs::File, rx: mpsc::Receiver<Vec<u8>>) -> Result<(), String> {
    thread::Builder::new()
        .spawn(move || {
            while let Ok(data) = rx.recv() {
                if writer.write_all(&data).is_err() || writer.flush().is_err() {
                    break;
                }
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// What `spawn_session` needs about the run to attach to and the webview asking for it
/// (bundled so the function doesn't take ten separate arguments).
struct AttachRequest {
    owner: String,
    /// `owner`'s reload generation when the attach started (see `PtySessions::insert_current`).
    generation: u64,
    claude: PathBuf,
    /// Arguments to `claude` (`attach <run_id>`, or none for an interactive session).
    args: Vec<String>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
}

/// Opens the pty, spawns `claude <args>` into it and starts the session's threads.
/// Synchronous (`portable_pty` is): called from `PtySessions::attach` via `spawn_blocking`
/// by the shell.
///
/// The threads are started in an order that never loses track of `child`: the reader thread,
/// which owns it (for `wait()`), goes first. If a later thread then fails to start, `killer`
/// (still held here) sends `SIGHUP`, and the already-running reader thread winds itself down
/// once the child exits — no separate signaling needed. The session is only inserted (making it
/// reachable from `write`/`resize`/`close`) once every thread is confirmed running; this also
/// means the child is always properly reaped, even if the owning webview reloaded while this
/// was setting up (in which case an error is returned instead of the id).
fn spawn_session(sessions: PtySessions, req: AttachRequest, on_data: OnData, on_exit: OnExit) -> Result<u32, String> {
    let AttachRequest { owner, generation, claude, args, cwd, cols, rows } = req;
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Couldn't open a pty: {e}"))?;

    let mut builder = CommandBuilder::new(&claude);
    builder.args(&args);
    configure_env(&mut builder);
    builder.cwd(resolve_cwd(cwd.as_deref()));

    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|e| format!("Couldn't start `{}`: {e}", ["claude"].into_iter().chain(args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")))?;
    // The reader only sees EOF once every open handle to the slave is closed; ours must go now
    // (verified in the spike: macOS doesn't deliver EOF to the master otherwise).
    drop(pair.slave);
    let master = pair.master;

    let reader = master.try_clone_reader().map_err(|e| format!("Couldn't read from the pty: {e}"))?;
    let writer = open_writer(&*master).map_err(|e| format!("Couldn't open the pty for writing: {e}"))?;
    let mut killer = child.clone_killer();
    let pid = child.process_id();
    let exited = Arc::new(Mutex::new(false));
    let id = sessions.next_id()?;

    let (writer_tx, writer_rx) = mpsc::channel();
    let (raw_tx, raw_rx) = mpsc::channel();

    spawn_reader_thread(sessions.clone(), id, reader, child, raw_tx, on_exit, exited.clone())
        .map_err(|e| format!("Couldn't start the terminal's reader thread: {e}"))?;
    if let Err(e) = spawn_writer_thread(writer, writer_rx) {
        let _ = killer.kill();
        return Err(format!("Couldn't start the terminal's writer thread: {e}"));
    }
    if let Err(e) = spawn_coalescing_thread(raw_rx, on_data) {
        let _ = killer.kill();
        return Err(format!("Couldn't start the terminal's output thread: {e}"));
    }

    let session = Session { owner, writer_tx, killer, pid, exited, master };
    if sessions.insert_current(id, generation, session) {
        Ok(id)
    } else {
        Err("Attach cancelled: the webview reloaded before it finished starting.".into())
    }
}

#[cfg(test)]
mod tests;
