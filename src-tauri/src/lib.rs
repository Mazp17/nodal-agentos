mod activity;
mod db;
mod domain;
mod events;
mod linear;
pub mod mcp;
mod migrate;
mod providers;
mod runs;
mod secrets;
mod updates;
mod util;
mod work;

use std::time::Duration;

/// Versión del CLI de `claude`: prueba mínima de que el core puede invocarlo.
/// Usa el mismo resolutor que los runs, así funciona también abierta desde Finder.
#[tauri::command]
async fn claude_version() -> Result<String, String> {
    let mut cmd = runs::claude_bin::claude_command()?;
    cmd.arg("--version");
    let out = runs::claude_bin::output_with_timeout(cmd, Duration::from_secs(10), "claude --version").await?;
    if !out.status.success() {
        return Err(runs::claude_bin::error_text(&out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let secrets = secrets::Secrets::keychain();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .menu(updates::menu)
        .on_menu_event(updates::on_menu_event)
        .manage(linear::LinearState::new(secrets.clone()))
        .manage(secrets)
        .setup(|app| {
            use tauri::Manager;
            app.manage(events::Events::new(app.handle().clone()));
            if util::paths::DEV {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_title("Nodal Dev");
                }
            }
            // Único lugar que arranca los workers de fondo: el pump de la cola (`work::init`)
            // con el socket MCP (`mcp::server`) y el sync de proveedores (`providers::init`). Sin base la app abre igual (para
            // mostrar el error): no hay pump, el sync no hace nada y los comandos fallan.
            match db::open(&util::paths::data_dir(app.handle())?.join(db::DB_FILE)) {
                Ok(db) => {
                    app.manage(db.clone());
                    if let Err(e) = work::init(app.handle(), db) {
                        eprintln!("work: {e}");
                    } else if let Err(e) = mcp::server::start(app.state::<work::WorkState>().0.clone()) {
                        eprintln!("mcp: {e}");
                    }
                }
                Err(e) => eprintln!("nodal.db: {e}"),
            }
            if let Err(e) = providers::init(app.handle()) {
                eprintln!("providers: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            claude_version,
            runs::launch_run,
            runs::list_runs,
            runs::get_run_detail,
            runs::get_agent_transcript,
            runs::get_launch_blocker,
            runs::terminal::attach_run,
            runs::terminal::stop_run,
            runs::terminal::open_terminal_at,
            util::paths::resolve_git_root,
            util::git::git_version,
            runs::claude_trust::repo_trust,
            linear::linear_key_status,
            linear::linear_set_api_key,
            linear::linear_clear_api_key,
            linear::linear_viewer,
            linear::linear_teams,
            linear::linear_board,
            linear::linear_issue_detail,
            work::commands::list_projects,
            work::commands::create_project,
            work::commands::update_project,
            work::commands::delete_project,
            work::commands::list_repos,
            work::commands::add_repo,
            work::commands::update_repo,
            work::commands::delete_repo,
            work::commands::list_tasks,
            work::commands::get_task,
            work::commands::create_task,
            work::commands::update_task,
            work::commands::delete_task,
            work::commands::move_task,
            work::commands::reorder_tasks,
            work::commands::read_task_plan,
            work::commands::list_task_relations,
            work::commands::add_task_relation,
            work::commands::remove_task_relation,
            work::commands::cleanup_worktree,
            work::commands::worktree_status,
            work::commands::list_executors,
            work::commands::list_task_runs,
            work::commands::list_runs_light,
            work::commands::get_run,
            work::commands::latest_runs_by_task,
            work::commands::list_queue,
            work::commands::work_summary,
            work::commands::launch_task,
            work::commands::hand_off,
            work::commands::review_now,
            work::commands::confirm_run,
            work::commands::cancel_run,
            work::commands::reorder_queue,
            work::commands::run_diff,
            work::commands::get_run_transcript,
            work::commands::open_in_editor,
            work::commands::open_worktree,
            work::commands::get_settings,
            work::commands::set_settings,
            providers::commands::provider_status,
            providers::commands::provider_set_key,
            providers::commands::provider_clear_key,
            providers::commands::provider_scopes,
            providers::commands::list_source_links,
            providers::commands::create_source_link,
            providers::commands::update_source_link,
            providers::commands::delete_source_link,
            providers::commands::unlink_task,
            providers::commands::source_states,
            providers::commands::save_state_map,
            providers::commands::provider_list_importable,
            providers::commands::import_tasks,
            providers::commands::sync_now,
            providers::commands::source_rule_projects,
            providers::commands::preview_rule_import,
            providers::commands::import_rule,
            providers::commands::resolve_moved_task,
            activity::repo_activity,
            activity::activity_summary,
            activity::project_activity,
            migrate::import_legacy_data,
            updates::updates_enabled,
            updates::restart_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
