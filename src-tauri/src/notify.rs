//! Chat notifications that open their chat when clicked.
//!
//! `tauri-plugin-notification` ~2.4 can't do this on desktop: its `show()` hands the notification
//! to `notify-rust` on a spawned task and drops the result, and `onAction` is mobile only. So chat
//! notifications are sent from here with `mac-notification-sys` (what `notify-rust` uses on macOS),
//! waiting for the click on a thread of their own; a click focuses the window and emits
//! `OPEN_CHAT_EVENT`. Elsewhere they still go through the plugin, without the click.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const OPEN_CHAT_EVENT: &str = "nodal://open-chat";

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct OpenChat {
    chat_id: String,
}

/// Shows the main window in front and asks the page to open `chat_id`.
fn open_chat(app: &AppHandle, chat_id: String) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    if let Err(e) = app.emit(OPEN_CHAT_EVENT, OpenChat { chat_id }) {
        tracing::warn!("open chat: {e}");
    }
}

/// Posts a system notification for `chat_id`; clicking it opens that chat (macOS).
#[tauri::command]
pub fn notify_chat(app: AppHandle, chat_id: String, title: String, body: String) {
    #[cfg(target_os = "macos")]
    {
        // Same sender as the plugin: in dev an unbundled binary can't post as itself.
        let _ = mac_notification_sys::set_application(if tauri::is_dev() {
            "com.apple.Terminal"
        } else {
            &app.config().identifier
        });
        // shortcut: one blocked thread per notification until it's clicked or cleared from
        // Notification Center; fine for a few a day, pool them if chats ever notify in bulk.
        std::thread::spawn(move || {
            let sent = mac_notification_sys::Notification::new()
                .title(&title)
                .message(&body)
                .wait_for_click(true)
                .send();
            match sent {
                Ok(mac_notification_sys::NotificationResponse::Click) => open_chat(&app, chat_id),
                Ok(_) => {}
                Err(e) => tracing::warn!("notify chat: {e}"),
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri_plugin_notification::NotificationExt;
        let _ = chat_id;
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            tracing::warn!("notify chat: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_chat_payload_is_camel_case() {
        let json = serde_json::to_value(OpenChat { chat_id: "c1".into() }).unwrap();
        assert_eq!(json, serde_json::json!({ "chatId": "c1" }));
    }
}
