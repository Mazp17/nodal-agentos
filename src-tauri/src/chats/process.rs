//! One `claude -p` stream-json process per active chat.
//!
//! A process starts with the chat's first message (with `--resume` once the chat has a
//! session), streams what it prints to the UI as `nodal://chat` events and stays alive
//! between messages. It is stopped when idle for `IDLE_TIMEOUT`, when the chat or its
//! project is deleted, or when the chat's settings change (then the next message restarts
//! it with `--resume`). Removing a `Proc` from the map is what stops it: its stdin closes
//! (Claude Code exits on EOF) and, after `EXIT_GRACE`, the process is killed.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::{pin, Pin};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::Poll;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use crate::db::{queries::chats, Db};
use crate::events::{Events, Kind};
use crate::runs::claude_bin;
use crate::runs::claude_fs::{self, is_valid_session_id};
use crate::runs::stream_json::{self, PermissionRequest, StreamEvent};

/// Moved to `nodal_domain::model::chat`; re-exported so current uses don't break.
pub use nodal_domain::model::chat::CHAT_EVENT as EVENT;
pub use nodal_domain::model::chat::{ChatEnvelope, ChatEvent, ChatLive, ChatSpec as Spec, HostEvent, RunState};

/// A process with no turn running and nothing to approve is stopped after this long.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const REAP_EVERY: Duration = Duration::from_secs(30);
/// Time to exit on its own after stdin closes, before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(3);
/// What the model reads when the user denies without a message.
pub const DENY_MESSAGE: &str = "The user denied this in Nodal.";
const DENY_MESSAGE_MAX: usize = 2000;
const STDERR_TAIL: usize = 2000;

pub type Emit = Arc<dyn Fn(ChatEnvelope) + Send + Sync>;

struct Proc {
    gen: u64,
    project_id: String,
    spec: Spec,
    stdin: mpsc::UnboundedSender<String>,
    /// Dropped with the `Proc`: that stops the process.
    _stop: oneshot::Sender<()>,
    /// Messages sent whose turn hasn't ended (they queue while a turn runs).
    turns: u32,
    pending: Vec<PermissionRequest>,
    last_active: Instant,
    interrupts: u64,
}

impl Proc {
    fn state(&self) -> RunState {
        if self.turns > 0 {
            RunState::Busy
        } else {
            RunState::Idle
        }
    }

    fn write(&self, line: String) -> Result<(), String> {
        self.stdin.send(line).map_err(|_| "The chat's Claude process is gone: send the message again.".to_string())
    }
}

struct Shared {
    procs: Mutex<HashMap<String, Proc>>,
    gens: AtomicU64,
    emit: Emit,
    db: Db,
    events: Events,
    /// `None`: the real `claude`. Tests point it at a fake.
    program: Option<PathBuf>,
    /// Where session files live, to read each session's title (`None` in tests).
    claude_dir: Option<PathBuf>,
}

/// The chat processes. Cheap to clone.
#[derive(Clone)]
pub struct Chats(Arc<Shared>);

impl Chats {
    pub fn new(db: Db, events: Events, emit: Emit) -> Self {
        Self::with_program(db, events, emit, None, claude_fs::claude_config_dir())
    }

    fn with_program(db: Db, events: Events, emit: Emit, program: Option<PathBuf>, claude_dir: Option<PathBuf>) -> Self {
        Chats(Arc::new(Shared {
            procs: Mutex::default(),
            gens: AtomicU64::new(1),
            emit,
            db,
            events,
            program,
            claude_dir,
        }))
    }

    fn procs(&self) -> MutexGuard<'_, HashMap<String, Proc>> {
        self.0.procs.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn emit(&self, chat_id: &str, event: ChatEvent) {
        (self.0.emit)(ChatEnvelope { chat_id: chat_id.to_string(), event });
    }

    fn emit_state(&self, chat_id: &str, state: RunState) {
        self.emit(chat_id, ChatEvent::Host(HostEvent::State { state }));
    }

