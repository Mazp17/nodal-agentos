mod config;
mod issue_runs;
mod linear;
mod runs;

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
        .manage(linear::LinearState::new())
        .setup(|app| {
            issue_runs::init(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            claude_version,
            runs::launch_run,
            runs::list_runs,
            runs::get_run_detail,
            linear::linear_key_status,
            linear::linear_set_api_key,
            linear::linear_clear_api_key,
            linear::linear_viewer,
            linear::linear_teams,
            linear::linear_board,
            linear::linear_issue_detail,
            config::get_config,
            config::save_config,
            config::resolve_repo,
            issue_runs::list_workflows,
            issue_runs::launch_issue_run,
            issue_runs::list_issue_runs,
            issue_runs::cancel_queued,
            issue_runs::attach_run,
            issue_runs::stop_run,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
