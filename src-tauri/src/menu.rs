//! The app menu: Tauri's default, with ⌘W closing the current tab instead of the window
//! (the window moves to ⇧⌘W) and the updates item from `updates`.

use tauri::menu::{Menu, MenuEvent, MenuItem, MenuItemKind};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::updates;

const CLOSE_TAB_ID: &str = "close-tab";
const CLOSE_WINDOW_ID: &str = "close-window";
pub const CLOSE_TAB_EVENT: &str = "nodal://close-tab";

pub fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = updates::menu(app)?;
    for top in menu.items()? {
        let Some(sub) = top.as_submenu() else { continue };
        let items = sub.items()?;
        // The default "Close Window" owns ⌘W natively, before the page ever sees the key.
        let Some(pos) = items
            .iter()
            .position(|i| matches!(i, MenuItemKind::Predefined(p) if p.text().is_ok_and(|t| t == "Close Window")))
        else {
            continue;
        };
        sub.remove_at(pos)?;
        let close_tab = MenuItem::with_id(app, CLOSE_TAB_ID, "Close Tab", true, Some("CmdOrCtrl+W"))?;
        let close_window = MenuItem::with_id(app, CLOSE_WINDOW_ID, "Close Window", true, Some("CmdOrCtrl+Shift+W"))?;
        sub.insert(&close_tab, pos)?;
        sub.insert(&close_window, pos + 1)?;
    }
    Ok(menu)
}

pub fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    match event.id().as_ref() {
        CLOSE_TAB_ID => {
            if let Err(e) = app.emit(CLOSE_TAB_EVENT, ()) {
                eprintln!("menu: {e}");
            }
        }
        CLOSE_WINDOW_ID => {
            // One window app: close the focused one, else any.
            let windows = app.webview_windows();
            let w = windows.values().find(|w| w.is_focused().unwrap_or(false)).or_else(|| windows.values().next());
            if let Some(w) = w {
                let _ = w.close();
            }
        }
        _ => updates::on_menu_event(app, event),
    }
}
