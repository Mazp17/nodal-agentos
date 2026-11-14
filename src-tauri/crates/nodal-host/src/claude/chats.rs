//! One `claude -p` stream-json process per active chat.
//!
//! A process starts with the chat's first message (with `--resume` once the chat has a
//! session), streams what it prints to the UI through the `ChatSink` port and stays alive
//! between messages. It is stopped when idle for `IDLE_TIMEOUT`, when the chat or its
//! project is deleted, or when the chat's settings change (then the next message restarts
//! it with `--resume`). Removing a `Proc` from the map is what stops it: its stdin closes
//! (Claude Code exits on EOF) and, after `EXIT_GRACE`, the process is killed.
//!
//! A rewind (`rewind_files`, `rewind_conversation`) never runs mid-turn: files are restored by
//! a `rewind_files` control request answered on stdout, and the conversation by stopping the
//! process so the next one resumes with `--resume-session-at`.
//!
//! DB side effects of a turn (saving the session id, refreshing the session title) go
//! through the `ChatHooks` port instead of touching a database directly: this crate has no
//! store dependency.

use std::collections::{HashMap, HashSet};
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
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use nodal_domain::error::HostError;
use serde_json::Value;

use nodal_domain::model::chat::{
    ChatEnvelope, ChatEvent, ChatLive, ChatSpec as Spec, HostEvent, ImageInput, PermissionRequest, RewindResult,
    RunState, StreamEvent,
};
use nodal_domain::model::claude::TranscriptItem;
use nodal_domain::ports::{BoxFut, ChatHooks, ChatRuntime, ChatSink};
use nodal_domain::sessions::transcript::{is_valid_message_uuid, is_valid_session_id};

use super::bin as claude_bin;
use super::stream_json;

/// A process with no turn running and nothing to approve is stopped after this long.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const REAP_EVERY: Duration = Duration::from_secs(30);
/// Time to exit on its own after stdin closes, before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(3);
/// What the model reads when the user denies without a message.
pub const DENY_MESSAGE: &str = "The user denied this in Nodal.";
const DENY_MESSAGE_MAX: usize = 2000;
/// The tool Claude Code asks the user questions with; allowing it carries the answers.
const ASK_USER_QUESTION: &str = "AskUserQuestion";
const STDERR_TAIL: usize = 2000;
/// How long a `rewind_files` request waits for its answer.
const REWIND_TIMEOUT: Duration = Duration::from_secs(20);
/// P05: caps `TextDelta`/`ThinkingDelta` emits at under 60/s per chat. Consecutive deltas of
/// the same kind arriving inside one window are merged into a single emit; anything else
/// (an item, a permission request, the turn ending…) flushes whatever is buffered first, so
/// ordering on the wire matches the order `claude` printed things in.
const DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(17);
/// P08: a misbehaving (or malicious) `claude` process could keep stdout open and noisy
/// forever; past this many lines in one process's lifetime, further lines are no longer
/// parsed/emitted and the process is stopped and reported. Real turns are nowhere near this.
#[cfg(not(test))]
const MAX_STDOUT_LINES: u64 = 200_000;
#[cfg(test)]
const MAX_STDOUT_LINES: u64 = 100;

enum DeltaKind {
    Text,
    Thinking,
}

struct PendingDelta {
    kind: DeltaKind,
    text: String,
}

impl PendingDelta {
    fn into_event(self) -> StreamEvent {
        match self.kind {
            DeltaKind::Text => StreamEvent::TextDelta { text: self.text },
            DeltaKind::Thinking => StreamEvent::ThinkingDelta { text: self.text },
        }
    }
}

