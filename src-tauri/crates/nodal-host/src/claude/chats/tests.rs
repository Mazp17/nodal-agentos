//! P05 (delta batching) and P08 (stdout line cap), driven against a real `ChatProcesses` and a
//! fake `claude` shell script — no database, no `ChatHooksImpl` (that's covered against a real
//! one in `nodal-app::chats::hooks::tests`, which this must keep passing unchanged).
#![allow(clippy::disallowed_methods)] // spins up a fake `claude` script on disk

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nodal_domain::error::HostError;
use nodal_domain::model::chat::{ChatEnvelope, ChatEvent, ChatSpec, HostEvent, RunState, StreamEvent};
use nodal_domain::ports::{ChatHooks, ChatSink};

use super::ChatProcesses;
use crate::testutil::block_on;

/// A `Handle` to the same static runtime `block_on` drives, for `ChatProcesses::with_program`
/// (which spawns background tasks that need to keep being polled while a test's own `block_on`
/// future is running).
fn rt() -> tokio::runtime::Handle {
    block_on(async { tokio::runtime::Handle::current() })
}

struct NoopHooks;

impl ChatHooks for NoopHooks {
    fn session_started(&self, _chat_id: &str, _session_id: String, _project_id: String) {}
    fn turn_ended(&self, _chat_id: &str, _cwd: PathBuf, _project_id: String) {}
}

struct RecordingSink(Arc<Mutex<Vec<ChatEnvelope>>>);

impl ChatSink for RecordingSink {
    fn emit(&self, ev: ChatEnvelope) {
        self.0.lock().unwrap().push(ev);
    }
}

struct Fixture {
    chats: Arc<ChatProcesses>,
    events: Arc<Mutex<Vec<ChatEnvelope>>>,
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str, script: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("nodal-chats-batch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("claude");
        std::fs::write(&program, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink: Arc<dyn ChatSink> = Arc::new(RecordingSink(events.clone()));
        let hooks: Arc<dyn ChatHooks> = Arc::new(NoopHooks);
        let chats = ChatProcesses::with_program(sink, hooks, rt(), Some(program));
        Fixture { chats, events, dir }
    }

    fn spec(&self) -> ChatSpec {
        ChatSpec { cwd: self.dir.clone(), args: vec![] }
    }

    fn events(&self) -> Vec<ChatEvent> {
        self.events.lock().unwrap().iter().map(|e| e.event.clone()).collect()
    }

    async fn wait_for(&self, n: usize, what: &str, pred: impl Fn(&ChatEvent) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.events().iter().filter(|e| pred(e)).count() < n {
            assert!(Instant::now() < deadline, "timed out waiting for {what}: {:#?}", self.events());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.chats.clear();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn send(chats: &ChatProcesses, spec: ChatSpec) -> Result<(), HostError> {
    use nodal_domain::ports::ChatRuntime;
    chats.send("c1", "p1", None, spec, "hi")
}

/// A burst of `text_delta` lines (no delay between them, like `claude` printing tokens faster
/// than 60/s) followed by the matching full text and a `result`.
const BURST_DELTAS: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"00000000-0000-4000-8000-000000000001","cwd":"/tmp"}'
i=0
while [ $i -lt 40 ]; do
  echo "{\"type\":\"stream_event\",\"event\":{\"delta\":{\"type\":\"text_delta\",\"text\":\" $i\"}},\"parent_tool_use_id\":null}"
  i=$((i + 1))
done
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]},"parent_tool_use_id":null}'
echo '{"type":"result","subtype":"success","is_error":false}'
"#;

#[test]
fn bursts_of_deltas_are_coalesced_but_lose_no_text() {
    let f = Fixture::new("burst", BURST_DELTAS);
    block_on(async {
        send(&f.chats, f.spec()).unwrap();
        f.wait_for(1, "turn end", |e| matches!(e, ChatEvent::Stream(StreamEvent::TurnEnd { .. }))).await;

        let expected: String = (0..40).map(|i| format!(" {i}")).collect();
        let deltas: Vec<String> = f
            .events()
            .into_iter()
            .filter_map(|e| match e {
                ChatEvent::Stream(StreamEvent::TextDelta { text }) => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(deltas.concat(), expected, "no text lost across flushes");
        assert!(deltas.len() < 40, "the burst must be coalesced into fewer emits: {}", deltas.len());
        assert!(!deltas.is_empty());
    });
}

/// After a `TextDelta` and a `ThinkingDelta` interleave, and after the process exits mid-turn,
/// nothing buffered is lost: a kind change flushes the old buffer, and `on_exit` flushes too.
const INTERLEAVED_THEN_CRASH: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"00000000-0000-4000-8000-000000000002","cwd":"/tmp"}'
echo '{"type":"stream_event","event":{"delta":{"type":"thinking_delta","thinking":"hmm"}},"parent_tool_use_id":null}'
echo '{"type":"stream_event","event":{"delta":{"type":"text_delta","text":"tail"}},"parent_tool_use_id":null}'
exit 3
"#;

#[test]
fn a_kind_change_and_a_crash_both_flush_the_buffer() {
    let f = Fixture::new("interleave", INTERLEAVED_THEN_CRASH);
    block_on(async {
        send(&f.chats, f.spec()).unwrap();
        f.wait_for(1, "error", |e| matches!(e, ChatEvent::Host(HostEvent::Error { .. }))).await;
        let ev = f.events();
        assert!(ev.contains(&ChatEvent::Stream(StreamEvent::ThinkingDelta { text: "hmm".into() })), "{ev:#?}");
        assert!(ev.contains(&ChatEvent::Stream(StreamEvent::TextDelta { text: "tail".into() })), "{ev:#?}");
    });
}

/// More lines than `MAX_STDOUT_LINES` (the `cfg(test)` value, 100): the reader stops parsing
/// past the cap, reports why and the process is stopped — not left running forever.
const FLOODS_OUTPUT: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"00000000-0000-4000-8000-000000000003","cwd":"/tmp"}'
i=0
while [ $i -lt 150 ]; do
  echo "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"line $i\"}]},\"parent_tool_use_id\":null}"
  i=$((i + 1))
