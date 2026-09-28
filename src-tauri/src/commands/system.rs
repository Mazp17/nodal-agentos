//! "Import data from a previous version…": Tauri command. The import logic (copy, then
//! import inside a transaction) lives in `nodal_store::legacy`.

use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::db::{self, Db, DbError};

#[tauri::command]
pub async fn import_legacy_data(
    app: AppHandle,
    db: State<'_, Db>,
    folder: String,
) -> Result<nodal_store::legacy::LegacyImportReport, String> {
    let data_dir = crate::util::paths::data_dir(&app)?;
    let now = crate::util::now_ms();
    let src = PathBuf::from(folder.trim());
    let dd = data_dir.clone();
    // The copy doesn't take the database lock.
    let (backup_dir, skipped) = tauri::async_runtime::spawn_blocking(move || nodal_store::legacy::backup(&src, &dd, now))
        .await
        .map_err(|e| format!("Backup task failed: {e}"))??;
    let db = db.inner().clone();
    let mut report = db::with_db(&db, move |conn| {
        nodal_store::legacy::import_backup(conn, &backup_dir, &data_dir, now).map_err(DbError::Invalid)
    })
    .await?;
    report.skipped.splice(0..0, skipped);
    Ok(report)
}