struct Proc {
    gen: u64,
    project_id: String,
    spec: Spec,
    stdin: mpsc::UnboundedSender<String>,
    /// Dropped with the `Proc`: that stops the process.
    _stop: oneshot::Sender<()>,
    /// Turns still to end: busy while > 0. Recounted from `unechoed` at each `result`, since
    /// several queued messages can share one (see `TurnEnd` in `dispatch`).
    turns: u32,
    /// Uuids of the messages written to stdin that `claude` hasn't echoed (read) yet.
    unechoed: Vec<String>,
    pending: Vec<PermissionRequest>,
    last_active: Instant,
    interrupts: u64,
    delta: Option<PendingDelta>,
    delta_last_flush: Instant,
    /// Control requests waiting for their `control_response`, by `request_id`.
    controls: HashMap<String, oneshot::Sender<Result<Value, String>>>,
    /// Shells the process runs in the background (`task_id`s): it isn't reaped while any runs,
    /// and only these can be stopped.
    shells: HashSet<String>,
}

impl Proc {
    fn state(&self) -> RunState {
        if self.turns > 0 {
            RunState::Busy
        } else {
            RunState::Idle
        }
    }

    fn write(&self, line: String) -> Result<(), HostError> {
        self.stdin
            .send(line)
            .map_err(|_| "The chat's Claude process is gone: send the message again.".into())
    }
}

struct Shared {
    procs: Mutex<HashMap<String, Proc>>,
    /// Per chat, the assistant message its next process resumes at (`--resume-session-at`)
    /// until a message is sent. shortcut: in memory, lost on app restart (the chat then
    /// resumes at the end of the abandoned branch); upgrade by persisting it with the chat.
    resume_at: Mutex<HashMap<String, String>>,
    gens: AtomicU64,
    sink: Arc<dyn ChatSink>,
    hooks: Arc<dyn ChatHooks>,
    rt: Handle,
    /// `None`: the real `claude`. Tests point it at a fake.
    program: Option<PathBuf>,
}

/// The chat processes (`nodal_domain::ports::ChatRuntime`). Cheap to clone.
#[derive(Clone)]
pub struct ChatProcesses(Arc<Shared>);