    /// Stops idle processes every `REAP_EVERY`.
    pub fn start_reaper(&self) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(REAP_EVERY).await;
                me.reap_idle(IDLE_TIMEOUT, Instant::now());
            }
        });
    }

    /// Stops the processes with no turn running, nothing to approve and no activity for
    /// `max_idle`. Returns how many.
    pub fn reap_idle(&self, max_idle: Duration, now: Instant) -> usize {
        let mut procs = self.procs();
        let before = procs.len();
        procs.retain(|_, p| p.turns > 0 || !p.pending.is_empty() || now.duration_since(p.last_active) < max_idle);
        before - procs.len()
    }

    /// Sends a message, starting the process if needed (resuming `session_id`). A process
    /// launched with another spec is restarted first, unless it is in the middle of a turn:
    /// then the message queues and the new spec applies once it goes idle.
    pub fn send(&self, chat_id: &str, project_id: &str, session_id: Option<&str>, spec: Spec, text: &str) -> Result<(), String> {
        let mut procs = self.procs();
        if procs.get(chat_id).is_some_and(|p| p.spec != spec && p.turns == 0 && p.pending.is_empty()) {
            procs.remove(chat_id);
        }
        if !procs.contains_key(chat_id) {
            let p = self.spawn(chat_id, project_id, session_id, spec)?;
            procs.insert(chat_id.to_string(), p);
        }
        let Some(p) = procs.get_mut(chat_id) else { return Err("The chat's Claude process didn't start.".into()) };
        p.write(stream_json::user_message(text))?;
        p.turns += 1;
        p.last_active = Instant::now();
        drop(procs);
        self.emit_state(chat_id, RunState::Busy);
        Ok(())
    }

    /// Answers a pending permission request. `message` is what the model reads on a denial.
    pub fn respond(&self, chat_id: &str, request_id: &str, allow: bool, message: Option<&str>) -> Result<(), String> {
        let mut procs = self.procs();
        let p = procs.get_mut(chat_id).ok_or("The chat isn't running anymore, so that request has expired.")?;
        let i = p
            .pending
            .iter()
            .position(|r| r.request_id == request_id)
            .ok_or("That permission request is no longer pending.")?;
        let deny = message.map(str::trim).filter(|m| !m.is_empty()).unwrap_or(DENY_MESSAGE);
        let deny = crate::util::clip_chars(deny, DENY_MESSAGE_MAX);
        let line = stream_json::permission_response(request_id, allow.then_some(&p.pending[i].input), &deny);
        p.write(line)?;
        p.pending.remove(i);
        p.last_active = Instant::now();
        drop(procs);
        self.emit(chat_id, ChatEvent::Host(HostEvent::PermissionResolved { request_id: request_id.into(), allowed: allow }));
        Ok(())
    }

    /// Stops the running turn; the process stays for the next message. Pending requests
    /// are dropped (the turn that asked is over). Nothing to do if no turn is running.
    pub fn interrupt(&self, chat_id: &str) -> Result<(), String> {
        let mut procs = self.procs();
        let Some(p) = procs.get_mut(chat_id).filter(|p| p.turns > 0) else { return Ok(()) };
        p.interrupts += 1;
        p.write(stream_json::interrupt_request(&format!("interrupt-{}", p.interrupts)))?;
        let dropped = std::mem::take(&mut p.pending);
        drop(procs);
        for r in dropped {
            self.emit(chat_id, ChatEvent::Host(HostEvent::PermissionResolved { request_id: r.request_id, allowed: false }));
        }
        Ok(())
    }

    /// Stops the chat's process, if any. Its session stays resumable.
    pub fn stop(&self, chat_id: &str) {
        self.procs().remove(chat_id);
    }

    /// Stops every process of the project (it is being deleted).
    pub fn stop_project(&self, project_id: &str) {
        self.procs().retain(|_, p| p.project_id != project_id);
    }

    pub fn live(&self, chat_id: &str) -> ChatLive {
        match self.procs().get(chat_id) {
            Some(p) => ChatLive { state: p.state(), pending: p.pending.clone() },
            None => ChatLive { state: RunState::Stopped, pending: vec![] },
        }
    }

    fn command(&self) -> Result<Command, String> {
        match &self.0.program {
            Some(p) => Ok(Command::new(p)),
            None => claude_bin::claude_command(),
        }
    }

    fn spawn(&self, chat_id: &str, project_id: &str, session_id: Option<&str>, spec: Spec) -> Result<Proc, String> {
        if !spec.cwd.is_dir() {
            return Err(format!("The chat's folder doesn't exist: {}", spec.cwd.display()));
        }
        let mut cmd = self.command()?;
        let (key, value) = stream_json::CHAT_ENTRYPOINT;
        cmd.args(stream_json::CHAT_ARGS).args(&spec.args).env(key, value);
        if let Some(sid) = session_id {
            if !is_valid_session_id(sid) {
                return Err(format!("Invalid session id: \"{sid}\"."));
            }
            cmd.args(stream_json::resume_args(sid));
        }
        cmd.current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| format!("Couldn't start `claude`: {e}"))?;
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err("Couldn't connect to `claude`'s input and output.".into());
        };
        let gen = self.0.gens.fetch_add(1, Ordering::Relaxed);

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        tauri::async_runtime::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = rx.recv().await {
                let ok = stdin.write_all(format!("{line}\n").as_bytes()).await.is_ok() && stdin.flush().await.is_ok();
                if !ok {
                    break;
                }
            }
        });

        let tail = Arc::new(Mutex::new(String::new()));
        let errors = drain_stderr(stderr, tail.clone());

        let me = self.clone();
        let id = chat_id.to_string();
        let reader = tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                me.on_line(&id, gen, &line);
            }
        });

        let (stop_tx, stop_rx) = oneshot::channel();
        let me = self.clone();
        let id = chat_id.to_string();
        tauri::async_runtime::spawn(async move {
            let (status, stopped) = supervise(child, stop_rx).await;
            // Every line read before reporting: the last events, and stderr for the error.
            let _ = tokio::time::timeout(EXIT_GRACE, async {
                let _ = reader.await;
                let _ = errors.await;
            })
            .await;
            let tail = tail.lock().map(|t| t.trim().to_string()).unwrap_or_default();
            me.on_exit(&id, gen, status, stopped, &tail);
        });

        Ok(Proc {
            gen,
            project_id: project_id.to_string(),
            spec,
            stdin: tx,
            _stop: stop_tx,
            turns: 0,
            pending: vec![],
            last_active: Instant::now(),
            interrupts: 0,
        })
    }

    fn on_line(&self, chat_id: &str, gen: u64, line: &str) {
        for ev in stream_json::parse_line(line) {
            let mut idle = false;
            let mut session = None;
            let mut title_from = None;
            if let Some(p) = self.procs().get_mut(chat_id).filter(|p| p.gen == gen) {
                p.last_active = Instant::now();
                match &ev {
                    StreamEvent::PermissionRequest(r) => p.pending.push(r.clone()),
                    StreamEvent::TurnEnd { .. } => {
                        title_from = Some((p.spec.cwd.clone(), p.project_id.clone()));
                        p.turns = p.turns.saturating_sub(1);
                        idle = p.turns == 0;
                        if idle {
                            p.pending.clear();
                        }
                    }
                    StreamEvent::Init { session_id, .. } => session = Some((session_id.clone(), p.project_id.clone())),
                    _ => {}
                }
            }
            if let Some((sid, project_id)) = session {
                self.save_session(chat_id, sid, project_id);
            }
            if let Some((cwd, project_id)) = title_from {
                self.refresh_title(chat_id, cwd, project_id);
            }
            self.emit(chat_id, ChatEvent::Stream(ev));
            if idle {
                self.emit_state(chat_id, RunState::Idle);
            }
        }
    }

    fn save_session(&self, chat_id: &str, session_id: String, project_id: String) {
        if !is_valid_session_id(&session_id) {
            return;
        }
        let (db, events, chat_id) = (self.0.db.clone(), self.0.events.clone(), chat_id.to_string());
        tauri::async_runtime::spawn_blocking(move || {
            let conn = db.lock().unwrap_or_else(|p| p.into_inner());
            match chats::set_session(&conn, &chat_id, &session_id) {
                Ok(true) => events.notify(Kind::Chats, Some(&project_id)),
                Ok(false) => {}
                Err(e) => eprintln!("chats: {e}"),
            }
        });
    }

    /// After a turn, stores the name Claude Code gave the session (it writes it to the session
    /// file, not to the stream).
    /// `cwd` is the running process's `Spec.cwd` (a repo or the project root), not one
    /// recomputed from the project, which may have changed since the process started.
    fn refresh_title(&self, chat_id: &str, cwd: PathBuf, project_id: String) {
        let Some(projects) = self.0.claude_dir.as_ref().map(|d| d.join("projects")) else { return };
        let (db, events, chat_id) = (self.0.db.clone(), self.0.events.clone(), chat_id.to_string());
        tauri::async_runtime::spawn_blocking(move || {
            let sid = {
                let conn = db.lock().unwrap_or_else(|p| p.into_inner());
                chats::get(&conn, &chat_id).ok().and_then(|c| c.session_id)
            };
            let Some(sid) = sid.filter(|s| is_valid_session_id(s)) else { return };
            let Some(path) = claude_fs::find_session_jsonl(&projects, &cwd.to_string_lossy(), &sid) else { return };
            let Some(title) = claude_fs::session_title(&path) else { return };
            let conn = db.lock().unwrap_or_else(|p| p.into_inner());
            match chats::set_session_title(&conn, &chat_id, &title) {
                Ok(true) => events.notify(Kind::Chats, Some(&project_id)),
                Ok(false) => {}
                Err(e) => eprintln!("chats: {e}"),
            }
        });
    }

    fn on_exit(&self, chat_id: &str, gen: u64, status: Option<ExitStatus>, stopped: bool, stderr: &str) {
        let mut procs = self.procs();
        let current = procs.get(chat_id).is_some_and(|p| p.gen == gen);
        let mid_turn = current && procs.get(chat_id).is_some_and(|p| p.turns > 0);
        if current {
            procs.remove(chat_id);
        }
        let replaced = procs.contains_key(chat_id);
        drop(procs);
        let failed = status.is_none_or(|s| !s.success());
        if current && !stopped && (failed || mid_turn) {
            let message = if !stderr.is_empty() {
                stderr.to_string()
            } else {
                match status.and_then(|s| s.code()) {
                    Some(code) => format!("Claude exited unexpectedly (exit code {code})."),
                    None => "Claude exited unexpectedly.".to_string(),
                }
            };
            self.emit(chat_id, ChatEvent::Host(HostEvent::Error { message }));
        }
        if !replaced {
            self.emit_state(chat_id, RunState::Stopped);
        }
    }
}

