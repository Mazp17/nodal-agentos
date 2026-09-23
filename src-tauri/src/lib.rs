use std::process::Command;

mod runs;

/// Versión del CLI de `claude`: prueba mínima de que el core puede invocarlo.
/// Ojo: una app abierta desde Finder no hereda el PATH del shell; en `tauri dev` sí.
/// `async` para que corra fuera del main thread: arrancar el CLI tarda y congelaría la UI.
#[tauri::command]
async fn claude_version() -> Result<String, String> {
    let out = Command::new("claude")
        .arg("--version")
        .output()
        .map_err(|e| format!("no se pudo ejecutar claude: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            claude_version,
            runs::launch_run,
            runs::list_runs,
            runs::get_run_detail
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