done
"#;

#[test]
fn a_flood_of_lines_is_capped_reported_and_stops_the_process() {
    let f = Fixture::new("flood", FLOODS_OUTPUT);
    block_on(async {
        send(&f.chats, f.spec()).unwrap();
        f.wait_for(1, "overflow error", |e| matches!(e, ChatEvent::Host(HostEvent::Error { .. }))).await;
        f.wait_for(1, "stopped", |e| *e == ChatEvent::Host(HostEvent::State { state: RunState::Stopped })).await;

        let items = f
            .events()
            .into_iter()
            .filter(|e| matches!(e, ChatEvent::Stream(StreamEvent::Item { .. })))
            .count();
        // The cap counts every line, including `init`: 99 `assistant` lines fit under it.
        assert_eq!(items, 99, "{:#?}", f.events());
        let message = f.events().into_iter().find_map(|e| match e {
            ChatEvent::Host(HostEvent::Error { message }) => Some(message),
            _ => None,
        });
        assert!(message.unwrap().contains("exceeded"));
    });
}

/// A `ChatSink` that panics for one poisoned chat id and records every other chat's events
/// normally — for P18: a panic in a chat's reader task (the sink is called from `on_line`,
/// same as production) must not affect any other chat sharing the same `ChatProcesses`.
struct PanicOnChatSink {
    poison_chat_id: &'static str,
    events: Arc<Mutex<Vec<ChatEnvelope>>>,
}

impl ChatSink for PanicOnChatSink {
    fn emit(&self, ev: ChatEnvelope) {
        // Only `Stream` events, which `on_line`/`dispatch` emit from inside the reader task:
        // `send()` itself synchronously emits a `Host(State::Busy)` on the caller's own task
        // before the process even starts, which isn't the panic this test means to isolate.
        if ev.chat_id == self.poison_chat_id && matches!(ev.event, ChatEvent::Stream(_)) {
            panic!("intentional test panic: PanicOnChatSink for chat {}", self.poison_chat_id);
        }
        self.events.lock().unwrap().push(ev);
    }
}

/// P18: a panic inside one chat's reader task — with `panic = "unwind"`, tokio catches it
/// (it's swallowed by the `let _ = tokio::time::timeout(..., reader.await).await` the
/// supervising task already has) — must not affect a second, healthy chat running on the
/// same `ChatProcesses`.
#[test]
fn a_panic_in_one_chats_reader_does_not_affect_another_chat() {
    let dir = std::env::temp_dir().join(format!("nodal-chats-panic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("claude");
    std::fs::write(&program, BURST_DELTAS).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink: Arc<dyn ChatSink> = Arc::new(PanicOnChatSink { poison_chat_id: "boom", events: events.clone() });
    let hooks: Arc<dyn ChatHooks> = Arc::new(NoopHooks);
    let chats = ChatProcesses::with_program(sink, hooks, rt(), Some(program));

    block_on(async {
        use nodal_domain::ports::ChatRuntime;
        let spec = ChatSpec { cwd: dir.clone(), args: vec![] };
        // Its reader panics on the very first line (`system init`).
        chats.send("boom", "p1", None, spec.clone(), "hi").unwrap();
        // A second, unrelated chat on the same `ChatProcesses` must run to completion.
        chats.send("ok", "p1", None, spec, "hi").unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let done = events.lock().unwrap().iter().any(|e| {
                e.chat_id == "ok" && matches!(e.event, ChatEvent::Stream(StreamEvent::TurnEnd { .. }))
            });
            if done {
                break;
            }
            assert!(Instant::now() < deadline, "the healthy chat never finished its turn: {:#?}", events.lock().unwrap());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // No *stream* event recorded for the poisoned chat: every one of those panicked in the
        // sink (only its synchronous, pre-process `Host(State::Busy)` from `send()` got through).
        assert!(events.lock().unwrap().iter().all(|e| e.chat_id != "boom" || !matches!(e.event, ChatEvent::Stream(_))));
    });
    chats.clear();
    let _ = std::fs::remove_dir_all(&dir);
}