impl ChatProcesses {
    pub fn new(sink: Arc<dyn ChatSink>, hooks: Arc<dyn ChatHooks>, rt: Handle) -> Arc<Self> {
        Self::with_program_inner(sink, hooks, rt, None)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_program(
        sink: Arc<dyn ChatSink>,
        hooks: Arc<dyn ChatHooks>,
        rt: Handle,
        program: Option<PathBuf>,
    ) -> Arc<Self> {
        Self::with_program_inner(sink, hooks, rt, program)
    }

    fn with_program_inner(
        sink: Arc<dyn ChatSink>,
        hooks: Arc<dyn ChatHooks>,
        rt: Handle,
        program: Option<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(ChatProcesses(Arc::new(Shared {
            procs: Mutex::default(),
            resume_at: Mutex::default(),
            gens: AtomicU64::new(1),
            sink,
            hooks,
            rt,
            program,
        })))
    }

    fn procs(&self) -> MutexGuard<'_, HashMap<String, Proc>> {
        self.0.procs.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn resume_at(&self) -> MutexGuard<'_, HashMap<String, String>> {
        self.0.resume_at.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn emit(&self, chat_id: &str, event: ChatEvent) {
        self.0.sink.emit(ChatEnvelope {
            chat_id: chat_id.to_string(),
            event,
        });
    }

    fn emit_state(&self, chat_id: &str, state: RunState) {
        self.emit(chat_id, ChatEvent::Host(HostEvent::State { state }));
    }

    /// Stops the processes with no turn running, nothing to approve, no background shell and no activity for
    /// `max_idle`. Returns how many.
    fn reap(&self, max_idle: Duration, now: Instant) -> usize {
        let mut procs = self.procs();
        let before = procs.len();
        procs.retain(|_, p| {
            p.turns > 0
                || !p.pending.is_empty()
                || !p.shells.is_empty()
                || now.duration_since(p.last_active) < max_idle
        });
        before - procs.len()
    }

    /// Test-only access to `reap`, driven by an explicit clock instead of the reaper loop.
    #[cfg(any(test, feature = "test-support"))]
    pub fn reap_idle(&self, max_idle: Duration, now: Instant) -> usize {
        self.reap(max_idle, now)
    }

    /// Drops every running process without detaching them cleanly (test cleanup only).
    #[cfg(any(test, feature = "test-support"))]
    pub fn clear(&self) {
        self.procs().clear();
    }

    fn command(&self) -> Result<Command, HostError> {
        match &self.0.program {
            Some(p) => Ok(Command::new(p)),
            None => claude_bin::claude_command().map_err(HostError::from),
        }
    }

    fn spawn(
        &self,
        chat_id: &str,
        project_id: &str,
        session_id: Option<&str>,
        spec: Spec,
    ) -> Result<Proc, HostError> {
        if !spec.cwd.is_dir() {
            return Err(format!("The chat's folder doesn't exist: {}", spec.cwd.display()).into());
        }
        let mut cmd = self.command()?;
        let (key, value) = stream_json::CHAT_ENTRYPOINT;
        let (ckey, cvalue) = stream_json::CHAT_CHECKPOINTING;
        cmd.args(stream_json::CHAT_ARGS)
            .args(&spec.args)
            .env(key, value)
            .env(ckey, cvalue);
        if let Some(sid) = session_id {
            if !is_valid_session_id(sid) {
                return Err(format!("Invalid session id: \"{sid}\".").into());
            }
            cmd.args(stream_json::resume_args(sid));
            if let Some(at) = self.resume_at().get(chat_id) {
                cmd.args(stream_json::resume_at_args(at));
            }
        }
        cmd.current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = claude_bin::spawn_retrying(|| cmd.spawn())
            .map_err(|e| format!("Couldn't start `claude`: {e}"))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err("Couldn't connect to `claude`'s input and output.".into());
        };
        let gen = self.0.gens.fetch_add(1, Ordering::Relaxed);
        let rt = self.0.rt.clone();

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        rt.spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = rx.recv().await {
                let ok = stdin
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                    .is_ok()
                    && stdin.flush().await.is_ok();
                if !ok {
                    break;
                }
            }
        });

        let tail = Arc::new(Mutex::new(String::new()));
        let errors = drain_stderr(stderr, tail.clone(), &self.0.rt);

        let me = self.clone();
        let id = chat_id.to_string();
        let reader = self.0.rt.spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut n: u64 = 0;
            while let Ok(Some(line)) = lines.next_line().await {
                n += 1;
                if n > MAX_STDOUT_LINES {
                    me.on_stdout_overflow(&id, gen);
                    break;
                }
                me.on_line(&id, gen, &line);
            }
        });

        let (stop_tx, stop_rx) = oneshot::channel();
        let me = self.clone();
        let id = chat_id.to_string();
        self.0.rt.spawn(async move {
            let (status, stopped) = supervise(child, stop_rx).await;
            // Every line read before reporting: the last events, and stderr for the error.
            let _ = tokio::time::timeout(EXIT_GRACE, async {
                let _ = reader.await;
                let _ = errors.await;
            })
            .await;
            let tail = tail
                .lock()
                .map(|t| t.trim().to_string())
                .unwrap_or_default();
            me.on_exit(&id, gen, status, stopped, &tail);
        });

        Ok(Proc {
            gen,
            project_id: project_id.to_string(),
            spec,
            stdin: tx,
            _stop: stop_tx,
            turns: 0,
            unechoed: vec![],
            pending: vec![],
            last_active: Instant::now(),
            interrupts: 0,
            delta: None,
            delta_last_flush: Instant::now(),
            controls: HashMap::new(),
            shells: HashSet::new(),
        })
    }

    /// Buffers a `TextDelta`/`ThinkingDelta`, merging it into whatever's already buffered when
    /// it's the same kind, and emits at most one delta event per `DELTA_FLUSH_INTERVAL` (P05:
    /// keeps a chat under 60 emits/s from deltas alone even when `claude` prints them faster).
    fn on_delta(&self, chat_id: &str, gen: u64, kind: DeltaKind, text: &str) {
        let mut to_flush = None;
        {
            let mut procs = self.procs();
            let Some(p) = procs.get_mut(chat_id).filter(|p| p.gen == gen) else {
                return;
            };
            p.last_active = Instant::now();
            match &mut p.delta {
                Some(pending) if matches!((&pending.kind, &kind), (DeltaKind::Text, DeltaKind::Text) | (DeltaKind::Thinking, DeltaKind::Thinking)) => {
                    pending.text.push_str(text);
                }
                Some(_) => {
                    to_flush = p.delta.take();
                    p.delta = Some(PendingDelta { kind, text: text.to_string() });
                    p.delta_last_flush = Instant::now();
                }
                None => {
                    p.delta = Some(PendingDelta { kind, text: text.to_string() });
                }
            }
            if to_flush.is_none() && p.delta_last_flush.elapsed() >= DELTA_FLUSH_INTERVAL {
                to_flush = p.delta.take();
                p.delta_last_flush = Instant::now();
            }
        }
        if let Some(pending) = to_flush {
            self.emit(chat_id, ChatEvent::Stream(pending.into_event()));
        }
    }

    /// Emits whatever delta is buffered, if any — called before any non-delta event so the
    /// wire order still matches what `claude` printed.
    fn flush_delta(&self, chat_id: &str, gen: u64) {
        let pending = self
            .procs()
            .get_mut(chat_id)
            .filter(|p| p.gen == gen)
            .and_then(|p| p.delta.take());
        if let Some(pending) = pending {
            self.emit(chat_id, ChatEvent::Stream(pending.into_event()));
        }
    }

    fn on_line(&self, chat_id: &str, gen: u64, line: &str) {
        if line.contains("\"control_response\"") {
            if let Some((id, response)) = stream_json::parse_control_response(line) {
                let waiting = self.procs().get_mut(chat_id).filter(|p| p.gen == gen).and_then(|p| p.controls.remove(&id));
                if let Some(tx) = waiting {
                    let _ = tx.send(response);
                }
            }
            return;
        }
        if line.contains("\"rate_limit_event\"") {
            use nodal_domain::ports::Clock;
            super::plan_usage::record(line, crate::adapters::SystemClock.now_ms());
            return;
        }
        for ev in stream_json::parse_line(line) {
            match ev {
                StreamEvent::TextDelta { text } => self.on_delta(chat_id, gen, DeltaKind::Text, &text),
                StreamEvent::ThinkingDelta { text } => self.on_delta(chat_id, gen, DeltaKind::Thinking, &text),
                ev => self.dispatch(chat_id, gen, ev),
            }
        }
    }

    fn dispatch(&self, chat_id: &str, gen: u64, ev: StreamEvent) {
        self.flush_delta(chat_id, gen);
        let mut idle = false;
        let mut session = None;
        let mut title_from = None;
        if let Some(p) = self.procs().get_mut(chat_id).filter(|p| p.gen == gen) {
            p.last_active = Instant::now();
            match &ev {
                StreamEvent::PermissionRequest(r) => p.pending.push(r.clone()),
                StreamEvent::ShellStarted { task_id, background: true, .. } => {
                    p.shells.insert(task_id.clone());
                }
                StreamEvent::TaskEnded { task_id, .. } => {
                    p.shells.remove(task_id);
                }
                StreamEvent::Item { item: TranscriptItem::User { uuid: Some(u), .. } } => {
                    p.unechoed.retain(|x| x != u);
                }
                // Messages sent mid-turn queue in `claude` (verified with 2.1.325): one sent
                // during a tool call is echoed and read at the next tool boundary and answered
                // in the same turn, so several messages can end with a single `result`; one
                // sent when no tool call is left starts its own turn and `result`. After an
                // interrupt, queued messages still run, as a new turn. So the turn is over only
                // when every message sent has been echoed by this `result`. (`min` with the
                // plain count keeps a `claude` that doesn't echo from staying busy forever.)
                StreamEvent::TurnEnd { .. } => {
                    title_from = Some((p.spec.cwd.clone(), p.project_id.clone()));
                    p.turns = p.turns.saturating_sub(1).min(p.unechoed.len() as u32);
                    if p.turns == 0 {
                        p.unechoed.clear();
                    }
                    idle = p.turns == 0;
                    if idle {
                        p.pending.clear();
                    }
                }
                StreamEvent::Init { session_id, .. } => {
                    session = Some((session_id.clone(), p.project_id.clone()))
                }
                _ => {}
            }
        }
        if let Some((sid, project_id)) = session {
            if is_valid_session_id(&sid) {
                self.0.hooks.session_started(chat_id, sid, project_id);
            }
        }
        if let Some((cwd, project_id)) = title_from {
            self.0.hooks.turn_ended(chat_id, cwd, project_id);
        }
        self.emit(chat_id, ChatEvent::Stream(ev));
        if idle {
            self.emit_state(chat_id, RunState::Idle);
        }
    }

    /// P08: the reader hit `MAX_STDOUT_LINES` without ever reaching a clean turn end — stops
    /// the process (like `ChatRuntime::stop`) and reports why, since `on_exit` won't (it only
    /// reports a failure for the process it still finds current).
    fn on_stdout_overflow(&self, chat_id: &str, gen: u64) {
        let removed = {
            let mut procs = self.procs();
            if procs.get(chat_id).is_some_and(|p| p.gen == gen) {
                procs.remove(chat_id);
                true
            } else {
                false
            }
        };
        if removed {
            self.emit(
                chat_id,
                ChatEvent::Host(HostEvent::Error {
                    message: format!(
                        "Claude's output exceeded {MAX_STDOUT_LINES} lines without ending the turn and was stopped."
                    ),
                }),
            );
        }
    }

    fn on_exit(
        &self,
        chat_id: &str,
        gen: u64,
        status: Option<ExitStatus>,
        stopped: bool,
        stderr: &str,
    ) {
        // A delta buffered right before the process exited would otherwise never reach the UI.
        self.flush_delta(chat_id, gen);
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

impl ChatRuntime for ChatProcesses {
    /// Sends a message, starting the process if needed (resuming `session_id`). A process
    /// launched with another spec is restarted first, unless it is in the middle of a turn:
    /// then the message queues and the new spec applies once it goes idle.
    fn send(
        &self,
        chat_id: &str,
        project_id: &str,
        session_id: Option<&str>,
        spec: Spec,
        text: &str,
        images: &[ImageInput],
    ) -> Result<(), HostError> {
        let mut procs = self.procs();
        if procs
            .get(chat_id)
            .is_some_and(|p| p.spec != spec && p.turns == 0 && p.pending.is_empty())
        {
            procs.remove(chat_id);
        }
        if !procs.contains_key(chat_id) {
            let p = self.spawn(chat_id, project_id, session_id, spec)?;
            procs.insert(chat_id.to_string(), p);
        }
        let Some(p) = procs.get_mut(chat_id) else {
            return Err("The chat's Claude process didn't start.".into());
        };
        let uuid = stream_json::new_uuid();
        p.write(stream_json::user_message(text, images, &uuid))?;
        p.unechoed.push(uuid);
        p.turns += 1;
        p.last_active = Instant::now();
        drop(procs);
        // The new message continues from the rewound point, so the process no longer needs it.
        self.resume_at().remove(chat_id);
        self.emit_state(chat_id, RunState::Busy);
        Ok(())
    }

    /// Answers a pending permission request. `message` is what the model reads on a denial.
    fn respond(
        &self,
        chat_id: &str,
        request_id: &str,
        allow: bool,
        message: Option<&str>,
        answers: Option<&std::collections::BTreeMap<String, String>>,
    ) -> Result<(), HostError> {
        let mut procs = self.procs();
        let p = procs
            .get_mut(chat_id)
            .ok_or("The chat isn't running anymore, so that request has expired.")?;
        let i = p
            .pending
            .iter()
            .position(|r| r.request_id == request_id)
            .ok_or("That permission request is no longer pending.")?;
        let deny = message
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or(DENY_MESSAGE);
        let deny = nodal_domain::util::clip_chars(deny, DENY_MESSAGE_MAX);
        let mut input = p.pending[i].input.clone();
        if let Some(answers) = answers.filter(|_| allow) {
            let obj = input
                .as_object_mut()
                .filter(|_| p.pending[i].tool_name == ASK_USER_QUESTION)
                .ok_or("Only a question from Claude takes answers.")?;
            obj.insert("answers".into(), serde_json::json!(answers));
        }
        let line = stream_json::permission_response(request_id, allow.then_some(&input), &deny);
        p.write(line)?;
        p.pending.remove(i);
        p.last_active = Instant::now();
        drop(procs);
        self.emit(
            chat_id,
            ChatEvent::Host(HostEvent::PermissionResolved {
                request_id: request_id.into(),
                allowed: allow,
            }),
        );
        Ok(())
    }

    /// Stops the running turn; the process stays for the next message. Pending requests
    /// are dropped (the turn that asked is over). Nothing to do if no turn is running.
    /// Messages queued in `claude` aren't dropped: it answers them next, as a new turn.
    fn interrupt(&self, chat_id: &str) -> Result<(), HostError> {
        let mut procs = self.procs();
        let Some(p) = procs.get_mut(chat_id).filter(|p| p.turns > 0) else {
            return Ok(());
        };
        p.interrupts += 1;
        p.write(stream_json::interrupt_request(&format!(
            "interrupt-{}",
            p.interrupts
        )))?;
        let dropped = std::mem::take(&mut p.pending);
        drop(procs);
        for r in dropped {
            self.emit(
                chat_id,
                ChatEvent::Host(HostEvent::PermissionResolved {
                    request_id: r.request_id,
                    allowed: false,
                }),
            );
        }
        Ok(())
    }

    /// Stops one of the chat's background shells through Claude Code (`stop_task`), which
    /// kills that shell's process tree; the chat's process and turn keep going. Only a shell
    /// this process reported as started is accepted, so nothing else can be signaled.
    fn stop_shell(&self, chat_id: &str, task_id: &str) -> Result<(), HostError> {
        let procs = self.procs();
        let Some(p) = procs.get(chat_id).filter(|p| p.shells.contains(task_id)) else {
            return Err("That shell isn't running.".into());
        };
        p.write(stream_json::stop_task_request(&format!("stop-{task_id}"), task_id))
    }

    fn rewind_files<'a>(
        &'a self,
        chat_id: &'a str,
        project_id: &'a str,
        session_id: Option<&'a str>,
        spec: Spec,
        message_uuid: &'a str,
        dry_run: bool,
    ) -> BoxFut<'a, Result<RewindResult, HostError>> {
        Box::pin(async move {
            if !is_valid_message_uuid(message_uuid) {
                return Err("Invalid message id.".into());
            }
            let request_id = format!("rewind-{}", stream_json::new_uuid());
            let (tx, rx) = oneshot::channel();
            {
                let mut procs = self.procs();
                if procs.get(chat_id).is_some_and(|p| p.turns > 0) {
                    return Err(REWIND_BUSY.into());
                }
                if !procs.contains_key(chat_id) {
                    let p = self.spawn(chat_id, project_id, session_id, spec)?;
                    procs.insert(chat_id.to_string(), p);
                }
                let Some(p) = procs.get_mut(chat_id) else {
                    return Err("The chat's Claude process didn't start.".into());
                };
                p.controls.insert(request_id.clone(), tx);
                p.write(stream_json::rewind_files_request(&request_id, message_uuid, dry_run))?;
                p.last_active = Instant::now();
            }
            let answer = tokio::time::timeout(REWIND_TIMEOUT, rx).await;
            if let Some(p) = self.procs().get_mut(chat_id) {
                p.controls.remove(&request_id);
            }
            match answer {
                Ok(Ok(response)) => Ok(rewind_result(response)),
                Ok(Err(_)) => Err("Claude stopped before answering the rewind.".into()),
                Err(_) => Err("Claude didn't answer the rewind in time.".into()),
            }
        })
    }

    fn rewind_conversation(&self, chat_id: &str, resume_at: Option<&str>) -> Result<(), HostError> {
        let mut procs = self.procs();
        if procs.get(chat_id).is_some_and(|p| p.turns > 0) {
            return Err(REWIND_BUSY.into());
        }
        if resume_at.is_some_and(|u| !is_valid_message_uuid(u)) {
            return Err("Invalid message id.".into());
        }
        procs.remove(chat_id);
        drop(procs);
        match resume_at {
            Some(u) => self.resume_at().insert(chat_id.to_string(), u.to_string()),
            None => self.resume_at().remove(chat_id),
        };
        Ok(())
    }

    /// Stops the chat's process, if any. Its session stays resumable.
    fn stop(&self, chat_id: &str) {
        self.procs().remove(chat_id);
    }

    /// Stops every process of the project (it is being deleted).
    fn stop_project(&self, project_id: &str) {
        self.procs().retain(|_, p| p.project_id != project_id);
    }

    fn live(&self, chat_id: &str) -> ChatLive {
        match self.procs().get(chat_id) {
            Some(p) => ChatLive {
                state: p.state(),
                pending: p.pending.clone(),
            },
            None => ChatLive {
                state: RunState::Stopped,
                pending: vec![],
            },
        }
    }

    /// Stops idle processes every `REAP_EVERY`.
    fn start_reaper(&self) {
        let me = self.clone();
        self.0.rt.spawn(async move {
            loop {
                tokio::time::sleep(REAP_EVERY).await;
                me.reap(IDLE_TIMEOUT, Instant::now());
            }
        });
    }
}

