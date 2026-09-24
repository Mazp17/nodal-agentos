mod activity;
mod config;
mod db;
mod domain;
mod issue_runs;
mod linear;
mod providers;
mod runs;
mod tasks;

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
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(linear::LinearState::new())
        .setup(|app| {
            use tauri::Manager;
            // Todavía no la usa nadie (llega en F1): si falla, se avisa y la app sigue.
            match db::open(&app.path().app_data_dir()?.join(db::DB_FILE)) {
                Ok(db) => {
                    app.manage(db);
                }
                Err(e) => eprintln!("nodal.db: {e}"),
            }
            // F1-C: sync de proveedores (worker cada 60 s). Sin base, el worker no hace nada.
            providers::init(app.handle())?;
            issue_runs::init(app.handle())?;
            tasks::init(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            claude_version,
            runs::launch_run,
            runs::list_runs,
            runs::get_run_detail,
            runs::get_agent_transcript,
            runs::get_launch_blocker,
            linear::linear_key_status,
            linear::linear_set_api_key,
            linear::linear_clear_api_key,
            linear::linear_viewer,
            linear::linear_teams,
            linear::linear_board,
            linear::linear_issue_detail,
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
            config::get_config,
            config::save_config,
            config::resolve_repo,
            config::resolve_repo_config,
            config::resolve_git_root,
            issue_runs::list_workflows,
            issue_runs::launch_issue_run,
            issue_runs::list_issue_runs,
            issue_runs::cancel_queued,
            issue_runs::attach_run,
            issue_runs::stop_run,
            issue_runs::open_terminal_at,
            tasks::create_task,
            tasks::update_task,
            tasks::delete_task,
            tasks::list_tasks,
            tasks::set_task_done,
            tasks::read_task_plan,
            tasks::launch_task_run,
            tasks::list_task_runs,
            tasks::cancel_task_run,
            activity::repo_activity,
            activity::activity_summary,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
