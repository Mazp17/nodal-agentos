mod activity;
mod db;
mod domain;
mod linear;
mod runs;
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
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(linear::LinearState::new())
        .setup(|app| {
            use tauri::Manager;
            // Sin base la app abre igual (para mostrar el error); los comandos que la usan fallan.
            match db::open(&app.path().app_data_dir()?.join(db::DB_FILE)) {
                Ok(db) => {
                    app.manage(db.clone());
                    work::init(app.handle(), db)?;
                }
                Err(e) => eprintln!("nodal.db: {e}"),
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
            work::commands::read_task_plan,
            work::commands::list_task_relations,
            work::commands::add_task_relation,
            work::commands::remove_task_relation,
            work::commands::cleanup_worktree,
            work::commands::list_executors,
            work::commands::list_task_runs,
            work::commands::list_queue,
            work::commands::launch_task,
            work::commands::hand_off,
            work::commands::review_now,
            work::commands::confirm_run,
            work::commands::cancel_run,
            work::commands::reorder_queue,
            work::commands::run_diff,
            work::commands::open_in_editor,
            work::commands::open_worktree,
            work::commands::get_settings,
            work::commands::set_settings,
            activity::repo_activity,
            activity::activity_summary,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