/// Waits for the child to exit, or for `stop` to fire (or be dropped): then gives it
/// `EXIT_GRACE` to exit on its own and kills it. Returns the status and whether it was stopped.
async fn supervise(mut child: Child, mut stop: oneshot::Receiver<()>) -> (Option<ExitStatus>, bool) {
    let exited = {
        let mut wait = pin!(child.wait());
        std::future::poll_fn(|cx| {
            if let Poll::Ready(r) = wait.as_mut().poll(cx) {
                return Poll::Ready(Some(r.ok()));
            }
            if Pin::new(&mut stop).poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            Poll::Pending
        })
        .await
    };
    if let Some(status) = exited {
        return (status, false);
    }
    if let Ok(r) = tokio::time::timeout(EXIT_GRACE, child.wait()).await {
        return (r.ok(), true);
    }
    let _ = child.start_kill();
    (child.wait().await.ok(), true)
}

/// Keeps the last `STDERR_TAIL` characters, reading until EOF so `claude` never blocks on a full pipe.
fn drain_stderr<R: AsyncRead + Unpin + Send + 'static>(
    stream: R,
    tail: Arc<Mutex<String>>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(mut t) = tail.lock() else { continue };
            t.push_str(&line);
            t.push('\n');
            let n = t.chars().count();
            if n > STDERR_TAIL {
                *t = t.chars().skip(n - STDERR_TAIL).collect();
            }
        }
    })
}

#[cfg(test)]
mod tests;
