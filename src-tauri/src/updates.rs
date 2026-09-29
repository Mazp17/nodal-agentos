//! In-app updates. The check, download and install run in the UI through
//! `@tauri-apps/plugin-updater`; here live the "Check for Updates…" menu item, the
//! `nodal://check-updates` event it sends and the switch that keeps debug builds out of it.

use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::{AppHandle, Emitter, Runtime};

use nodal_domain::DEV;

pub const MENU_ID: &str = "check-for-updates";
pub const CHECK_EVENT: &str = "nodal://check-updates";

/// Debug builds ("Nodal Dev") never check: they'd offer to replace themselves with a release.
pub const fn enabled(dev: bool) -> bool {
    !dev
}

/// Whether this build checks for updates. The UI asks before its launch check.
#[tauri::command]
pub fn updates_enabled() -> bool {
    enabled(DEV)
}

/// Relaunch after an update was installed.
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}

/// Default menu plus "Check for Updates…" right after "About Nodal" (release builds only).
pub fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    if !enabled(DEV) {
        return Ok(menu);
    }
    let item = MenuItem::with_id(app, MENU_ID, "Check for Updates…", true, None::<&str>)?;
    if let Some(app_menu) = menu.items()?.first().and_then(|m| m.as_submenu()) {
        app_menu.insert(&item, 1)?;
    }
    Ok(menu)
}

pub fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() == MENU_ID {
        if let Err(e) = app.emit(CHECK_EVENT, ()) {
            eprintln!("updates: {e}");
        }
    }
}

#[cfg(test)]
mod tests;
