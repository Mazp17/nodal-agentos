//! In-app updates. The check, download and install run in the UI through
//! `@tauri-apps/plugin-updater`; here live the "Check for Updates…" menu item, the
//! `nodal://check-updates` event it sends and the switch that keeps debug builds out of it.

use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::{AppHandle, Emitter, Runtime};

use crate::util::paths::DEV;

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
mod tests {
    use super::*;

    #[test]
    fn debug_builds_do_not_check() {
        assert!(!enabled(true));
        assert!(enabled(false));
        assert_eq!(updates_enabled(), !cfg!(debug_assertions));
    }

    #[test]
    fn config_ships_signed_updater_artifacts() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["bundle"]["createUpdaterArtifacts"], true);
        let updater = &conf["plugins"]["updater"];
        assert!(!updater["pubkey"].as_str().unwrap_or_default().is_empty());
        let endpoints = updater["endpoints"].as_array().unwrap();
        assert!(!endpoints.is_empty());
        for e in endpoints {
            let url = e.as_str().unwrap();
            assert!(url.starts_with("https://") && url.ends_with("/latest.json"), "{url}");
        }
    }
}