const REWIND_BUSY: &str = "Wait for the current turn to finish (or stop it) before rewinding.";

/// `rewind_files`' answer: `{canRewind, filesChanged, insertions, deletions, error}`.
/// A request that can't be honored is reported in `files_error`, not as a failure: the
/// conversation can still be rewound.
fn rewind_result(response: Result<Value, String>) -> RewindResult {
    let v = match response {
        Ok(v) => v,
        Err(e) => return RewindResult { files_error: Some(e), ..Default::default() },
    };
    if v.get("canRewind").and_then(Value::as_bool) != Some(true) {
        let e = v.get("error").and_then(Value::as_str).unwrap_or("The files can't be restored.");
        return RewindResult { files_error: Some(e.to_string()), ..Default::default() };
    }
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    RewindResult {
        files_changed: v
            .get("filesChanged")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
        insertions: n("insertions"),
        deletions: n("deletions"),
        files_error: None,
    }
}

/// Waits for the child to exit, or for `stop` to fire (or be dropped): then gives it
/// `EXIT_GRACE` to exit on its own and kills it. Returns the status and whether it was stopped.
async fn supervise(
    mut child: Child,
    mut stop: oneshot::Receiver<()>,
) -> (Option<ExitStatus>, bool) {
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
    // shortcut: kills only the direct child (grandchildren survive), upgrade to a Job object if orphaned processes show up
    let _ = child.start_kill();
    (child.wait().await.ok(), true)
}

/// Keeps the last `STDERR_TAIL` characters, reading until EOF so `claude` never blocks on a full pipe.
fn drain_stderr<R: AsyncRead + Unpin + Send + 'static>(
    stream: R,
    tail: Arc<Mutex<String>>,
    rt: &Handle,
) -> tokio::task::JoinHandle<()> {
    rt.spawn(async move {
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
