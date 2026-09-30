use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tauri::ipc::InvokeResponseBody;
use tauri::Listener;

use nodal_domain::model::chat::{ChatEvent, StreamEvent};

use super::*;

const CHATS: [&str; 2] = ["chat-a", "chat-b"];
const EVENTS_PER_CHAT: usize = 500;

#[derive(Default)]
struct Tally {
    /// Deliveries a chat view received (whether it was for it or not).
    received: AtomicUsize,
    /// Of those, deliveries for another chat that the view had to parse and drop.
    foreign: AtomicUsize,
    bytes: AtomicUsize,
}

fn delta(chat: &str, i: usize) -> ChatEnvelope {
    ChatEnvelope {
        chat_id: chat.into(),
        event: ChatEvent::Stream(StreamEvent::TextDelta { text: format!("token {i} ") }),
    }
}

fn drive<R: Runtime>(sink: &TauriChatSink<R>) {
    for i in 0..EVENTS_PER_CHAT {
        for chat in CHATS {
            sink.emit(delta(chat, i));
        }
    }
}

fn report(label: &str, tallies: &[Arc<Tally>]) -> (usize, usize) {
    let received: usize = tallies.iter().map(|t| t.received.load(Ordering::SeqCst)).sum();
    let foreign: usize = tallies.iter().map(|t| t.foreign.load(Ordering::SeqCst)).sum();
    let bytes: usize = tallies.iter().map(|t| t.bytes.load(Ordering::SeqCst)).sum();
    eprintln!("P05 {label}: 2 chats x {EVENTS_PER_CHAT} emits -> {received} deliveries ({foreign} for another chat), {bytes} B");
    (received, foreign)
}

/// P05 before/after, same 2-chat workload. Before (no channel attached: the `nodal://chat`
/// broadcast, which each chat view filtered by `chatId`): every view gets every chat's
/// events. After (a channel per chat): each view gets only its own.
#[test]
fn measure_broadcast_vs_channel_deliveries() {
    let app = tauri::test::mock_app();
    let handle = app.handle().clone();

    let broadcast: Vec<Arc<Tally>> = CHATS
        .iter()
        .map(|&mine| {
            let t = Arc::new(Tally::default());
            let tt = t.clone();
            handle.listen(CHAT_EVENT, move |ev| {
                tt.received.fetch_add(1, Ordering::SeqCst);
                tt.bytes.fetch_add(ev.payload().len(), Ordering::SeqCst);
                if !ev.payload().contains(&format!("\"chatId\":\"{mine}\"")) {
                    tt.foreign.fetch_add(1, Ordering::SeqCst);
                }
            });
            t
        })
        .collect();
    drive(&TauriChatSink::new(handle.clone()));
    let (before, before_foreign) = report("broadcast", &broadcast);

    let sink = TauriChatSink::new(handle.clone());
    let channel: Vec<Arc<Tally>> = CHATS
        .iter()
        .map(|&mine| {
            let t = Arc::new(Tally::default());
            let tt = t.clone();
            sink.attach(
                mine.into(),
                Channel::new(move |body| {
                    let InvokeResponseBody::Json(json) = body else { unreachable!() };
                    tt.received.fetch_add(1, Ordering::SeqCst);
                    tt.bytes.fetch_add(json.len(), Ordering::SeqCst);
                    if !json.contains(&format!("\"chatId\":\"{mine}\"")) {
                        tt.foreign.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(())
                }),
            );
            t
        })
        .collect();
    let broadcast_after_attach = || broadcast.iter().map(|t| t.received.load(Ordering::SeqCst)).sum::<usize>();
    let untouched = broadcast_after_attach();
    drive(&sink);
    let (after, after_foreign) = report("channel", &channel);

    assert_eq!(before, CHATS.len() * CHATS.len() * EVENTS_PER_CHAT);
    assert_eq!(before_foreign, (CHATS.len() - 1) * CHATS.len() * EVENTS_PER_CHAT);
    assert_eq!(after, CHATS.len() * EVENTS_PER_CHAT);
    assert_eq!(after_foreign, 0);
    assert_eq!(broadcast_after_attach(), untouched, "attached chats must not also broadcast");
}
